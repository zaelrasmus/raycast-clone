# Global shortcuts and window management rework (F17)

Branch `f17-shortcuts-windows` off `main` at `f1b9ea0`. Push/PR/merge are the user's.

Research (2026-10-05): Raycast manual `window-management` and
`command-aliases-and-hotkeys`. Engram `lumen/window-management-rework`.
Every command can carry an alias and a global hotkey; window management is the
first heavy consumer.

Out of scope: virtual desktops (undocumented COM), modifier-only / double-tap
hotkeys (Raw Input, later), physical vs key-equivalent recording.

## Phase A: global shortcuts (foundation)

- [x] A1. (`fbe771c`; core 1195 pass, clippy, fmt) Stable slug ids for window commands (`window:left-half`), migrating
  frecency and favourites from the title-based ids.
- [x] A2. (`b208cf6`; RED: 0 samples, exit 1) Measurement first: `WM_HOTKEY` -> action completed latency harness
  (budget proposal in ARCHITECTURE).
- [x] A3. (`6f410a8`; platform 40, windows 82, core 1195, app 2) Platform: dynamic hotkey registry (register/unregister on the
  listener thread), `GestureEvent::Command(slot)`, timers hotkey folded in.
  Both implementations (I-9).
- [x] A4. (`411573c`; measure-hotkey release p50 3 ms, p95 5 ms, 15/15; core 1203, ui 140) Core: `shortcuts.json` (item id -> hotkey, item id -> alias),
  `Root::item_by_id`, headless execution of hotkeyed commands, live reload.
- [x] A5. (`6a4681d`; core 1221, budgets 4, ui 140) One alias mechanism for every item: exact alias leads, prefix boosts;
  replaces per-provider special cases.
- [x] A6. (1/3 `7dd5911` recorder+core API; 2/3 `7339832` page, alias edit,
  hotkey show/clear, OS failures; 3/3 `19a6e5d` record from the page, suspend
  command hotkeys while recording, Overwrite, launcher-chord message) Settings -> Shortcuts page: list, search, hotkey recorder, conflicts
  (incl. the launcher hotkey). ARCHITECTURE note on I-14 and the recorder.
- [x] A7. (`e904c26`; ui 163; measure-hotkey p50 3 / p95 5 ms) Hotkeys on UI-bound commands summon the launcher straight into them.
- [x] A8. (`b8dee70`; core 1228, ui 165; Ctrl+Shift+, arrives as Ctrl+< on US layout, both bound) Configure Command (set alias / hotkey) from the action panel.

## Phase B: window management rework

- [x] B1. (`565d07f`; 36 layouts; platform 53, core 1231) Missing presets: sixths, fourths, maximize height/width, almost
  maximize, reasonable size, move to edge.
- [x] B2. (`94b1b2b`; 41 layouts; arrange_window takes Option<FocusOwner>; ARCH 7.57) Minimize, Minimize on This Display, Restore with remembered history.
- [x] B3. (1/2 `982b334`; 2/2 `429a315` window-management table + page; core 1239, ui 169) Gaps, cycling, Window Management settings page.
- [x] B4. (`db47fa1` model, `7970be8` editor, `9635f71` page; ui 176. Follow-ups: Duplicate for built-ins, Shortcuts page refresh after save) Custom commands: size %/px, offset, 9-point anchor.
- [x] B5. (1/3 `22e3d00` platform apply_layout + current_layout; 2/3 `1067188` core+daemon, ARCH 7.60; 3/3 `14262c4` editor, ~1700 lines in one commit) Custom layouts (apps + placements, other windows: none/minimize/close)
  and Save Current Layout.

## Decisions

- Shortcuts file: `<config_dir>/shortcuts.json`, beside config.json, live
  reloaded. Shape: `{ "commands": { "<item id>": { "hotkey": "Ctrl+Alt+Left",
  "alias": "lh" } } }`. Hotkey as a string (round-trips with `Hotkey`'s Display;
  `FromStr` added in A3) because people edit this file by hand.
