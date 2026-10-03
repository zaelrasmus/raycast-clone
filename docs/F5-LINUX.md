# F5: Linux and Wayland

**Status: frozen.** Nothing here is scheduled. This is the notebook to open on
the day F5 starts, so the work begins from a list and not from a blank page.

Target: **Arch Linux + Hyprland**, and other compositors that implement
`wlr-layer-shell` (Sway, KWin). GNOME/Mutter do not implement it and are out of
scope (plan, section 8).

Estimate from the plan: **3–6 weeks**, *if* I-1 and I-2 held. They have: CI has
cross-checked `lumen-platform-linux` on every commit since F0.

---

## What already exists

| Piece | State |
| --- | --- |
| `lumen-core` | Portable. No `cfg`, no OS calls (I-1). Config, search, frecency, sheets, providers, clipboard store and view model all run unchanged on Linux. |
| `lumen-ui` | Portable. GPUI renders on Wayland already; only window *placement* differs (see layer-shell below). |
| `lumen-app` | Portable except `build.rs` (Windows icon resource, already `cfg`-gated) and `build_platform`, which already has a Linux arm. |
| IPC | `interprocess` namespaced local socket: a named pipe on Windows, an abstract Unix socket on Linux. `lumen toggle` / `overlay-press` / `overlay-release` already exist for Hyprland binds. |
| Paths | `directories` crate → `~/.config/lumen`, `~/.local/share/lumen`. File search roots come from XDG user dirs through the same crate. |
| `lumen-platform-linux` | Every method of `trait Platform` is stubbed with `todo!()` and a comment saying how it will be done. Compiles in CI (I-2). |

So F5 is **implementing one trait**, plus the GPUI layer-shell problem, plus
packaging.

---

## The one real risk: layer-shell in GPUI

Wayland gives a normal window no way to put itself on top, position itself, or
grab the keyboard globally. A launcher has to be a `wlr-layer-shell` surface on
`LAYER_OVERLAY` with exclusive keyboard interactivity.

GPUI 0.2.2 (pinned, I-6) does not create layer surfaces. Options, in order of
preference:

1. **Check upstream first.** Zed's GPUI has been adding Wayland features
   steadily. If a newer release supports layer shell, update the pin, as a
   deliberate commit with its own verification (I-6).
2. **A small patch to GPUI's Wayland backend**, carried as a `[patch]` in the
   workspace `Cargo.toml`. The plan accepts this as planned maintenance (R-5,
   R-6).
3. **A fork**, only if the patch cannot be kept small.

Three surfaces, three treatments:

| Role | Wayland surface | Keyboard | Notes |
| --- | --- | --- | --- |
| Launcher | layer surface, `LAYER_OVERLAY` | exclusive | Anchored top, with a margin standing in for `vertical-bias` |
| Overlay | layer surface, `LAYER_OVERLAY` | none | Empty input region = click-through, like `WS_EX_TRANSPARENT` |
| Settings | ordinary `xdg_toplevel` | normal | A normal window; no layer shell needed |

The consequence for `Rect`-based placement: under layer-shell **the compositor
places the surface**. `show_window` / `set_window_bounds` translate a `Rect`
into *size + anchor + margins*. `MonitorInfo::place` still does the arithmetic,
and the Linux side turns its result into margins.

`warm_the_renderer` (in `main.rs`) draws one frame off-screen to pay the
first-frame cost early. A layer surface cannot be placed off-screen, so on
Linux it becomes a zero-opacity frame on the real output, or is dropped if the
first-frame budget holds without it. Measure before deciding.

---

## `trait Platform`, method by method

Each row is one method in `crates/lumen-platform-linux/src/lib.rs`. **H** means
the answer goes through Hyprland's IPC socket (`$XDG_RUNTIME_DIR/hypr/<sig>/.socket.sock`,
the same thing `hyprctl` talks to). Talk to the socket directly rather than
shelling out to `hyprctl`: it's one JSON request, and a process spawn per
keystroke would show in the budgets.

