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
- **Snippets.** Blocks of text you paste often, searchable and copied with
  `Enter`.
- **Clipboard history.** Everything you copy is kept for thirty days and found
  by searching its text, with the application you copied it from shown beside
  it. Entries that look like credentials are *never written down* -- not
  redacted, not stored, skipped -- and so are entries any application marked
  not-for-history, which is what every password manager does. It only appears
  once you have typed something: the root list stays about applications.
- **Script commands (tier T1).** Drop any executable into the scripts folder
  with a `@lumen.title` comment header and it becomes a searchable command.
  Headers are read at index time, never by running the file (AD-08).
- **Window management.** Seventeen layouts in the list -- halves, quarters,
  thirds, two-thirds, maximise, centre, and move to the next or previous
  monitor -- applied to the window you were using when you opened the launcher.
  The edges land flush: Windows draws a window's resize border outside what you
  see, and Lumen compensates for it, so tiled windows meet with no seam and do
  not hang off the screen.
- **File search** across the folders you keep your own things in -- Desktop,
  Documents, Downloads, Pictures, Video, Music, plus anything else you name.
  `Enter` opens with the default application, `Ctrl+K` copies the path.
  Dependency and build trees are never walked into, so the index is about your
  files rather than about `node_modules`.
- **Lumen's own commands** in the list: open settings, reload sheets, quit.
- **An action panel** on `Ctrl+K`: every action a row has, reachable from the
  keyboard, which is what F2 asks for.
- **Inter bundled into the binary** rather than the system font, so the interface
  is the same typeface on Windows and on Hyprland.
- **An icon set**, eight glyphs drawn for this project on a 16-and-20 px grid
  and compiled into the binary, so every row has something in its icon square
  even when the shell has no icon to give. They take their colour from the
  theme's `icon` token.
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
- **The settings window has no icons.** The launcher's rows do; the sheet
  editor is still text and buttons.
- **File search is not a search of the whole disk.** It walks the folders
  listed above, to a bounded depth and a bounded count, and matches from
  memory. It finds the file you were working on, not every file on the machine.
  Nothing watches those folders yet either, so a file created after the daemon
  started is found on the next restart or `lumen reload-sheets`.
- **Windows at a higher integrity level cannot be arranged.** A layout applied
  to an elevated window does nothing, and Windows reports no error for it to
  report back.
- **No text expansion.** Snippets are copied from the launcher. Expanding them
  as you type anywhere would need a global keyboard hook, which is the one
  mechanism this project deliberately avoids (see R-13).
- **Browsers and Electron apps report their container.** A sheet keyed on
  `chrome.exe` covers every tab, because the window does not say what is open
  inside it. Key those on the window title instead. Packaged apps *are* resolved
  correctly, by descending past `ApplicationFrameHost`.
- **Two of the six budgets are enforced in CI, not six.** Ranking and installed
  size break the build on a regression. The other four are measured by the
  daemon on a real desktop and recorded below, because a hosted runner has no
  GPU, no display and no idle session to measure them on.
- **Linux is stubs.** `lumen-platform-linux` compiles on every commit and every
  method is `todo!()`. That is deliberate — see I-2.

---

## Measurements

Numbers, not adjectives. Reproduce with `cargo bench -p lumen-core --bench search`.

**Ranking cost per keystroke** (the p95 < 8 ms per-frame budget). Ryzen laptop,
release build, median of criterion's samples:

| Index size | Root list | One character typed | Typical query |
| ---------- | --------- | ------------------- | ------------- |
| 500 entries (a normal Start menu) | 73 µs | 90 µs | 53 µs |
| 2 000 entries | 374 µs | 395 µs | 235 µs |
| 5 000 entries (every SDK installed, and the file-search cap) | 1.09 ms | 1.06 ms | 558 µs |
| 20 000 entries | 7.03 ms | 7.42 ms | 2.81 ms |

The worst case at 5 000 is 1.1 ms against a budget of 8 ms for the whole frame,
which is where the decision to keep ranking a plain linear pass over prepared
haystacks comes from: there is no index to invalidate and nothing to get subtly
wrong.

The 20 000 row is why `files.limits.max-entries` defaults to 5 000 rather than
to something more generous. Ranking is linear in the number of candidates, and
the worst case is a single character typed — where nothing is filtered out
early. At 20 000 entries that one list spends the entire frame budget. The
number was chosen by running this benchmark, not by taste, and raising it is
supported with the cost stated above.

### Every budget in section 5, and where each number comes from

Release build, Ryzen 7 7735HS laptop, Radeon 660M, 1920 × 1080 at 125 %,
Windows 11. The daemon reports the four it can see itself at `info`, so these
are reproducible by running it and reading the log.

| Budget | Threshold | Measured | |
| ------ | --------- | -------- | --- |
| Per-frame ranking | p95 < 8 ms | 1.06 ms at 5 000 entries | ✅ |
| Gesture → first frame | p50 < 40 ms | **8 ms** (p95 26 ms, 15 summonses) | ✅ |
| Idle resident memory | < 150 MB | **84 MB** working set, window hidden | ✅ |
| Idle CPU | < 0.2 % | **0.08 %** over 30 s, window hidden | ✅ |
| Cold start to usable | < 400 ms | **354 ms** from `main` to summonable | ✅ |
| Installed size | < 40 MB | **10.6 MB**, the whole install | ✅ |