- Hotkey budget: WM_HOTKEY -> action done, p50 < 20 ms, p95 < 40 ms, same as
  the summon budget; measured by `scripts/measure-hotkey.ps1` under LUMEN_HOME.
- Legacy window ids frozen in `LEGACY_TITLES` (arrange.rs), not read from the
  live titles, so B1 renames cannot break the migration.

## Evidence log

- A1 `fbe771c`: worker RED (compile), GREEN core 1195; parent froze legacy titles.
- A2 `b208cf6`: measure-hotkey.ps1 fails with 0 samples, as expected before A4.
  Budget counts updated (seven budgets, five by hand).
- A3 `6f410a8`: first worker stalled on ipc.rs scope (answer: add
  `Message::CommandHotkey`, `#[serde(skip)]`); resumed worker finished. ARCH 7.53.
  Follow-ups for A4: a timed-out set request is still applied later (late
  apply after `Failed` replies); Chord prints Ctrl, Alt, Shift, Win order.
- A4 `411573c`: added Platform::foreground_window (both impls, approved mid-task),
  ARCH 7.54, README section. Follow-ups: `lumen reload` does not re-apply
  shortcuts (file watch does); foreground filter for desktop/taskbar checked by
  reading only.
- A5 `6a4681d`: aliases.rs table in RootSearch, timers/quicklinks/translate
  folded (translate aliases now lead: behaviour change); quicklink alias+space and
  Tab kept specialised; badge in the row's accessory column with tag tokens;
  `lumen reload` re-applies shortcuts; ARCH 7.55. Pending live look: alias badge,
  feel of prefix lifting.
- A6 1/3 `7dd5911` (core 1227, ui 148 standalone), 2/3 `7339832` (ui 151).
  ARCH 7.56 recorder exception; CLAUDE.md I-14 pointer. Catalogue shared via
  Launcher::catalogue (Arc); OS status pushed by apply_shortcuts. Pending visual
  check list in the worker report (nav order, rows, caps, red failures, themes).
- A6 3/3 `19a6e5d`: two workers stalled; parent fixed the compile error, a
  narrower worker wired Flow + daemon ShortcutsCommand{Reload,Suspend,Resume}.
  core 1227, ui 160, app 5; release measure-hotkey p50 3 ms, p95 4 ms. No
  countdown (would redraw for itself). Pending visual: recorder outline, caps,
  Overwrite row height, Esc/blur/close restoring, recording an existing hotkey.
  Risk: recording row scrolled out of the virtual list -- blur unverified.
- A7: `Launcher::enter` is the one dispatch for views/forms/questions (Enter and
  hotkey); `show_into` enters before show_window; Summons{Show,Switch,Hide}.
  NOT RUN: measure-gesture.ps1 (it kills the installed Lumen; user to run).
  Follow-up: `entered` goes stale when the daemon replaces a view (e.g. Timers
  key over Clipboard), so that command's hotkey hides instead of switching.
  Live checks: views open with no root flash, quicklink argument focus, forms,
  switch, paste target after Clipboard History by hotkey.
- CLOSE 2026-10-05: release build installed (scripts/install.ps1 -SkipBuild, pid
  17184, HEAD 14262c4). measure-hotkey release p50 3 ms, p95 7 ms. NOT RUN:
  measure-gesture.ps1 (kills the installed Lumen). Manual test guide given to the
  user. Push/PR/merge are the user's.
- fix `d9162a2`: Save Current Layout opens the editor with an empty name (user report); reinstalled.
- MERGED 2026-10-05: fast-forwarded into local `main` at `d9162a2` (user request). Not pushed.
- fix `1cf1862` (branch fix/hotkey-frecency): hotkey presses no longer recorded as use; hotkeyed ids dropped from frecency on shortcuts load (user report: window commands crowding root). Installed; window:minimize gone from frecency.json.
- `b68609a`: Reset Ranking action (panel, indexed rows with history). Both fixes fast-forwarded into local `main`; installed. Not pushed.
