# Lumen

A native command palette written entirely in Rust, with its interface built on
[GPUI](https://gpui.rs). Windows 11 today, Arch Linux + Hyprland later.

The priority is quality: imperceptible latency, a near-zero idle footprint, and
an aesthetic worth looking at fifty times a day.

> **Status: in daily use on Windows.** The launcher, the application index, the
> shortcut overlay and its settings window are what the author uses every day.
> Extensions (F4) and the Linux port (F5) are frozen; see
> [`docs/F5-LINUX.md`](docs/F5-LINUX.md) for where the port starts.

---

## What works

- **Resident daemon, one window.** The window is created once at startup and
  from then on only shown and hidden, so invoking the launcher never pays GPU
  initialisation cost.
- **Global hotkey** (`Alt+Space` by default), suppressed so it does not also
  reach the focused application.
- **Application index** covering Win32 shortcuts *and* packaged apps in one pass,
  read from the shell's `AppsFolder` — the same list as Start → All apps. It
  follows installs and uninstalls by itself, a few seconds after they happen,
  by watching the Start Menu folders and the packaged-app registry.
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
  application you were just using. Since F15 it is laid out after Raycast's:
  pages down the left with a search over them (`Ctrl+Tab` steps through
  them), and settings in cards of rows with toggles and compact dropdowns,
  in the active palette's colours and typeface.
- **Inline calculator** (F14). Type a calculation and the answer is the first
  thing in the list, drawn as a card -- what you typed, and the result --
  ready to copy with `Enter`:
  - functions and constants: `sqrt(2)^pi / log(1000) * sin(45 deg)`, `5!`;
  - units: `5 km in mi`, `100 f in c`, `1 GiB in MB`, `100 km/h in mph`,
    `2.4 ghz in mhz`, `1 kwh in mj`, `2 cups in ml` -- length, mass, time,
    temperature, data, angle, frequency, power, energy, speed, area, volume
    and pressure;
  - percentages: `25% off 399`, `20% of 50`, `399 + 25%`, `5 as % of 20`.
    `%` is a percentage; the remainder is `10 mod 3`;
  - dates: `days until 25 dec`, `weeks since 1 jan`, `today + 3 weeks`;
  - money, in any of it: `100 usd + 20 eur in cad`, `25% off 399 usd in
    eur`, dated by the rates it used.

  Numbers are shown and typed as your Windows region writes them --
  `1,800,000.5`, and `2,000 * 3` works -- and copied without the grouping,
  so they paste into any number field. *Plain* on the *Calculator* settings
  page turns that off.

  It stays quiet for anything that is not a calculation, including `7-zip`,
  `python 3.12` and `28 days later`. *Calculator History* keeps the answers
  you copied -- twenty by default, set on the *Calculator* settings page,
  where it can be cleared.
- **Snippets.** Blocks of text you paste often, in *Search Snippets*: found by
  name, keyword, tag or what they say, filtered by tag, grouped by when they
  last changed, shown in full beside the list, and pasted with `Enter` into
  the application you were in -- your clipboard put back afterwards.
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
- **File search** across your home folder by default, or the folders and
  drives you name, with exclusions inside them. Files and folders are both
  results; `Enter` opens a file with its default application and a folder in
  Explorer, `Ctrl+K` copies the path. Dependency and build trees, tool caches
  such as `.cargo` and `.npm`, ignore files and hidden entries are skipped, so
  the index is about your files rather than about `node_modules`. It stays current by itself: changes are watched while
  Lumen runs, and at start only the folders that changed while it was not are
  read again; *Rebuild File Index* starts over from nothing if it ever seems
  wrong.
- **Typed paths:** type `C:\Users\do`, `d:/src/lu` or `~\Doc` and root or
  *Search Files* lists that folder live, the entries whose names contain
  what follows the last separator; `Tab` completes the selected row.
- **Extensions with views of their own** (F4, up to step 3). One ships as
  an example: **Characters**. An extension is a folder with a
  manifest and a program; its commands are in the root list without anything
  being started, and choosing one starts its process, which sends views as
  JSON over stdio: sections, icons, accessories, coloured status tags and an
  action panel with the extension's own verbs. Anything it wants from the
  system, like the clipboard, it asks the host for.
- **Quicklinks and web search** (F6). A quicklink is a saved link: a channel,
  a dashboard, a search page. Put `{query}` in it and it searches: `g lofi
  beats` opens Google, and `yt` then `Tab` asks for a YouTube search in a pill
  beside the query. A link marked *fallback* is offered as "Search Google for
  ..." at the foot of any search that finds nothing better. Create one from
  the launcher with *Create Quicklink*, or edit, delete and mark them in
  settings; either way they live in `quicklinks.json` and reload as it
  changes. Each shows its site's icon, fetched once and kept, and the browser
  it opens in is picked from the ones installed. `Ctrl+K` on a link also
  opens it in a private window, or in any installed browser, just this once.
- **Type a URL, open it.** `github.com` or `https://example.com/page` puts
  *Open github.com* first; `Ctrl+K` opens it privately, in another browser,
  or saves it as a quicklink. Typed URLs are never written to Lumen's usage
  history, and nothing opened in a private window is.
- **Translate** (F7). *Translate* opens a view where what you type is
  translated as you pause, by Google (no setup) or DeepL (your own key).
  Save the directions you use as **presets** -- *Spanish → English*, or one
  source into up to four languages at once -- and each is its own row in the
  root list, so choosing one means you only type. In the view, the *From*
  and *To* pills choose any language at any time, without losing the text:
  click one, or `Tab` to open its list. `Enter` copies the translation and
  closes.
- **Timers, Pomodoro and focus** (F8). Type a duration -- `25m`, `1h30`, `90s`, `1:30`
  -- and *Start 25 min timer* leads the list; words after it name the timer,
  `25m write chapter 3`. A notification says when it ends. *Timers* shows
  what is running with the time left, and pauses, resumes, restarts or stops
  each one; *Stop Running Timer* stops the one closest to ending. Timers
  survive a restart of Lumen or of the machine, and cost nothing while they
  wait: the daemon sleeps until the next one ends. From a terminal, `lumen
  timer 25m tea`. Save the ones you start often as **presets** on the
  *Timers* settings page -- *Deep work*, alias `dw`, `1h30` -- and each is a
  row of its own; *Timers* also offers them, and quick 2, 5, 10 and 30
  minute timers, one Enter away. A timer ends with one of the system's
  alarm sounds -- hear them all with *Preview Alert Sounds* -- once, or
  until *Dismiss Ringing Timer* if you set it to ring until dismissed.
  *Start Stopwatch* counts up instead -- `stopwatch standup` names it --
  and it lives in *Timers* too, with pause, reset and stop.
  *Start Pomodoro* runs 4 × 25 minutes of work with 5 minute breaks and a
  15 minute break at the end, moving on by itself and telling you at each
  change; *Skip to Next Phase* is in `Ctrl+K`. Give any preset a *Break* in
  settings and it cycles the same way, with your own lengths and rounds.
  A set ends after its long break, or repeats until you stop it: choose
  per preset, with `Ctrl+K` when starting, or on the running timer.
  **Focus**: list the apps and sites that distract you in settings, then
  `focus 45m write chapter 3` -- or *Start with Focus* from `Ctrl+K` on any
  timer -- and Lumen won't open them until it ends, showing your goal and
  the time left instead, with *Open Anyway* if you mean it. In a Pomodoro
  it holds during work and lifts for breaks. Only what opens through Lumen
  is checked: a nudge, not a lock.
  **Seeing them**: the launcher's footer shows what is ringing or ending
  soonest, with how many more; `Alt+Shift+Space` (or `lumen timers`) opens
  *Timers* from anywhere. Presets can be made and edited without leaving
  the launcher: *Create Timer Preset*, and *Edit Preset* / *Save as
  Preset* in `Ctrl+K`.
- **Currency** (F9). *Currency* converts an amount -- `10`, `1+2` -- into
  all 166 currencies at once, with the date of the rates and who published
  them. `Tab` picks the currency you have from a list you can search by
  code, name or country (`can`, `yen`, `japan`), and it is remembered.
  `10 cad` converts from Canadian dollars whatever the picker says, and
  `10 in japan` or `10 to eur` narrows the rows. *Pin* in `Ctrl+K` keeps a
  currency on top. `Enter` copies the plain number. In the root list,
  `10 usd in cad` answers *14.16 CAD* straight away, from the rates Lumen
  already has -- and since F14 money works inside any calculation. The
  rates are fetched once a day at most, only when a conversion asks for
  them, and the last ones keep working offline, dated.
- **World Clock and Weather** (F13). Over 34,000 cities, bundled, so finding
  one never needs the internet. Type a city, a region, a country or a
  continent -- `vancouver`, `california`, `canada`, `europe` -- or go into a
  continent and a country with `Enter`, shown as a pill (`Europe › Spain`);
  `Escape` comes back out. *World Clock* shows each place's time and how far
  it is from yours, and, if you want, when your working days overlap.
  *Weather* shows the conditions and the week ahead. *My Places* keeps the
  cities you care about first, in both. In the root list, `time in vancouver`,
  `3pm vancouver` and `weather in vancouver` are answered directly.
- **Lumen's own commands** in the list: open settings, reload sheets, quit.
- **An action panel** on `Ctrl+K`: every action a row has, reachable from the
  keyboard, which is what F2 asks for.
- **Inter bundled into the binary** rather than the system font, so the interface
  is the same typeface on Windows and on Hyprland.
- **An icon set**, twelve glyphs drawn for this project on a 16-and-20 px grid
  and compiled into the binary, so every row has something in its icon square
  even when the shell has no icon to give. They take their colour from the
  theme's `icon` token.
- **Themes and sheets as data** with live reload for themes: edit the JSON, see
  it on screen, no rebuild. Two in-house themes ship, one dark and one light.
- **The look is the user's (F15).** A *palette* (colours, type family) and a
  *style* (row heights, spacing, type sizes and weights, and a few layout
  variants: the subtitle beside the title, the footer's keys as keycaps, a
  fill-only selection, outlined keycaps, upper-case section headers) are
  chosen apart, both from files, both reloaded live. Three presets ship
  beside the original look, each measured from a screenshot: **Carbon** from
  Raycast, **Graphite** from Asyar and **Ink** from Vicinae.
- **The empty list in sections**: *Favorites*, *Commands*, *Applications* by
  default, or commands and applications together, or commands only; each
  header can be turned off. Typing collapses them into one ranked list.
- **Favourites**, from the action panel or `Ctrl+Shift+F`, in the order you
  put them (`Ctrl+Alt+Up` / `Down`).
- **An Appearance page** in settings for all of the above.
- **Windows 11 chrome**: acrylic backdrop, rounded corners, dark mode, out of
  Alt+Tab and the taskbar, appearing on the monitor holding the cursor, and
  returning focus to exactly the window it took it from.
- **Daemon/client IPC** over a named pipe, so the Hyprland activation pattern
  already works the way F5 will need it.

## What does not work yet

Listed because a README that implies more than exists is worse than no README.

- **The shortcut sheet editor has no icons.** The launcher's rows and the
  settings Quicklinks list do; the sheet editor is still text and buttons.
- **Windows at a higher integrity level cannot be arranged.** A layout applied
  to an elevated window does nothing, and Windows reports no error for it to
  report back.
- **No text expansion.** Snippets are pasted from the launcher. Expanding them
  as you type anywhere is deferred: F11 writes down a way to do it without a
  global keyboard hook, which is the one mechanism this project deliberately
  avoids (see R-13).
- **Browsers and Electron apps report their container.** A sheet keyed on
  `chrome.exe` covers every tab, because the window does not say what is open
  inside it. Key those on the window title instead. Packaged apps *are* resolved
  correctly, by descending past `ApplicationFrameHost`.
- **Two of the six budgets are enforced in CI, not six.** Ranking and installed
  size break the build on a regression. The other four are measured by the
  daemon on a real desktop and recorded below, because a hosted runner has no
  GPU, no display and no idle session to measure them on.
- **Extensions are not sandboxed, and the protocol is not stable.** An
  extension is an ordinary process running with your privileges, exactly like a
  script command: installing one is trusting it. The capability boundary is
  drawn so a sandbox can be added later, but nothing loads WebAssembly yet, and
  F4 is frozen there in favour of daily use (ARCHITECTURE 7.37).
- **Linux is stubs.** `lumen-platform-linux` compiles on every commit and every
  method is `todo!()`. That is deliberate — see I-2. The plan for the port,
  method by method, is in [`docs/F5-LINUX.md`](docs/F5-LINUX.md).

---

## Measurements

Numbers, not adjectives. Reproduce with `cargo bench -p lumen-core --bench search`.

**Ranking cost per keystroke** (the p95 < 8 ms per-frame budget). Ryzen laptop,
release build, median of criterion's samples:

| Index size | Root list | One character typed | Typical query |
| ---------- | --------- | ------------------- | ------------- |
| 500 entries (a normal Start menu) | 73 µs | 90 µs | 53 µs |
| 2 000 entries | 374 µs | 395 µs | 235 µs |
| 5 000 entries (every SDK installed, and F3's file-search cap) | 1.09 ms | 1.06 ms | 558 µs |
| 20 000 entries | 7.03 ms | 7.42 ms | 2.81 ms |

The worst case at 5 000 is 1.1 ms against a budget of 8 ms for the whole frame,
which is where the decision to keep ranking a plain linear pass over prepared
haystacks comes from: there is no index to invalidate and nothing to get subtly
wrong.

The 20 000 row is why F3 capped file search at 5 000 entries: ranking is
linear in the number of candidates, and at 20 000 one list spends the entire
frame budget. Files are no longer ranked here -- since F16 the file index
matches them off the UI thread (see [File search](#file-search)) -- so this
table is now about the application index alone.

### Every budget in section 5, and where each number comes from

Release build, Ryzen 7 7735HS laptop, Radeon 660M, 1920 × 1080 at 125 %,
Windows 11. The daemon times the first frame and cold start itself and logs
them at `info`; `scripts/measure-gesture.ps1` summons it 15 times and reads
them back, and `scripts/measure-idle.ps1` samples memory and CPU with the
window hidden. `scripts/measure-typing.ps1` types into the launcher and reads
gpui's own frame timings. They are how the numbers below were taken, last at
the close of F15.

| Budget | Threshold | Measured | |
| ------ | --------- | -------- | --- |
| Per-frame ranking | p95 < 8 ms | 1.06 ms at 5 000 entries | ✅ |
| Per-frame drawing while typing | p95 < 8 ms | **2.0 ms** (p50 1.5 ms, ~525 frames) | ✅ |
| Gesture → first frame | p50 < 20 ms, p95 < 40 ms | **8 ms** (p95 19 ms, 15 summonses) | ✅ |
| Idle resident memory | < 100 MB | **67–75 MB** working set, window hidden | ✅ |
| Idle CPU | < 0.02 % | **0.002–0.009 %** over 60 s, one to four 15.6 ms ticks | ✅ |
| Cold start to usable | < 400 ms | **357–377 ms** from `main` to summonable | ✅ |
| Installed size | < 40 MB | **15.9 MB**, the whole install | ✅ |

The budgets were tightened once these numbers were in (ARCHITECTURE 7.36). Idle
CPU is near zero because nothing in the daemon runs on a timer: file changes
are pushed, and gpui's frame clock is patched to stop while every window is
hidden (7.35). About 280 ms of cold start is gpui creating its GPU devices and
font collection before any Lumen code runs. The first run at a new install
path also carries the GPU driver's shader compiler, about 40 MB, until the
next login; the scripts warm it first so it does not read as a regression.

What each measurement excludes is stated in
[`timing.rs`](crates/lumen-app/src/timing.rs) rather than left to be assumed.
Gesture-to-first-frame starts when the message arrives, so it misses the few
hundred microseconds before `WM_HOTKEY` and the compositor's present after the
frame is drawn; cold start ends when the daemon can answer a summons, which by
I-3 is before the application index has landed, because root search never waits
on it either.

**The first summons used to cost 101 ms**, past the p95 of the time (80 ms), because the
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
| `lumen reload`          | Re-read the configuration, sheets, scripts, snippets and files |
| `lumen quit`            | Stop the daemon                               |

`LUMEN_HOME` keeps a daemon's files and socket apart from the installed one:
its `config`, `data` and `cache` live under that folder, and it listens on a
socket of its own, so it runs beside your everyday Lumen without reading or
writing anything of it. The capture and test scripts use it; it is how to try
a change without risking your configuration.

### Keys

| Key | Action |
| --- | ------ |
| `↓` / `↑`, `Ctrl+N` / `Ctrl+P` | Move the selection |
| `Enter` | Run the selected item |
| `Esc` | Clear the query; dismiss when already empty |
| `Home` / `End`, `PgUp` / `PgDn` | Jump to the first or last result |
| `←` / `→` | Move the caret |
| `Ctrl+←` / `Ctrl+→` | Move the caret a word at a time |
| `Shift` + any of those, `Shift+Home` / `Shift+End` | Select text |
| `Ctrl+A` | Select the whole query |
| `Delete`, `Backspace` | Delete forward, back, or the selection |
| `Ctrl+W`, `Ctrl+Backspace` / `Ctrl+Delete` | Delete the previous / next word |
| `Ctrl+U` | Clear the query |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy, cut, paste |
| `Ctrl+K` | Show every action for the selected row |
| `Tab` | Type a search for the selected quicklink, or the one whose name or alias is the query |
| `Tab` on a typed path | Complete it to the selected file or folder; a folder's lists inside it |
| `Tab` / `Shift+Tab` in *Translate* | Open the *From* / *To* language list; again, the other one. `↑`/`↓` and `Enter` choose, `Esc` closes |

The query is a real text field: the mouse places the caret and drags a
selection, double-click selects a word, and dead keys, AltGr and input-method
composition all work (`´` then `e` gives `é` on a US-International layout).
In the settings window `Home` and `End` move the caret instead.

---

## Configuring

On first run Lumen writes `%APPDATA%\Lumen\config\config.json`. Save it and
the change applies at once, even with the launcher open: position, size, row
count, theme and file search. Three settings are wired into things created at
startup and need `lumen quit` then `lumen`: `hotkey`, `backdrop` and
`clipboard`. The log says so when one of them is waiting.

```json
{
  "hotkey": { "modifiers": { "alt": true }, "key": "Space", "hold": null },
  "theme": "nocturne",
  "style": "lumen",
  "root": {
    "sections": [
      { "title": "Favorites", "shows": ["favorites"], "header": true },
      { "title": "Commands", "shows": ["commands"], "header": true },
      { "title": "Applications", "shows": ["applications"], "header": true }
    ],
    "results-header": true
  },
  "backdrop": "system-acrylic",
  "window": {
    "width": null,
    "visible-rows": null,
    "horizontal-bias": 0.5,
    "vertical-bias": 0.22,
    "dismiss-on-focus-loss": true
  },
  "clipboard": {
    "enabled": true,
    "retention": { "max-entries": 500, "max-age-days": 30 }
  },
  "files": {
    "enabled": true,
    "in-root": "folders-and-files",
    "scopes": ["~"],
    "exclusions": [],
    "ignore-patterns": [],
    "use-ignore-files": true,
    "include-hidden": false,
    "default-skips-off": []
  },
  "preferred-browser": null
}
```

### Where the launcher appears

`horizontal-bias` and `vertical-bias` place the window in the work area of
whichever monitor holds the cursor. Both are fractions of the free space, so
the window is always fully on screen whatever you set:

| Value | Horizontal | Vertical |
| ----- | ---------- | -------- |
| `0.0` | against the left edge | against the top edge |
| `0.5` | centred | centred |
| `1.0` | against the right edge | against the bottom edge |

The defaults are `0.5` and `0.22` — centred, and above the optical centre,
because 0.5 vertically reads as low: the eye weights the top of the screen.

On an ultrawide monitor the centre of the screen is often nowhere near the
centre of your attention, which is the case this setting exists for. Try
`0.3` horizontally if you work on the left of a wide display.

Values outside 0–1 are clamped rather than obeyed, so a `5` typed where `0.5`
was meant cannot put the launcher somewhere you can neither see nor dismiss.

The shortcut overlay and the settings window ignore both and stay centred. The
overlay documents whatever is underneath it, so it belongs where the eye
already is rather than wherever the launcher has been moved to.

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

A theme -- a *palette* -- sets **colour and typography only**: its family,
any font installed on the system by name (`"font_family": "Segoe UI"`), and
the family's OpenType features (`"font_features": { "cv11": 1 }`), and a
monospace family for text shown as written, such as a file's preview in
*Search Files* (`"font_mono": "Consolas"`; Cascadia Mono, which Windows 11
ships, when unset). A family that is not installed draws in the system's
interface font. The rest of the look is a style's.

A font that is not installed can be dropped into
`%APPDATA%\Lumen\config\fonts\` as `.ttf`, `.otf` or `.ttc`; it is loaded
at startup, so `lumen quit` then `lumen` after adding one. Raycast and
Vicinae both set their interface in Inter, which Lumen bundles: what differs
on screen is the renderer -- a web view's against DirectWrite -- not the
typeface. Letter spacing is a style's, not a palette's: `text.tracking`, in em
(-0.05 to 0.1, base 0), applies to the launcher's text except the keycaps.

### Styles

A style is the launcher's geometry and a few layout variants, in
`%APPDATA%\Lumen\config\styles\`, named by `style` in `config.json` and
chosen apart from the palette. Like a palette it is data, it reloads live, and
everything it does not set comes from the base style, which is Lumen's
original look (`lumen`). The presets pair a style and a palette of the same
name: `carbon` (after Raycast), `graphite` (after Asyar) and `ink` (after
Vicinae).

```json
{
  "schema_version": 1,
  "name": "Graphite",
  "metrics": { "row.height": 41, "search.height": 56, "text.query": 21 },
  "variants": {
    "row.subtitle": "inline",
    "selection": "fill-border",
    "footer.hints": "keycaps",
    "section.case": "upper"
  }
}
```

The action panel (Ctrl+K) is placed by the style too. `actions.anchor` is one
of `bottom-right` (the default), `bottom-left`, `bottom-center`, `top-right`,
`top-left`, `top-center` or `center`, within the space between the search
field and the footer. `actions.offset-x` and `actions.offset-y` (0 to 64,
default 8) move it in from the edges it is anchored to, or right and down from
the centre on a centred axis. `actions.width` is 280 to 720 (default 440), and
`actions.max-rows` 3 to 12 (default 7) is how many actions show before the
rest scroll; its height follows from them.

```json
{
  "schema_version": 1,
  "name": "Top Actions",
  "metrics": {
    "actions.width": 520,
    "actions.offset-x": 0,
    "actions.offset-y": 12,
    "actions.max-rows": 9
  },
  "variants": { "actions.anchor": "top-center" }
}
```

The shipped palettes and styles follow Lumen's releases for as long as you
leave them as they were: Lumen remembers what it wrote, and a file you have
edited is never touched again. To make one your own without that question,
copy it under a new name.

Beyond geometry, a style can put an icon in the search field
(`"search.icon": "search"` or `"logo"`) and Lumen's mark in the footer
(`"footer.icon": "logo"`); a palette can give the footer its own
`surface.footer` and the window's edge its own `border.window`.

A style also sizes the window: `window.width` and `list.height`, the list's
height before it scrolls. `width` and `visible-rows` under `window` in
`config.json` override them for your own screen (560 to 1000 pixels wide, 3
to 20 rows); `null` leaves them to the style.

A style sets numbers from a closed list and picks variants from a closed
list; it cannot describe a layout. Every number has a range, and the relations
between them are checked when the style loads: a row must hold its text, a
corner must fit its row, the query must fit its field. What is out of range is
clamped and what breaks a relation goes back to the base, with a warning in
the log naming the style and the rule. The list of tokens, their ranges and the
variants is in [`ARCHITECTURE.md`](docs/ARCHITECTURE.md), F15 decisions.

### The root list's sections, and favourites

With nothing typed, the root list is the sections in `root.sections`, in
order. Each has a `title`, what it `shows` -- `favorites`, `commands`,
`applications`, or several -- and whether its `header` is drawn. An item is
listed once, in the first section that claims it; a section with nothing in
it is not drawn; what no section shows is not browsed, but is still found by
typing. Typing collapses everything into one ranked list, under a *Results*
header unless `results-header` is `false`. The *Appearance* page in settings
offers four arrangements by name and a checkbox for each header.

Any application or command can be a favourite: **Add to Favorites** in the
action panel, or `Ctrl+Shift+F` on the selected row; `Ctrl+Alt+Up` and
`Ctrl+Alt+Down` move it. Favourites are kept in
`%APPDATA%\Lumen\data\favorites.json`, in your order; one whose application
is uninstalled keeps its place, and comes back there if it is reinstalled.

### Quicklinks

`%APPDATA%\Lumen\config\quicklinks.json`, written with Google and YouTube on
first run. Save it and the list follows; a broken entry is skipped and logged,
and the rest still load. The settings window's *Quicklinks* page edits the
same file.

```json
{
  "schema_version": 1,
  "quicklinks": [
    {
      "name": "Google",
      "link": "https://www.google.com/search?q={query}",
      "alias": "g",
      "fallback": true,
      "open_with": "Brave",
      "keywords": ["search", "web"]
    }
  ]
}
```

| field | meaning |
| ----- | ------- |
| `name` | required, and unique. What the list shows and what usage is remembered by. |
| `link` | required. `http://` or `https://`, with `{query}` where a search goes and `{clipboard}` for what was last copied. Both are URL-encoded. Any other `{...}` is a mistake and the link is refused. |
| `alias` | optional, one word, unique. Typed before a search: `g lofi beats`. Typed alone, it puts the link first. |
| `fallback` | optional. Offer the link for any search, at the foot of the first screen. Needs a `{query}`. |
| `open_with` | optional. A browser's name, or part of one (`zen`). Otherwise `preferred-browser` in `config.json`, otherwise the system default. The forms offer the installed browsers in a dropdown. |
| `keywords` | optional. Matched, never shown. |
| `icon` | optional. Absent, the row shows the site's icon; `"default"` keeps Lumen's link icon. |

**Icons.** The first time a link is listed, Lumen asks its site for an icon --
the page's own `<link rel="icon">`, then `/favicon.ico`, never a third-party
favicon service -- in the background, after the list is already usable. It is
kept in `%LOCALAPPDATA%\Lumen\cache\favicons\` and asked for again after
thirty days. Offline, or for a site with none, the row keeps Lumen's icon and
the site is not asked again for a day. *Refresh icons* on the settings
Quicklinks page forgets them all and asks every site again.

**When something fails.** An action runs after the launcher has closed, so a
failure -- a private window a browser cannot open, a program that will not
start -- is reported in a Windows notification from Lumen's tray icon, with the
reason. The full detail is in the log.

Fields Lumen does not know are kept when it saves the file, and so are entries
it could not read: saving a link from the launcher never loses a hand edit.
While the file does not parse, Lumen refuses to save over it and says so.

`Ctrl+K` on any quicklink row offers *Open in Private Window* -- incognito in
Chrome, Brave and Helium, InPrivate in Edge, a private window in Zen, Firefox
and Opera -- and *Open With...*, which lists the installed browsers. A browser
Lumen does not know the private flag of is refused rather than opened in an
ordinary window; the reason is in the log.

In the settings page `↑`/`↓` choose a link and `Enter` or `Tab` goes into its
form, where `Enter` saves, `Esc` undoes and `Esc` again goes back. Deleting
asks first.

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

Lumen indexes your home folder by default, files and folders both, and shows
the best eight in a *Files* section after the other results. `Enter` opens a
file with its default application and a folder in Explorer; `Ctrl+K` offers to
copy the path. The full design is in
[`docs/FILE-SEARCH.md`](docs/FILE-SEARCH.md).

```json
"files": {
  "enabled": true,
  "in-root": "folders-and-files",
  "scopes": ["~", "D:/Dev"],
  "exclusions": ["~/Videos"],
  "ignore-patterns": ["*.log", "**/build/**"],
  "use-ignore-files": true,
  "include-hidden": false,
  "default-skips-off": [],
  "index-detail": "balanced"
}
```

- **`scopes`** are where to look, in order; `~` is your home folder, written
  as `~` so a config copied to another machine still works. An empty list
  indexes nothing. **`exclusions`** are folders inside them never walked.
- **`in-root`** is `"off"`, `"folders"` or `"folders-and-files"`.
- **`use-ignore-files`** applies `.gitignore` (inside a repository), `.ignore`
  and `.lumenignore`; **`ignore-patterns`** are gitignore-style globs for every
  scope; **`include-hidden`** takes dot-files and the Windows hidden attribute
  too.
- **`index-detail`** is `"complete"`, `"balanced"` (the default) or
  `"compact"`: how much the index keeps on disk beside the names. Every level
  ranks the same. `complete` also keeps a trigram index, so *Search Files*
  usually finds a match inside a word in milliseconds rather than by a
  scan of every name (a fifth of a second at 500 000 entries); it takes about
  150 MB for 500 000 entries, `balanced` about 110 MB, `compact` about
  90 MB. Changing it rebuilds only the word index, in the background and
  never a walk; the daemon applies it at start and whenever the setting
  changes.
- An F3 config still loads: `roots` becomes `scopes`, and `limits` is ignored.
- An internal drive with no scope on it is offered in *Search Files*:
  *Choose folders...* opens the picker on it, *Not now* writes it to
  **`declined-drives`** so it is not offered again.

Not walked into by default: `node_modules`, `target`, `build`, `dist`, `.git`,
`.venv`, `AppData`, the tool caches `.rustup`, `.cargo`, `.npm`, `.nuget`,
`.pnpm-store`, `.m2`, `.conda` and `.docker`, and about fifteen others,
matched by directory name, hidden files on or off. Name
one in **`default-skips-off`** to walk it after all. A scope you add is
always walked, even inside a skipped, excluded or hidden folder (say
`~/AppData/Roaming/Lumen/config` beside `~`), unless it is itself one of the
`exclusions`.

**No ceiling: the index is on disk, and a hot layer is in memory.** Every
scope is indexed to its end. The entries are kept on disk; memory holds a
*hot layer* of up to 8 MB of names -- what you have used, then folders and
the shallowest files -- which keeps the launcher inside its idle-memory
budget with any index. A home folder (tens of thousands of entries) fits
whole, and then nothing below changes. The log says how much of the index
is hot.

**Root trades a little for speed; *Search Files* finds everything.** In root,
an entry outside the hot layer is found by the start of one of its words
(`report` finds `annual-report.md` and `AnnualReport`), not inside a word
(`report` does not find `myreport.txt` there). *Search Files* shows the same
first answer at once, then, saying *Searching...*, adds the matches inside a
word from the rest of the index below it, up to 200 rows.

**One or two letters search what you use.** Typed in root, they find files and
folders you have opened from Lumen and names that short (`go`); three letters
reach everything.

The index is kept on disk and loaded at start, then walked again behind it. A
file created afterwards is found on the next restart, or by running
`lumen reload`, which also rebuilds it.

### Translation

The **Translation** page of the settings window (`lumen settings`) chooses the
service, keeps the DeepL key, and adds, edits and deletes presets, with the
languages in dropdowns. It writes the `translate` section of `config.json`
and leaves the rest of the file as it was; the launcher follows at once. The
same section can be edited by hand. With nothing set, *Translate* uses Google
and translates any language into English.

```json
"translate": {
  "service": "google",
  "presets": [
    { "name": "Spanish → English", "alias": "esen", "from": "es", "to": "en" },
    { "name": "Spanish → Many", "from": "es", "to": ["en", "fr", "pt"] },
    { "name": "To German", "to": "de" }
  ]
}
```

| field | meaning |
| ----- | ------- |
| `service` | `"google"` (default, no key) or `"deepl"`. Never both: text meant for one is not sent to the other when it fails. |
| `deepl-key` | Your DeepL API key. The free plan's keys end in `:fx` and allow 500,000 characters a month. Never logged. |
| `presets[].name` | required, and unique. The row's title in the root list and the pill in the view. |
| `presets[].alias` | optional. Another word the root list finds it by. |
| `presets[].from` | optional. A code (`es`) or a name (`Spanish`); absent or `"auto"` detects it. |
| `presets[].to` | required. One language, or a list of up to four; each gets its own row. |

The languages are the thirty both services translate: Arabic, Bulgarian,
Chinese (`zh`, simplified), Czech, Danish, Dutch, English, Estonian, Finnish,
French, German, Greek, Hungarian, Indonesian, Italian, Japanese, Korean,
Latvian, Lithuanian, Norwegian (`nb`), Polish, Portuguese, Romanian, Russian,
Slovak, Slovenian, Spanish, Swedish, Turkish and Ukrainian. A preset naming
anything else is skipped and logged, and the others still load.

*Translate* on its own opens the languages you used last, whether they came
from a preset or were chosen in the view. The view names the
service that receives your text, before you type any. Nothing is sent until
you pause, and nothing longer than 5,000 characters. The Google service is an
undocumented endpoint and may stop working without notice; if it does, the
row says why, and DeepL is the documented alternative.

Without an internet connection each row says **No internet connection**,
naming the service it could not reach, and the section title says
*Offline*. `Enter` tries again, and so does typing.

### Currency

With nothing set, *Currency* asks ExchangeRate-API's open endpoint, which
needs no key and publishes one table of rates a day. The **Currency** tab of
the settings window chooses another service and keeps its key, in the
`currency` section of `config.json`:

```json
"currency": {
  "service": "exchangerate-api",
  "api-key": "your key"
}
```

| field | meaning |
| ----- | ------- |
| `service` | `"open"` (default, no key), `"exchangerate-api"` or `"unirate"`. Never more than one: a failure is shown, not retried with another. |
| `api-key` | The key for `exchangerate-api` or `unirate`. Never logged, and never shown back in settings. |

The last table, the currency you convert from and your pins are kept in
`currency.json` in the data folder. A table is fetched when a conversion
needs one and the one in hand is due -- at most once a day -- never on a
schedule. Offline, the last table keeps converting and the header says how
old it is; with no table at all, a row says **No internet connection** and
`Enter` tries again. Rates are for reference: they are one publisher's daily
figures, not what a bank or card will charge.

### Snippets

*Create Snippet* opens a form: the snippet on the left, in an editor that
draws Markdown as you write it -- the markers dimmed, the text bold,
italic, underlined (`++u++`), struck (`~~s~~`), code or a link -- with a
toolbar under it; its name, keyword and tags on the right. `Ctrl+B`, `I`,
`U`, `E`, `Ctrl+Shift+X` and `Ctrl+K` format the selection, `Ctrl+Z` undoes,
and `Ctrl+Enter` saves. *Edit*, *Duplicate* and *Delete* are in *Search
Snippets*' action panel (`Ctrl+K`). A Markdown snippet is pasted formatted
where formatting is read (a document, a mail) and as plain text elsewhere;
one written as plain text is pasted exactly as written.

Snippets are kept in `%APPDATA%\Lumen\config\snippets.json`, and
*Search Snippets* reads the file each time it opens:

```json
{
  "schema_version": 2,
  "snippets": [
    {
      "name": "Email signature",
      "text": "Best,\n\nYour Name",
      "keyword": ";sig",
      "tags": ["email"]
    }
  ]
}
```

A snippet that cannot be read is listed under **Could not read**, with the
reason; a file that does not parse says so instead of showing an empty list.

`Enter` pastes into the window the launcher was opened over; `Ctrl+K` offers
**Copy to Clipboard** instead. Opened from the desktop, with no window to paste
into, the snippet is copied. The `snippets` section of `config.json`:

```json
"snippets": {
  "paste": "restore",
  "paste-keys": { "noctty.exe": "ctrl+shift+v" }
}
```

| field | meaning |
| ----- | ------- |
| `paste` | `"restore"` (default) puts back what was on the clipboard as soon as the application has read the snippet, and keeps the snippet out of clipboard history, Lumen's and Windows'. `"keep"` leaves the snippet on the clipboard, like any copy. |
| `paste-keys` | The keystroke that pastes, by program: `"ctrl+v"` (everything not named), `"ctrl+shift+v"` or `"shift+insert"`. For terminals that pass `Ctrl+V` on to the program running in them. WezTerm, Alacritty and mintty are known already; an entry here wins. |

**Placeholders** are filled in when a snippet is pasted, in Raycast's
syntax:

| placeholder | becomes |
| ----------- | ------- |
| `{clipboard}` | The clipboard's text; `{clipboard offset=1}` the entry before it in clipboard history |
| `{date}`, `{time}`, `{datetime}`, `{day}` | Now, as your regional settings write it; `locale="fr-FR"` for another locale's way |
| `{date format="yyyy-MM-dd"}` | Now, by a Unicode pattern: `EEEE, MMM d, yyyy`, `HH:mm`, `'literal text'` |
| `offset="+1d"` on any date | Another moment: `+`/`-`, a number, and `m`, `h`, `d`, `M` (months) or `y`; several at once, `"+2y +5M"` |
| `{uuid}` | A new random UUID |
| `{snippet name="Signature"}` | Another snippet, one level deep |
| `{calculator expression="12*7"}` | `84` |
| `{argument name="tone" default="happy" options="happy, sad"}` | Asked for before pasting, one at a time in the search field; `options` makes it a choice, `default` makes it optional. The same name is asked once. Three at most |
| `{cursor}` | Where the caret is left after pasting |

After any of them, modifiers in order: `{clipboard | trim | uppercase}`, with
`uppercase`, `lowercase`, `trim`, `percent-encode`, `json-stringify`,
`calculate` (evaluate the value as arithmetic) and `raw`. Anything in braces
that is not a valid placeholder is pasted as written, so code and JSON are
safe.

A window running as administrator cannot be typed into by Lumen -- Windows
discards the keystrokes without a word -- so the snippet is left on the
clipboard, and a notification says to paste it by hand.

---

## Layout

```
crates/
  lumen-core/              index, ranking, actions, theming     no OS, no GPUI
  lumen-platform/          trait Platform and its value types   the OS boundary
  lumen-platform-windows/  the Windows 11 implementation
  lumen-platform-linux/    the Wayland implementation (stubs, always compiles)
  lumen-ui/                the GPUI presentation layer
  lumen-app/               the daemon and its thin client
```

The dependency rule is the point. `lumen-core` has no `#[cfg(windows)]`, no
`#[cfg(unix)]`, no system calls and no GPUI: everything that touches the system
goes through `trait Platform`, and everything that draws lives in `lumen-ui`.
CI can cross-check the Linux crates on every commit, so that a Windows
assumption leaking into the core is caught the same day rather than in a year.
That check is paused while the Linux port is frozen (ARCHITECTURE 7.38).

The views decide nothing either. What the root list shows and in what order is
`lumen_core::root`, and what choosing a row does is `Action::perform`, so both
are tested without a window and would survive a different UI layer (AD-03).
`lumen-ui` draws, and dispatches the few actions that belong to the launcher
itself, like opening settings.

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

The places are from [GeoNames](https://www.geonames.org/) and the forecasts
from [Open-Meteo](https://open-meteo.com/), both under
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
