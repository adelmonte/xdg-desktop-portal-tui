//! An xdg-desktop-portal FileChooser backend: each open or save request
//! opens a file-browser-tui window (in kitty) to pick with, and hands what
//! was picked back to the program that asked.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use zbus::object_server::ObjectServer;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{connection, interface};

const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.tui";

// The picking request and answer, as file-browser-tui's `--pick` reads them

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum Mode {
    #[default]
    Open,
    Save,
    SaveFiles,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Filter {
    name: String,
    globs: Vec<String>,
    mimes: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
struct PickRequest {
    mode: Mode,
    title: String,
    accept_label: String,
    multiple: bool,
    directory: bool,
    filters: Vec<Filter>,
    current_filter: Option<usize>,
    current_name: String,
    current_folder: Option<PathBuf>,
    current_file: Option<PathBuf>,
    files: Vec<String>,
    answer: PathBuf,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Answer {
    paths: Vec<PathBuf>,
    filter: Option<usize>,
}

type Options = HashMap<String, OwnedValue>;
/// A portal filter: its name and (0: glob, 1: mime type, pattern) pairs.
type PortalFilter = (String, Vec<(u32, String)>);

fn opt<'a, T>(options: &'a Options, key: &str) -> Option<T>
where
    T: TryFrom<Value<'a>>,
{
    let v = options.get(key)?;
    T::try_from(Value::from(v.try_clone().ok()?)).ok()
}

/// A path the portal sends as a null-terminated byte string.
fn path_opt(options: &Options, key: &str) -> Option<PathBuf> {
    let bytes: Vec<u8> = opt(options, key)?;
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(&bytes);
    (!bytes.is_empty()).then(|| PathBuf::from(OsStr::from_bytes(bytes)))
}

fn to_filter((name, patterns): &PortalFilter) -> Filter {
    let mut f = Filter { name: name.clone(), ..Default::default() };
    for (kind, pattern) in patterns {
        match kind {
            1 => f.mimes.push(pattern.clone()),
            _ => f.globs.push(pattern.clone()),
        }
    }
    f
}

fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

struct Picker {
    browser: PathBuf,
    runtime: PathBuf,
    seq: AtomicU64,
}

/// The impl side of a portal Request: Close stops the picking window.
struct Request {
    pid: Arc<Mutex<Option<u32>>>,
}

#[interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    async fn close(&self) {
        if let Some(pid) = *self.pid.lock().unwrap() {
            let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), nix::sys::signal::Signal::SIGTERM);
        }
    }
}

impl Picker {
    async fn pick(&self, server: &ObjectServer, handle: OwnedObjectPath, mut req: PickRequest, portal_filters: &[PortalFilter]) -> (u32, Options) {
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        let base = self.runtime.join(format!("{}-{n}", std::process::id()));
        let req_path = base.with_extension("request.toml");
        req.answer = base.with_extension("answer.toml");
        let _ = std::fs::remove_file(&req.answer);
        let text = match toml::to_string(&req) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("xdg-desktop-portal-tui: {e}");
                return (2, Options::new());
            }
        };
        if let Err(e) = std::fs::write(&req_path, text) {
            eprintln!("xdg-desktop-portal-tui: writing {}: {e}", req_path.display());
            return (2, Options::new());
        }

        let pid = Arc::new(Mutex::new(None));
        let _ = server.at(&handle, Request { pid: pid.clone() }).await;
        let child = tokio::process::Command::new(&self.browser)
            .arg("--kitty").arg("--pick").arg(&req_path)
            .stdin(std::process::Stdio::null())
            .spawn();
        let status = match child {
            Ok(mut child) => {
                *pid.lock().unwrap() = child.id();
                child.wait().await.ok()
            }
            Err(e) => {
                eprintln!("xdg-desktop-portal-tui: starting {}: {e}", self.browser.display());
                None
            }
        };
        let _ = server.remove::<Request, _>(&handle).await;
        let _ = std::fs::remove_file(&req_path);
        let answer = std::fs::read_to_string(&req.answer).ok()
            .and_then(|s| toml::from_str::<Answer>(&s).ok());
        let _ = std::fs::remove_file(&req.answer);

        let Some(answer) = answer.filter(|a| !a.paths.is_empty()) else {
            // Closed without picking: cancelled; failed to start: an error
            return (if status.is_some() { 1 } else { 2 }, Options::new());
        };
        let mut results = Options::new();
        let uris: Vec<String> = answer.paths.iter().map(|p| file_uri(p)).collect();
        if let Ok(v) = Value::from(uris).try_into() {
            results.insert("uris".into(), v);
        }
        if let Some(f) = answer.filter.and_then(|i| portal_filters.get(i))
            && let Ok(v) = Value::from(zbus::zvariant::Structure::from(f.clone())).try_into()
        {
            results.insert("current_filter".into(), v);
        }
        (0, results)
    }

    fn base_request(title: String, options: &Options) -> (PickRequest, Vec<PortalFilter>) {
        let filters: Vec<PortalFilter> = opt(options, "filters").unwrap_or_default();
        let current: Option<PortalFilter> = opt(options, "current_filter");
        let mut current_filter = current.as_ref().and_then(|c| filters.iter().position(|f| f == c));
        let mut filters = filters;
        // A current filter that isn't in the list joins it
        if let (Some(c), None) = (current, current_filter) {
            filters.insert(0, c);
            current_filter = Some(0);
        }
        let req = PickRequest {
            title,
            accept_label: opt(options, "accept_label").unwrap_or_default(),
            filters: filters.iter().map(to_filter).collect(),
            current_filter,
            current_folder: path_opt(options, "current_folder"),
            ..Default::default()
        };
        (req, filters)
    }
}

