# Lumen

A native command palette written entirely in Rust, with its interface built on
[GPUI](https://gpui.rs). Windows 11 today, Arch Linux + Hyprland later.

The priority is quality: imperceptible latency, a near-zero idle footprint, and
an aesthetic worth looking at fifty times a day.

> **Status: early.** F0 (the platform spike) is done and most of F1 (the minimum
> usable launcher) works. It indexes and launches applications, and it is
> pleasant to use. It is not yet the author's default launcher.

---

## What works

- **Resident daemon, one window.** The window is created once at startup and
  from then on only shown and hidden, so invoking the launcher never pays GPU
  initialisation cost.
- **Global hotkey** (`Alt+Space` by default), suppressed so it does not also
  reach the focused application.
- **Application index** covering Win32 shortcuts *and* packaged apps in one pass,
  read from the shell's `AppsFolder` — the same list as Start → All apps.
- **High-resolution icons** via `IShellItemImageFactory`, extracted off the UI
  thread, cached, and requested well above their drawn size so they stay crisp
  at any display scale.
- **Fuzzy matching** with `nucleo`, ranked by frequency and recency of use, with
  the matched characters highlighted.
- **Themes as data** with live reload: edit the JSON, see it on screen, no
  rebuild. Two in-house themes ship, one dark and one light.
- **Windows 11 chrome**: acrylic backdrop, rounded corners, dark mode, out of
  Alt+Tab and the taskbar, appearing on the monitor holding the cursor, and
  returning focus to exactly the window it took it from.
- **Daemon/client IPC** over a named pipe, so the Hyprland activation pattern
  already works the way F5 will need it.

## What does not work yet

Listed because a README that implies more than exists is worse than no README.

- **No IME.** The search field is a plain string with a drawn caret, not a GPUI
  `EntityInputHandler`, so composition input does not work. ASCII typing,
  backspace, word-delete and paste all do.
- **No bundled typeface.** F2 calls for Inter or Geist compiled into the binary;
  today the system font is used, which is the one thing that visibly differs
  between Windows and Linux.
- **No action panel and no icon set.** `Enter` runs the primary action; there is
  no `Ctrl+K` panel behind it yet.
- **No internal providers.** No calculator, clipboard history, snippets or file
  search — that is all F3.
- **No shortcut overlay.** The press-and-hold gesture is detected and plumbed end
  to end, but nothing draws yet; the overlay needs its own window.
- **Budgets are only partly measured.** See below: ranking is benchmarked, the
  rest is not yet, and a regression does not yet fail the build (I-8).
- **Linux is stubs.** `lumen-platform-linux` compiles on every commit and every
  method is `todo!()`. That is deliberate — see I-2.

---

## Measurements

Numbers, not adjectives. Reproduce with `cargo bench -p lumen-core --bench search`.

**Ranking cost per keystroke** (the p95 < 8 ms per-frame budget). Ryzen laptop,
release build, median of criterion's samples:

| Index size | Root list | One character typed | Typical query |
| ---------- | --------- | ------------------- | ------------- |
| 500 entries (a normal Start menu) | 58 µs | 69 µs | 41 µs |
| 2 000 entries | 294 µs | 306 µs | 175 µs |
| 5 000 entries (every SDK installed) | 846 µs | 812 µs | 421 µs |

The worst case is 0.85 ms against a budget of 8 ms, which is where the decision
to keep ranking a plain linear pass over prepared haystacks comes from: there is
no need for an index to invalidate, and nothing to get subtly wrong.

**Idle resident memory** is roughly 85 MB with the window hidden and the full
index loaded, against a budget of 150 MB. That figure is from Task Manager on a
debug build, not from a harness, so treat it as an indication rather than a
measurement.

**Not yet measured:** gesture-to-first-frame (p50 < 40 ms), idle CPU (< 0.2 %),
cold start (< 400 ms), installed size (< 40 MB). Each needs a harness that
drives the real binary, which is the remaining F1 work.

---

## Building

Requires a recent stable Rust toolchain and the MSVC build tools.

```sh
cargo build --release
```

Then run the binary once to start the daemon:

```sh
./target/release/lumen
```

| Command                 | Effect                                        |
| ----------------------- | --------------------------------------------- |
| `lumen`                 | Toggle the launcher, starting the daemon if needed |
| `lumen show` / `hide`   | Show or hide explicitly                       |
| `lumen overlay-press`   | Report that the hold gesture began (Hyprland `bind`)  |
| `lumen overlay-release` | Report that it ended (Hyprland `bindr`)       |

### Keys

| Key | Action |
| --- | ------ |
| `↓` / `↑`, `Ctrl+N` / `Ctrl+P` | Move the selection |
| `Enter` | Run the selected item |
| `Esc` | Clear the query; dismiss when already empty |
| `Ctrl+W`, `Ctrl+Backspace` | Delete the previous word |
| `Ctrl+U` | Clear the query |
| `Ctrl+V` | Paste |

---

## Configuring

On first run Lumen writes `%APPDATA%\Lumen\config\config.json`:

```json
{
  "hotkey": { "modifiers": { "alt": true }, "key": "Space", "hold": null },
  "theme": "nocturne",
  "backdrop": "system-acrylic",
  "window": { "width": 720.0, "visible-rows": 8, "vertical-bias": 0.22 }
}
```

**If the launcher does not respond to `Alt+Space`,** another application already
holds that combination — global hotkeys on Windows are first come, first served.
Lumen still runs and still responds to `lumen toggle`; change `"key"` (or the
modifiers) and restart. The log names the conflict explicitly.

`backdrop` selects how the window background is produced:

| Value | Meaning |
| ----- | ------- |
| `system-acrylic` | `DWMSBT_TRANSIENTWINDOW`, the Acrylic Windows 11 designed for flyouts. The default. |
| `legacy-acrylic` | The undocumented `SetWindowCompositionAttribute` blur. Use if the documented attribute is ignored on your build. |
| `solid` | No blur at all. Visually defensible, and the fallback if neither works. |

### Themes

Themes are pure data — no code, no runtime, nothing to review. Drop a JSON file
into `%APPDATA%\Lumen\config\themes\` and it is available; edit the active one
and the change appears immediately, without restarting.

```json
{
  "schema_version": 1,
  "name": "Nocturne",
  "appearance": "dark",
  "colors": {
    "surface.background": "#0E0E12D8",
    "text.primary": "#EDEDF2",
    "accent": "#7C6BD6"
  }
}
```

Every token is optional and falls back to the base theme, so a theme written
today still loads after the palette grows. Tokens are semantic
(`surface.background`, `text.muted`, `accent`) and never name components.

A theme may set **colour and type family only**. Geometry — spacing, radii, row
heights, icon sizes — is not themeable, deliberately: it is the calibration the
whole project exists to protect.

---

## Layout

```
crates/
  lumen-core/              index, matching, ranking, theming    no OS, no GPUI
  lumen-platform/          trait Platform and its value types   the OS boundary
  lumen-platform-windows/  the Windows 11 implementation
  lumen-platform-linux/    the Wayland implementation (stubs, always compiles)
  lumen-ui/                the GPUI presentation layer
  lumen-app/               the daemon and its thin client
```

The dependency rule is the point. `lumen-core` has no `#[cfg(windows)]`, no
`#[cfg(unix)]`, no system calls and no GPUI: everything that touches the system
goes through `trait Platform`, and everything that draws lives in `lumen-ui`.
CI cross-checks the Linux crates on every commit so that a Windows assumption
leaking into the core is caught the same day rather than in a year.

Full rationale, decisions and risks: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).
The invariants and performance budgets that constrain day-to-day work:
[`CLAUDE.md`](CLAUDE.md).

---

## Licence

MIT OR Apache-2.0, at your option. See [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE).

Lumen is an independent project. It studies the architecture of
[Vicinae](https://github.com/vicinaehq/vicinae) and
[Asyar](https://github.com/asyarapp/asyar) — both GPL-3.0 — but contains none of
their code, and it implements neither Raycast's API nor its store.
