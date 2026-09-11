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
- **Context-aware shortcut overlay.** Hold Right Ctrl and a sheet of shortcuts
  for whatever application is in front appears; release it and it goes away.
  The shortcuts are *yours*, written by you, and the sheet follows the
  foreground application. It never takes focus, is click-through, and is not in
  the keyboard path at all, so it cannot swallow the keystroke that follows it.
- **A settings window** for writing those sheets, so configuring the overlay
  does not mean hand-editing JSON. It offers to start a sheet for whichever
  application you were just using.
- **Inline calculator.** Type `12*7` and the answer is the first row, ready to
  copy. It stays quiet for anything that is not arithmetic, including `7-zip`.
- **Script commands (tier T1).** Drop any executable into the scripts folder
  with a `@lumen.title` comment header and it becomes a searchable command.
  Headers are read at index time, never by running the file (AD-08).
- **Lumen's own commands** in the list: open settings, reload sheets, quit.
- **Themes and sheets as data** with live reload for themes: edit the JSON, see
  it on screen, no rebuild. Two in-house themes ship, one dark and one light.
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
- **Most internal providers are missing.** The calculator and tier T1 are in;
  clipboard history, snippets, window management and file search are not.
- **Browsers and Electron apps report their container.** A sheet keyed on
  `chrome.exe` covers every tab, because the window does not say what is open
  inside it. Key those on the window title instead. Packaged apps *are* resolved
  correctly, by descending past `ApplicationFrameHost`.
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

**Idle resident memory** is roughly 70 MB with the window hidden and the full
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
| `lumen settings`        | Open the settings window                      |
| `lumen reload-sheets`   | Re-read the shortcut sheets from disk         |
| `lumen quit`            | Stop the daemon                               |

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

### Script commands

Any executable in `%APPDATA%umennfigscripts` with a `@lumen.title`
header becomes a command in the launcher. an example is written there on first
run; delete it once you have your own.

```powershell
# @lumen.title copy ip address
# @lumen.subtitle example script command
# @lumen.mode copy
# @lumen.keywords network, ip

(get-netipaddress -addressfamily ipv4 | select-object -first 1 -expandproperty ipaddress)
```

| header | meaning |
| ------ | ------- |
| `@lumen.title` | required. what appears in the list. |
| `@lumen.subtitle` | optional second line. |
| `@lumen.keywords` | optional, comma separated. matched but never shown. |
| `@lumen.mode` | `silent` (default) runs and ignores the output; `copy` puts the output on the clipboard. |

the comment character is whatever the language uses — lumen looks for the
`@lumen.` marker and does not care what precedes it, so `.ps1`, `.cmd`, `.bat`
and plain executables all work. `.ps1` runs through powershell with `-noprofile`,
`.cmd` and `.bat` through `cmd`, and anything else is executed directly.

headers are read when the folder is indexed; the file itself is run only when you
pick its row. nothing executes to populate the list (ad-08). scripts get no
console window, run from their own folder, and are abandoned after 30 seconds if
they have not finished.

### shortcut sheets

Hold **Right Ctrl** for about a third of a second and Lumen shows the shortcuts
for whatever application is in front.

The easiest way to write them is the settings window: run `lumen settings`, or
search for **Configure Shortcuts** in the launcher. It lists your sheets and lets
you edit the name, what the sheet matches, and the groups of shortcuts — and it
offers to start a sheet for whichever application you were using a moment ago, so
you never have to look up your own executable's file name. `Ctrl+S` saves, `Esc`
closes, and the overlay picks the change up immediately.

Sheets remain plain files, distributed exactly like themes: drop a JSON file into
`%APPDATA%\Lumen\config\sheets\` and it is picked up without a restart.

```json
{
  "schema_version": 1,
  "name": "Visual Studio Code",
  "match": { "on": "executable", "any": ["code.exe"] },
  "groups": [
    {
      "title": "Navigate",
      "shortcuts": [
        { "keys": "Ctrl+P", "description": "Go to file" },
        { "keys": "Ctrl+Shift+P", "description": "Command palette" }
      ]
    }
  ]
}
```

`match` takes one of three forms, tried in this order:

| Form | Matches |
| ---- | ------- |
| `{ "on": "executable", "any": ["code.exe"] }` | The foreground executable, case-insensitively. |
| `{ "on": "title-contains", "any": ["Mozilla Firefox"] }` | A substring of the window title. For browsers and Electron apps, which do not identify themselves any other way. |
| `{ "on": "global" }` | Anything. The last resort. |

The chain is exhaustive by design: **the overlay never appears empty**. If no
sheet matches and you have written no global sheet, a built-in one is used.

`keys` is free text. Lumen is describing *other* applications' shortcuts, which
it cannot parse, validate or intercept — so write what you actually press.
`"Ctrl+Shift+P"` is drawn as three keycaps; `"Hold Right Ctrl"` is drawn as one.

Lumen's own sheet is not a file: it is generated from the keymap, so it can
never drift from the keys that are actually bound.

To change the hold key, set `hotkey.hold` in `config.json` to one of
`left_control`, `right_control`, `left_alt`, `right_alt`, `left_shift`,
`right_shift`, or `null` to turn the gesture off. Avoid the Super keys —
releasing one opens the Start menu, and suppressing that would require a
low-level keyboard hook the project deliberately does not install.

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