What each measurement excludes is stated in
[`timing.rs`](crates/lumen-app/src/timing.rs) rather than left to be assumed.
Gesture-to-first-frame starts when the message arrives, so it misses the few
hundred microseconds before `WM_HOTKEY` and the compositor's present after the
frame is drawn; cold start ends when the daemon can answer a summons, which by
I-3 is before the application index has landed, because root search never waits
on it either.

**The first summons used to cost 101 ms**, past the 80 ms p95, because the
first frame a GPUI process paints builds pipelines, compiles shaders and fills
the glyph atlas. The daemon now draws one frame nobody sees — the overlay,
mapped far outside the virtual screen and hidden again — once it is already
summonable. That took the first summons to 20 ms and left cold start where it
was.

**What CI enforces, and what it does not.** I-8 asks for the budgets to be
measured in CI with a regression breaking the build. Two of the six are:
ranking, by a test that asserts ranking stays linear in the size of the index
rather than asserting a stopwatch reading a shared runner cannot honour, and
installed size, by comparing bytes. The other four need a compositor presenting
to a real display, a genuinely idle session and a GPU. A hosted runner has
none, and a number taken there would look like evidence without being any. So
the daemon measures them and this table records them; the gap is stated rather
than filled.

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
| `lumen reload-sheets`   | Re-read sheets, scripts, snippets and files from disk |
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
| `Ctrl+K` | Show every action for the selected row |

---

## Configuring

On first run Lumen writes `%APPDATA%\Lumen\config\config.json`:

```json
{
  "hotkey": { "modifiers": { "alt": true }, "key": "Space", "hold": null },
  "theme": "nocturne",
  "backdrop": "system-acrylic",
  "window": { "width": 720.0, "visible-rows": 8, "vertical-bias": 0.22 },
  "clipboard": {
    "enabled": true,
    "retention": { "max-entries": 500, "max-age-days": 30 }
  },
  "files": {
    "enabled": true,
    "roots": [],
    "limits": { "max-entries": 5000, "max-depth": 6 }
  }
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

The `icon` token colours the bundled glyphs. It sits between `text.primary` and
`text.muted` on purpose: a 20-px glyph carries less ink than a word, so muted
makes it vanish and primary makes it shout over the title beside it.

A theme may set **colour and type family only**. Geometry — spacing, radii, row
heights, icon sizes — is not themeable, deliberately: it is the calibration the
whole project exists to protect.

### Script commands

Any executable in `%APPDATA%\Lumen\config\scripts\` with a `@lumen.title`
header becomes a command in the launcher. An example is written there on first
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

### Clipboard history

Everything you copy as text is kept in
`%APPDATA%\Lumen\data\clipboard.sqlite3` and found by searching its text. Rows
show a preview and the application the text came from. `Enter` copies it back.

The history keeps the last 500 entries or 30 days, whichever comes first, and
re-copying something moves it back to the top rather than adding a duplicate.
It is only searched once you have typed something, and at most eight rows come
from it at a time, so it can never take the list over.

**What is never written down.** The protection is a decision made before the
write, not a mask applied when drawing, because a masked secret is still a
secret sitting in an unencrypted file:

- Anything copied out of a known password manager: 1Password, Bitwarden,
  KeePass and KeePassXC, LastPass, Dashlane, Enpass, NordPass, Proton Pass.
- Anything an application marked not-for-history, using any of the three
  Windows clipboard formats that mean it. Password managers set these, and so
  do most terminals when you copy a masked field.
- Anything that matches a credential format: PEM private-key blocks, AWS access
  keys, GitHub, Slack, OpenAI, Anthropic, Google and Stripe live keys, JWTs,
  and URLs with a username and password in them.

A rejected entry produces no row, no file and no log line containing the text.
The cost is that a false positive is lost silently, which is the right side of
that trade; the cases that must *not* be rejected are tests.

To turn it off, set `clipboard.enabled` to `false`. The listener is then never
started at all. To keep less, set the retention:

```json
"clipboard": {
  "enabled": true,
  "retention": { "max-entries": 200, "max-age-days": 7 }
}
```

### File search

By default Lumen walks your Desktop, Documents, Downloads, Pictures, Video and
Music folders — resolved through the system's own idea of where those are, so a
Documents folder redirected to OneDrive is followed. `Enter` opens a file with
its default application; `Ctrl+K` offers to copy its path.

To search somewhere else instead, name the folders:

```json
"files": {
  "enabled": true,
  "roots": ["D:/work", "C:/Users/me/notes"],
  "limits": { "max-entries": 5000, "max-depth": 6 }
}
```

Leaving `roots` empty keeps the defaults, which is why it is empty rather than
pre-filled: a config file copied to another machine still works.

**`max-entries` is a frame-budget number.** Ranking is linear in the number of
candidates and the worst case is one character typed. The measurements section
above has the cost of raising it; 5 000 leaves the frame room to spare and
20 000 does not.

Not walked into, ever: `node_modules`, `target`, `build`, `dist`, `.git`,
`.venv`, `AppData` and about fifteen others, matched by directory name. Files
beginning with a dot are skipped.

The index is built once when the daemon starts. A file created afterwards is
found on the next restart, or by running `lumen reload-sheets`, which also
rebuilds it.

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