#[interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl Picker {
    #[zbus(out_args("response", "results"))]
    async fn open_file(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        handle: OwnedObjectPath,
        _app_id: String,
        _parent_window: String,
        title: String,
        options: Options,
    ) -> (u32, Options) {
        let (mut req, filters) = Self::base_request(title, &options);
        req.mode = Mode::Open;
        req.multiple = opt(&options, "multiple").unwrap_or(false);
        req.directory = opt(&options, "directory").unwrap_or(false);
        self.pick(server, handle, req, &filters).await
    }

    #[zbus(out_args("response", "results"))]
    async fn save_file(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        handle: OwnedObjectPath,
        _app_id: String,
        _parent_window: String,
        title: String,
        options: Options,
    ) -> (u32, Options) {
        let (mut req, filters) = Self::base_request(title, &options);
        req.mode = Mode::Save;
        req.current_file = path_opt(&options, "current_file");
        req.current_name = opt::<String>(&options, "current_name")
            .filter(|n| !n.is_empty())
            .or_else(|| req.current_file.as_ref().and_then(|f| f.file_name()).map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_default();
        self.pick(server, handle, req, &filters).await
    }

    #[zbus(out_args("response", "results"))]
    async fn save_files(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        handle: OwnedObjectPath,
        _app_id: String,
        _parent_window: String,
        title: String,
        options: Options,
    ) -> (u32, Options) {
        let (mut req, filters) = Self::base_request(title, &options);
        req.mode = Mode::SaveFiles;
        let files: Vec<Vec<u8>> = opt(&options, "files").unwrap_or_default();
        req.files = files.iter()
            .map(|b| String::from_utf8_lossy(b.strip_suffix(&[0]).unwrap_or(b)).to_string())
            .filter(|n| !n.is_empty() && !n.contains('/'))
            .collect();
        self.pick(server, handle, req, &filters).await
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }
}

const HELP: &str = "\
xdg-desktop-portal-tui: an xdg-desktop-portal file chooser using file-browser-tui

Usage: xdg-desktop-portal-tui [--browser PATH]
  --browser PATH   the file-browser-tui to pick with (default: from PATH)

Started by D-Bus when a program asks the portal for a file; set
org.freedesktop.impl.portal.FileChooser=tui in
~/.config/xdg-desktop-portal/portals.conf to use it.";

#[tokio::main(flavor = "current_thread")]
async fn main() -> zbus::Result<()> {
    let mut browser = PathBuf::from("file-browser-tui");
    let mut args = std::env::args_os().skip(1);
    while let Some(a) = args.next() {
        match a.to_str() {
            Some("--browser") => match args.next() {
                Some(p) => browser = PathBuf::from(p),
                None => {
                    eprintln!("xdg-desktop-portal-tui: --browser needs a path");
                    std::process::exit(2);
                }
            },
            Some("--help" | "-h") => {
                println!("{HELP}");
                return Ok(());
            }
            Some("--version" | "-V") => {
                println!("xdg-desktop-portal-tui {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => {
                eprintln!("xdg-desktop-portal-tui: unknown argument {} (see --help)", a.to_string_lossy());
                std::process::exit(2);
            }
        }
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("xdg-desktop-portal-tui");
    std::fs::create_dir_all(&runtime)?;

    let picker = Picker { browser, runtime, seq: AtomicU64::new(0) };
    let _conn = connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at("/org/freedesktop/portal/desktop", picker)?
        .build()
        .await?;
    tokio::signal::ctrl_c().await.ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_are_escaped() {
        assert_eq!(file_uri(Path::new("/home/a b/ü#.txt")), "file:///home/a%20b/%C3%BC%23.txt");
    }

    #[test]
    fn options_parse() {
        let mut o = Options::new();
        let filters: Vec<PortalFilter> = vec![("Images".into(), vec![(0, "*.png".into()), (1, "image/jpeg".into())])];
        o.insert("filters".into(), Value::from(filters.clone()).try_into().unwrap());
        o.insert("current_folder".into(), Value::from(b"/tmp\0".to_vec()).try_into().unwrap());
        o.insert("multiple".into(), Value::from(true).try_into().unwrap());
        let (req, back) = Picker::base_request("Open".into(), &o);
        assert_eq!(back, filters);
        assert_eq!(req.filters[0].globs, vec!["*.png"]);
        assert_eq!(req.filters[0].mimes, vec!["image/jpeg"]);
        assert_eq!(req.current_folder.as_deref(), Some(Path::new("/tmp")));
        assert_eq!(opt::<bool>(&o, "multiple"), Some(true));
    }
}
