# xdg-desktop-portal-tui

An xdg-desktop-portal file chooser that uses
[file-browser-tui](https://github.com/adelmonte/file-browser-tui) in kitty as the
Open and Save dialog for Firefox and any other program that asks the portal.

Each request opens a kitty window (app id `xdg-desktop-portal-tui`) running
`file-browser-tui --kitty --pick`; what you pick goes back to the program. Open
(one or more files, or folders), Save (a name field) and Save Files (a folder) are
supported, with the program's file type filters.

## Install

```sh
cargo build --release
install -Dm755 target/release/xdg-desktop-portal-tui /usr/lib/xdg-desktop-portal-tui
install -Dm644 data/tui.portal /usr/share/xdg-desktop-portal/portals/tui.portal
install -Dm644 data/org.freedesktop.impl.portal.desktop.tui.service \
    /usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.tui.service
```

Then choose it for file dialogs in `~/.config/xdg-desktop-portal/portals.conf`:

```ini
[preferred]
org.freedesktop.impl.portal.FileChooser=tui
```

and restart the portal (`pkill -x xdg-desktop-portal`; D-Bus starts it again when
needed). Firefox uses the portal when `widget.use-xdg-desktop-portal.file-picker`
is `1` in about:config.

`--browser PATH` picks with a file-browser-tui other than the one on `PATH`.

To float the window in niri:

```kdl
window-rule {
    match app-id="xdg-desktop-portal-tui"
    open-floating true
    default-window-height { proportion 0.7; }
    default-column-width { proportion 0.8; }
}
```

## License

GPL-3.0-only