| Method | Approach | Difficulty |
| --- | --- | --- |
| `attach_window` | Turn the GPUI window into a layer surface (see above) | **Hard**: depends on the GPUI patch |
| `show_window` / `hide_window` | Map / unmap the layer surface. Unmapping gives focus back by itself, so `restore_to` goes unused | Medium |
| `set_window_bounds` | Set the layer surface's size and margins; the compositor positions it | Medium |
| `cursor_monitor` | **H** `j/monitors` → the focused monitor; `wl_output` for scale | Easy |
| `foreground_application` | **H** `j/activewindow` → `class` and `title`. Simpler than Windows, with no process handle or frame host involved | Easy |
| `start_gesture_listener` | **No-op, return `Ok(())`**, for the timers key too (`bind = ALT SHIFT, SPACE, exec, lumen timers`). The compositor owns the shortcut (see binds below). The current `todo!()` should become a no-op, not an implementation | Trivial |
| `enumerate_applications` | Parse `.desktop` files from `$XDG_DATA_HOME/applications` and every `$XDG_DATA_DIRS/*/applications`. Respect `NoDisplay`, `Hidden`, `OnlyShowIn`/`NotShowIn`, `TryExec`; later directories are overridden by earlier ones with the same desktop-file ID | Medium |
| `watch_applications` | `inotify` (the `notify` crate is already a dependency) on the same folders. Package managers write `.desktop` files there and nowhere else. Keep the coalescing idea from `app_watch.rs` | Easy |
| `load_icon` | Icon theme spec: current theme → `hicolor` → `/usr/share/pixmaps`, choosing the closest size ≥ requested; SVGs through `resvg` (GPUI already renders SVG) | Medium |
| `launch` | `Reuse`: **H** `j/clients`, match `class` against the entry's `StartupWMClass` or desktop-file ID, then `dispatch focuswindow address:<addr>`. Otherwise run `Exec` with the field codes (`%f %u %F %U %i %c %k`) stripped or expanded, detached (`setsid`) | Medium |
| `start_clipboard_listener` | `wlr-data-control` (via `wayland-client` + `wayland-protocols-wlr`), the protocol built for clipboard managers. **Privacy equivalent** of the Windows exclusion formats: skip offers carrying the MIME type `x-kde-passwordManagerHint` = `secret` (KeePassXC and others set it) | Medium |
| `clipboard_text` / `set_clipboard_text` | Same `wlr-data-control` device, or GPUI's own clipboard | Easy |
| `run_script` | `std::process::Command`; respect the executable bit and shebang; no console-window flag needed | Easy |
| `spawn_extension` | Same as `run_script`, resolved against the extension's folder, not `PATH` | Easy |
| `arrange_window` | **H** `dispatch focuswindow`, then `dispatch movewindowpixel` / `resizewindowpixel` or `setfloating`. `lumen_platform::tiling` already does the arithmetic. On a tiling compositor, decide whether "left half" means *float it there* or *tile it there*; that's a design call, not a port | Medium |
| `install_tray` | `StatusNotifierItem` over D-Bus (`ksni` crate). Waybar and the other Hyprland bars display it | Easy |
| `installed_browsers` | Desktop entries whose `MimeType=` lists `x-scheme-handler/https`, named by their `Name=`. The same scan as `enumerate_applications`, filtered | Easy |
| `open_url` | `None`: `xdg-open`. `Some`: the browser's `Exec` with `%u` replaced by the URL, detached like `launch`. Keep `Browser::name` the desktop entry's `Name=` so `preferred-browser` is the same string on both systems | Easy |
| `alert_sounds` | The XDG sound theme's `stereo/` folder (freedesktop: `alarm-clock-elapsed`, `bell`, `complete`...). Windows names (`Alarm01`) will not exist: a saved name that is not listed plays nothing and logs, so decide whether to map the Windows default to `alarm-clock-elapsed` | Easy |
| `play_alert` / `stop_alert` | Spawn `pw-play` (or `paplay`) on the file, looping by respawning while `repeat`; stop by killing the child. One child at a time, like `PlaySoundW` | Easy |
| `recent_files` | Stubbed as an empty list, so *Search Files* shows Lumen's own recents only. Read `$XDG_DATA_HOME/recently-used.xbel` (the GTK list file managers write), newest `modified` first, `file://` URIs that exist | Easy |
| `show_in_folder` | Stubbed as an error. `org.freedesktop.FileManager1.ShowItems` over D-Bus (Nautilus, Dolphin, Thunar implement it); without it, `xdg-open` on the parent folder | Easy |
| `open_with` | Stubbed as an error. The desktop portal's `OpenURI` with `ask: true`, which shows the chooser | Easy |
| `pick_folder` | Stubbed as `Ok(None)`, so *Add Folder* and *Choose folders...* add nothing. The desktop portal's `FileChooser.OpenFile` with `directory: true` (`ashpd`), parented to the window in front, with `current_folder` set from `start`; the chosen URI's path | Easy |
| `internal_drives` | Stubbed as an empty list, so *Search Files* offers no other drive. `udisks2` over D-Bus: block devices not `Removable`, not the root filesystem's, by their mount point and `IdLabel`; or `/proc/self/mounts` filtered to local filesystems under `/mnt`, `/media` and `/run/media` | Medium |

