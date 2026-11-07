# xdg-desktop-portal-tui

> The Open and Save dialog for [file-browser-tui](https://github.com/adelmonte/file-browser-tui),
> a Dolphin-like file manager for kitty. Install both to use it as your system's file dialog.

An [xdg-desktop-portal](https://flatpak.github.io/xdg-desktop-portal/) file chooser.
When Firefox, Chromium, a Flatpak app or any other program asks the portal for a file,
it opens file-browser-tui in a kitty window to pick with, instead of a GTK or KDE
dialog. What you pick goes back to the program.

- **Open**: one file, several (Ctrl/Shift-click, or drag a box), or a folder.
- **Save**: a name field at the bottom, filled with the program's suggestion and the
  cursor before the extension. Saving over an existing file asks first.
- **Save several files**: pick the folder they go in.
- **File types**: the program's filters (PDF, Images, ...) in a menu at the bottom;
  choosing one while saving changes the name's extension.
- Opens in the folder the program asks for, else the one you last picked in.
- Enter or a double click picks, Esc or closing the window cancels, Tab moves between
  the name field and the files.

## Requirements

- [file-browser-tui](https://github.com/adelmonte/file-browser-tui) 1.1.0 or later, and
  [kitty](https://sw.kovidgoyal.net/kitty/)
- xdg-desktop-portal
- Another portal backend (xdg-desktop-portal-gtk, -kde, -gnome, ...) for everything that
  isn't a file dialog: this one only does file dialogs

## Install

Arch Linux: [`xdg-desktop-portal-tui`](https://aur.archlinux.org/packages/xdg-desktop-portal-tui)
from the AUR.

From source:

```sh
cargo build --release
sudo install -Dm755 target/release/xdg-desktop-portal-tui /usr/lib/xdg-desktop-portal-tui
sudo install -Dm644 data/tui.portal /usr/share/xdg-desktop-portal/portals/tui.portal
sudo install -Dm644 data/org.freedesktop.impl.portal.desktop.tui.service \
    /usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.tui.service
```

## Setup

Tell the portal to use it in `~/.config/xdg-desktop-portal/portals.conf`. Either for file
dialogs only:

```ini
[preferred]
default=gtk
org.freedesktop.impl.portal.FileChooser=tui
```

or first wherever it can, with another backend for the rest:

```ini
[preferred]
default=tui;gtk
```

Then restart the portal so it reads the file: `pkill -f /usr/lib/xdg-desktop-portal`
(D-Bus starts it again when a program next asks), or log out and in.

Programs that don't use the portal by default:

- **Firefox**: set `widget.use-xdg-desktop-portal.file-picker` to `1` in `about:config`.
- **GTK apps**: set the environment variable `GTK_USE_PORTAL=1`.

### Window rules

The dialog's window has the app id `xdg-desktop-portal-tui`, so a window manager can
float it. In niri:

```kdl
window-rule {
    match app-id="xdg-desktop-portal-tui"
    open-floating true
    default-window-height { proportion 0.7; }
    default-column-width { proportion 0.8; }
}
```

In Sway: `for_window [app_id="xdg-desktop-portal-tui"] floating enable`.

## Options

`--browser PATH` picks with a file-browser-tui other than the one on `PATH` (a
development build, say). Set it on the `Exec=` line of a copy of the D-Bus service
file in `~/.local/share/dbus-1/services/`, which takes the place of the system one.

## How it works

The portal calls `OpenFile`, `SaveFile` or `SaveFiles`. The backend writes the request
(title, mode, filters, suggested name and folder) to a TOML file in
`$XDG_RUNTIME_DIR/xdg-desktop-portal-tui/` and runs
`file-browser-tui --kitty --pick REQUEST`. The browser writes what was picked to the
answer file the request names, and the backend sends it back as `file://` URIs. When
the program gives up on the dialog, the portal's `Close` ends the window.

## License

GPL-3.0-only