---

## Hyprland configuration the user writes

```ini
# ~/.config/hypr/hyprland.conf

# Start the daemon with the session.
exec-once = lumen

# Summon. A second press toggles over the socket (AD-02).
bind = ALT, SPACE, exec, lumen

# The shortcut overlay: separate press and release binds, so the daemon
# measures the hold with no input capture and no special permissions.
bind  = , Control_R, exec, lumen overlay-press
bindr = , Control_R, exec, lumen overlay-release

# Blur comes from the compositor, not the application (plan, section 8).
layerrule = blur, lumen
layerrule = ignorezero, lumen
```

The layer surface's *namespace* must be `lumen` for the `layerrule` lines to
match. Set it when creating the surface.

On Linux, `hotkey` in `config.json` does nothing, because the binds above
replace it. The settings UI and README should say so rather than silently
ignore it.

---

## Outside the trait

- **Install.** An `install.sh` counterpart to `scripts/install.ps1`: binary to
  `~/.local/bin`, a `lumen.desktop` (so Lumen can find *itself*), and a note
  about the Hyprland lines above. Later, an AUR `PKGBUILD`.
- **Autostart.** `exec-once` above, or a systemd user unit for people who
  prefer one.
- **Fonts.** Already bundled and registered with GPUI, so text looks the same.
  Rasterisation will differ slightly (FreeType versus DirectWrite); the plan
  accepts that.
- **Theme appearance.** The Windows platform takes `dark` for the Mica/Acrylic
  tint. On Linux the backdrop is the compositor's blur, so `BackdropMode` maps
  to "translucent surface or not" and nothing more.
- **CI.** Today CI *checks* Linux. F5 adds a Linux *build* and runs the tests
  there. The four budgets that need a display (see `ARCHITECTURE.md` 7.24) are
  measured by hand on the Arch machine, as on Windows.

---

## Suggested order

Each step should be usable on its own, so the port can stop at any point
without leaving something half-working.

1. **Headless half:** `enumerate_applications`, `watch_applications`,
   `load_icon`, `run_script`, `spawn_extension`, `launch` (without reuse). All
   testable without a window.
2. **Layer-shell spike.** Get *one* GPUI window onto `LAYER_OVERLAY` with
   keyboard focus. This is the risk, so find out early how big it is.
3. **Launcher usable:** show/hide/bounds, `cursor_monitor`, the binds, the
   no-op gesture listener. Everything that is the daily-use core on Windows
   now works on Linux.
4. **Hyprland IPC:** `foreground_application` (so the overlay picks the right
   sheet), launch-reuse, `arrange_window`.
5. **Clipboard:** `wlr-data-control` with the password-manager hint.
6. **Tray, install script, docs.**

## Acceptance (from the plan)

- The same six performance budgets are met on Linux.
- Side by side with Windows, indistinguishable apart from the background blur.
