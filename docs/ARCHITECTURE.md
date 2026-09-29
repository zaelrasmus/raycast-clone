# Architecture and execution plan

A native command palette in Rust + GPUI. Windows 11 now, Arch Linux + Hyprland later.

The authoritative source for this document is
[`launcher-rust-gpui-plan-EN.pdf`](launcher-rust-gpui-plan-EN.pdf) (v0.3, 26 August 2026).
What follows is the working version: the decisions, invariants, budgets and risks
in the form they are actually referenced from code, plus a record of where the
implementation deviates from the plan and why.

---

## 1. Vision and non-goals

**What it is.** A native desktop command palette written entirely in Rust, with
its interface built on GPUI. The declared priority is quality: imperceptible
latency, a near-zero idle footprint, and an aesthetic on par with Raycast.

**What it is not.**

- Not a commercial product. No market pressure, no launch date, nobody to beat.
- Not aiming for compatibility with the Raycast ecosystem, its store or its API (AD-10).
- Not attempting to support every Linux desktop. The target is Hyprland and
  compositors supporting `wlr-layer-shell`. GNOME is explicitly out of scope.
- Not a project for accumulating features. Every UI primitive added must be
  rendered well on two platforms, forever.

**Success criterion.** That it becomes the application the author opens fifty
times a day for a year without it ever once feeling slow, ugly or fragile.

### Why native, rather than Tauri

Open latency is not the differentiator — Tauri already reaches ~12 ms and there
is no perceptible gain left there. The differentiator is the **idle** column. A
launcher spends 99.9 % of its time hidden, and 435 MB with 3.2 % CPU permanently
is the price of the WebView. That, not open speed, is what justifies going native.

| Metric (July 2026 reference, M4 Max) | Asyar (Tauri) | Raycast | Target here |
| ------------------------------------ | ------------- | ------- | ----------- |
| Hotkey → window visible (p50)        | 12.0 ms       | 21.7 ms | < 20 ms     |
| Cold start to usable                 | 572 ms        | 888 ms  | < 400 ms    |
| Idle RAM                             | 435.6 MB      | 272.6 MB| < 100 MB    |
| Idle CPU (30 s)                      | 3.20 %        | 0.04 %  | < 0.02 %    |
| On-disk size                         | 64 MB         | 209 MB  | < 40 MB     |

---

## 2. Licensing

Lumen is **MIT OR Apache-2.0**.

This was decided before the first commit (AD-11), because deciding later means
relicensing, and because the choice governs which third-party code may be read
with intent to reuse.

Consequences that bind day to day:

- **Vicinae** (C++23/Qt, GPL-3.0) and **Asyar** (Tauri, GPL-3.0 core) may be
  *read* to understand architecture, protocol shape and manifest structure —
  the highest-value activity available — but their code must never be copied
  into this repository. That would force Lumen to GPL-3.0.
- Asyar's SDK and extension tooling are MIT and are reusable with attribution.
- GPUI itself is Apache-2.0 and published on crates.io, so the UI layer
  introduces no copyleft obligation.

---

## 3. Architecture decisions

| # | Decision | Reason |
| - | -------- | ------ |
| AD-01 | Rust throughout, GPUI as the interface layer | GPUI renders on the GPU and draws all its own controls, guaranteeing visual parity across platforms. Accepted cost: pre-1.0, breaking changes between versions, sparse documentation. The practical reference is Zed's source. |
| AD-02 | Resident daemon plus a client that toggles visibility over IPC | Solves startup (the window is created once and merely hidden) and Wayland (no global shortcut API exists, so the compositor runs the binary and a second execution toggles the instance). |
| AD-03 | Four layers: `core`, `platform`, `ui`, `app` | `core` holds index, matching, providers and actions with no knowledge of the OS or of GPUI. `ui` depends on GPUI. `platform` defines the system boundary. If GPUI proves unworkable, the presentation layer is replaced without losing the project. |
| AD-04 | A single `trait Platform` as the OS boundary | A deliberately small surface. Any need to cross it with something else signals coupling and must be fixed, not accommodated by widening the trait. |
| AD-05 | The theme is a data file, not code, and reloads live | Aesthetics are iterated hundreds of times. Recompiling per iteration kills the design process. The highest-return afternoon of work in the project. |
| AD-06 | The type contract is defined once and is transport-independent | UI tree and event types are Rust structs with `serde`; WIT and JSON Schema are *derived*. The contract is the durable artefact, the transport is reversible. |
| AD-07 | A closed, small UI vocabulary | Extensions never draw pixels; they emit a declarative tree over fixed primitives. Consistency by construction, not by third-party discipline. |
| AD-08 | Static manifest indexed at install time | With zero extension processes running, commands appear because they are in the local index. |
| AD-09 | Every extension capability goes through the host RPC | No real sandbox at first, but the boundary is drawn, so adding one later changes the runtime rather than redesigning the system. |
| AD-10 | No compatibility with Raycast's API or store | Avoids reimplementing their reconciler and inheriting trademark/ToS risk. Their API *shape* is adopted so porting is mechanical. |
| AD-11 | Licence chosen before the first commit | See section 2. |
| AD-12 | Themes are pure data, no code, no runtime | Zed's model. Semantic tokens only; a token that names a component turns every new component into a break for every existing theme. |
| AD-13 | Themes control colour and typography, never geometry | If a third-party theme can alter geometry, the calibrated layout dies the first day someone publishes one. This is the decision that protects what motivates the project. |
| AD-14 | Extensibility in four tiers, not one mechanism | T0 themes, T1 script commands, T2 WebAssembly view extensions, T3 optional JS runtime. Each has a radically different effort-to-value ratio. |
| AD-15 | Keyboard handling at two levels with a sharp boundary | Outside the window is bespoke platform code behind `trait Platform`; inside is GPUI's action and keymap system. GPUI resolves keystrokes against the focus tree, which requires the window to already hold focus — so it cannot cover global invocation. |
| AD-16 | The press-and-hold gesture is not a shortcut | Accelerator APIs require modifiers plus a non-modifier key and report only the press. A lone held modifier with measured duration and a release event is a gesture over keyboard state, not an accelerator. |

---

## 4. Invariants

Reproduced verbatim in [`../CLAUDE.md`](../CLAUDE.md), which is the copy that
gets read during development.

---

## 5. Performance budgets

Reproduced in [`../CLAUDE.md`](../CLAUDE.md). Current measurement status is in
the README. The targets were first set against the table above and tightened
once Lumen was measured well inside them (7.36).

---

## 6. Phases

| Phase | Goal | Status |
| ----- | ---- | ------ |
| **F0** | Platform spike on Windows: transparent undecorated window, backdrop, monitor placement, global gesture, focus capture and restore | **Done** |
| **F1** | The minimum usable launcher: daemon + client, application index, icons, fuzzy matching with frecency, benchmark harness | **Mostly done** — see README for what is outstanding |
| **F2** | Design system and aesthetics: token file with live reload, theme tier T0, bundled typeface, icon set, action panel, appearance animation | **Done.** Themes, live reload, the fade, the bundled typeface, the icon set and the action panel are all in; the appearance animation omits the scale, deliberately (7.5) |
| **F3** | Internal providers: calculator, clipboard history, snippets, window management, file search. Tier T1 script commands. Context-aware shortcut overlay | **Done.** Overlay, settings editor, calculator, snippets, clipboard history, window management, file search and tier T1, behind the `trait Provider` seam |
| **F4** | View extensions, tiers T2 and T3 | **Frozen after step 3.** The UI vocabulary and event types exist once, in `lumen_core::view` (AD-06, I-13); the GPUI renderer draws lists and details with virtualisation; extensions run as subprocesses speaking JSON over stdio, and two (World Clock, Characters) were written against it before anything was called stable. No WebAssembly runtime (T2) and no JS runtime (T3): see 7.37 |
| **F5** | Port to Linux and Wayland | **Frozen.** Stubbed and cross-checked in CI (I-2). The work, method by method, is in [`F5-LINUX.md`](F5-LINUX.md) |
| **F6** | Quicklinks and web search: saved links opened in a chosen browser, with a `{query}` placeholder that makes "search Google" one quicklink among others | **Done.** Step 1: file, placeholders, alias and fallback in root search, browser choice. Step 2: an argument pill on `Tab`, a *Create Quicklink* form in the launcher, and a Quicklinks page in settings. Step 3: site icons, and the browser picked from a dropdown |
| **F7** | Translator: a *Translate* command that turns what is typed into another language, online first, offline later | **Done.** Google and DeepL behind `trait Translator`; presets from `config.json` as root rows, edited on a *Translation* settings page; any source and target chosen in the view through *From* and *To* scopes. *Save as Preset* from the view was not built. **Offline translation is frozen.** See F7 decisions |
| **F8** | Timers: countdowns started from the launcher, named presets with aliases, alert sounds, stopwatches, cycles (Pomodoro) and focus (blocking launches through Lumen) -- one concept, not three features | **Done.** Timers, presets, alert sounds and ringing, stopwatches, cycles (Pomodoro, once or repeating) and focus, as one concept. Laps declined. See F8 decisions |

### F4 build order is not optional

1. Hand-write the JSON of a `List` tree with ten items and an action panel.
   Render it in GPUI from a static file. No protocol, no processes.
2. Wrap that behind `render_node(node) -> AnyElement`, with list virtualisation.
3. Only then move the tree across a channel and introduce events.
4. Write the **second** extension before stabilising anything in the protocol.
   The first extension lies: it fits perfectly because the protocol was designed
   while looking at it.

### F6 decisions

The shape is Raycast's, where web search is not a feature of its own: "Search
Google" is a quicklink whose link is `https://www.google.com/search?q={query}`.
One mechanism, not two.

- **A data file, not a database.** `quicklinks.json` beside `snippets.json`,
  reloaded live. SQLite earned its place for the clipboard because of the write
  pattern -- a row per copy, forever (7.14). Quicklinks are a few dozen rows,
  edited by hand or rarely from the UI, which is the snippets case. `sqlx` was
  considered and rejected: it would be a second link to `libsqlite3-sys` beside
  `rusqlite`, and an async runtime in a daemon whose idle budget is 0.02 % CPU.
- **Two placeholders.** `{query}` is what follows the alias (`g lofi beats`) and
  `{clipboard}` is read when the link is opened, never per keystroke. Both are
  percent-encoded. An unknown placeholder rejects the link at load rather than
  opening a URL with braces in it.
- **A fallback is a property of the link.** A quicklink marked `fallback` is
  offered at the bottom of the list for any query, as "Search Google for ...".
  Google ships as the default entry of the example file, so removing it is an
  edit, not a setting.
- **The browser is named, not assumed.** A link's `open_with`, else
  `preferred-browser` in `config.json`, else the system default. Names are
  matched against the display names the platform reports, because the registry
  key is not a name: Zen registers as `Firefox-<hash>` and Helium's executable
  is `chrome.exe`.
- **No network, in step 1.** Favicons and live suggestions both need an HTTP
  client, which is installed size and, for suggestions, a query that root
  search must never wait on (I-3). Step 3 took the first and not the second:
  see below.

Step 2 made quicklinks something you can make and change without the file:

- **An argument is a pill, not a mode.** `Tab` on a link that takes a
  `{query}` -- or on the link whose alias or whole name the query is, whatever
  usage ranked first -- puts the link's name in a pill where the query starts,
  and the list becomes the one row that link makes of what is typed. Escape
  unwinds the text, then the pill, then the launcher. Backspace in an empty
  argument removes the pill: the field reports it (`DeletedPastStart`) rather
  than the launcher reading keys (I-14).
- **The file stays the source of truth.** `quicklinks::store` edits it the way
  a person would, and the watcher reloads it as after a hand edit, so there is
  one path from the file to the list. A write refuses a file that does not
  parse, keeps entries and fields this build skips, finds a link by name
  rather than position, and replaces the file by renaming a sibling over it.
  A name and an alias are unique: one is the link's identity, the other a word
  the query starts with.
- **One form, two hosts.** The launcher's *Create Quicklink* and the settings
  page show the same entity, with its own key context so `Tab`, `Enter` and
  `Esc` mean field, save and back inside it. Settings adds the fallback
  checkbox; the launcher's form is four fields, because deciding a link
  should answer every failed search is a decision made later, beside the
  others. A refusal gets a reserved line under the fields, so it never resizes
  the window (I-10), in a new `text.error` token that older themes inherit
  (I-12).
- **A delete asks first,** in a dialog over the settings window with `Enter`
  and `Esc` bound like any other keys. Every other change can be edited back;
  a deleted link is gone unless its URL is remembered.
- **A link typed exactly leads.** The whole name or the alias puts the link
  first among the matches, below answers; before, usage let `google` open a
  much-used Google Drive. A partial name still competes on score.
- **The launcher's top edge stays put.** Its height follows the results, and
  placing it by its own height moved the search field on every keystroke that
  changed the count. It is now placed by the height of a full list, and only
  its bottom edge moves.
- **GPUI clicks on Enter and Space.** A focused element with a click handler
  also gets a synthetic click when either key comes up. Every control here
  takes its keys from the keymap alone and ignores keyboard clicks: the
  fallback checkbox saved the opposite of what it showed until it did.

Step 3 made the list look like the sites it opens:

- **Icons come from the site, not a service.** A favicon service is one
  request and a clean PNG, and it learns the domain of every link saved. The
  page's `<link rel="icon">` tags are read instead -- scalable first, then the
  nearest size at or above `ICON_RESOLUTION` -- with `/favicon.ico` after
  them. PNG, ICO, JPEG, WebP, GIF and SVG are fitted to one square PNG per
  site.
- **Nothing waits on the network.** The launcher draws what the cache holds;
  after each index, one background pass fetches what is missing, and a
  reindex picks the files up as `IconSource::File`. The renderer and
  `trait Platform` did not change. A failure leaves a marker retried after a
  day, so an offline machine does not ask on every reindex; an icon is
  refetched after thirty days, keeping the old one if that fails.
- **The client costs 3.5 MB, and that was measured.** The blocking half of the
  `reqwest` gpui already depends on, pinned exactly (I-6), made for a pass and
  dropped after it. Being in the dependency tree was not being in the binary:
  the linker had been removing it. Installed size went from 12.0 to 15.5 MB
  against a budget of 40; idle CPU, working set and thread count did not
  move. WinHTTP behind `trait Platform` would have saved most of it, at the
  price of a trait change and a second fetcher for Linux, and was declined.
- **The browser is chosen, not typed.** "Open with" is a dropdown of what the
  platform reports installed, plus the default. A saved name selects the
  browser it would open in, by the same matcher; one that matches nothing is
  shown as not installed and kept, so saving an unrelated field never moves a
  link to another browser. Closed, the dropdown passes Up, Down, Enter and
  Escape through to the form. In the launcher it opens upward: GPUI moves a
  popup to the other side only if it fits there entirely, and the launcher's
  window ends just below the form.
- **A private window is asked for, never approximated.** `Ctrl+K` offers
  *Open in Private Window* and *Open With...* on every link row. The flag is
  known by the browser's program rather than its name (Helium's program is
  `chrome.exe`), and a browser whose flag is not known is refused with an
  error: an ordinary window would record exactly what was asked not to be.
  With no browser named, the default's program is read from
  `UserChoiceLatest\ProgId` or `UserChoice`, because the shell, which
  opens everything else, takes no flags. `trait Platform::open_url` gained
  the `private` argument, in both implementations at once (I-9).
- **A failure is said where the user is.** Actions run after the launcher
  has hidden -- a launch hands the foreground away -- so an error had nowhere
  to go but the log, and from the user's side nothing happened.
  `trait Platform::notify` raises a system notification instead; on Windows
  from the tray icon (`NIF_INFO`), which needs no app registration. The tray
  window is kept in an atomic so any thread can ask.
- **A typed URL is an answer, not a mode.** `providers::web` recognises one
  as it is typed, like the calculator, so nothing is looked up or waited on.
  How sure it must be to lead is the whole design: a scheme, `www.`,
  `localhost` or an IP address leads; a bare domain leads only with a common
  ending, because `readme.md`, `main.rs` and `setup.py` are file names whose
  endings are also countries' domains. Those get their row at the foot of the
  first screen, above the web search fallback. Every typed URL shares one id,
  `web:typed`, so the usage history on disk never holds an address, and a
  private open records no usage at all.
- **Live suggestions are still out.** A suggestion is a request per
  keystroke whose answer root search could never wait for (I-3); it would be a
  view of its own, not rows in the root list.

### F7 decisions

The shape is Raycast's *Translate* command: open it, type, read the
translation, copy it. Translation is a request to a service, so the whole
design is about keeping that request where nothing waits on it. Step 1 is
built; what changed on the way from the plan is said where it happened.

- **A view, not rows in root search.** A translation is a request per
  keystroke, which is exactly why live suggestions stayed out of the root list
  (F6). *Translate* opens a view, and root search does not change (I-3). An
  inline prefix such as `tr hola` was considered and deferred: it is the same
  request in the one list that must never wait.
- **A built-in view source, not an extension.** `ViewSource::Translate` speaks
  the `lumen_core::view` contract in process: it receives `QueryChanged` under
  `Filtering::Extension` and answers with a `View`. The renderer, the action
  panel and the fixed row geometry are the ones extensions already use (I-5,
  I-10), and no second definition of the tree appears (I-13). An extension
  would have needed an HTTP capability added to the host RPC first (I-7), for
  a feature the host can do itself.
- **The decisions are a value; the daemon carries them out.**
  `translate::session::Session` turns events, typing pauses and answers into
  effects -- render, wait, send, copy, remember -- and never blocks, so every
  rule below is a unit test. `lumen-app/src/translate.rs` owns the threads,
  timers and client, as `extensions.rs` owns a process, and reaches the
  launcher through `begin_view` and `show_view` exactly as an extension does.
- **Nothing waits on the network, inside the view either.** A query is sent
  300 ms after typing pauses, one thread per target. Each request carries a
  generation and an answer from an older one is dropped, so a slow reply to
  "hol" never overwrites "hola". While a request is out, the previous
  translation stays in its row, with no actions, so `Enter` cannot copy text
  that is about to change.
- **The client is the one already linked, kept briefly.** The blocking
  `zed-reqwest` that favicons put in the binary (F6, step 3); no new
  dependency and no change to `trait Platform` (I-9). It is built by the first
  request -- that reads the certificate store -- shared by the others, and
  dropped thirty seconds after the last one, so a translation in the morning
  does not leave a connection pool in a daemon that then idles all day.
- **A service behind a trait, chosen by the user.** `trait Translator` takes
  text, an optional source and a target, and returns the translation and the
  detected source. `translate.service` in `config.json` picks one:
  - **Google** (the default), keyless, so the command works on first use.
    Undocumented, so it can change or throttle without notice; a failure is a
    row saying so, with *Try Again*. **The endpoint changed during the
    build.** `translate.googleapis.com/translate_a/single`, which most
    libraries use, answered the blocking client with its "unusual traffic"
    page (429) on the very first request, whatever the headers, while curl
    on the same machine got 200: it judges the TLS client, not the request.
    `clients5.google.com/translate_a/t`, the endpoint Chrome's dictionary
    extension uses, answered the same client normally, and keeps line
    breaks. An ignored test runs against it, for whoever touches the file.
  - **DeepL**, with the user's own key. The host is picked from the key --
    free keys end in `:fx` -- and a refused key, a spent quota and throttling
    each get their own message.

  There is **no automatic fallback** between them: text the user sent to
  DeepL must not go to Google because DeepL was down. The view names the
  service in its empty state, before anything is typed, and in the section
  title while translating.
- **The key lives in `config.json`,** as `translate.deepl-key`.
  `TranslateConfig` implements `Debug` by hand so the key never reaches a log
  line, and requests to Google are logged without their URL, which carries
  the text. The Windows Credential Manager would be better, and would be a
  `trait Platform` change with a Linux counterpart; deferred, not rejected.
- **Languages are chosen once, as presets.** Picking a source and a target
  every time is the slow part of translating, and most people translate
  between the same few languages. A **preset** is a saved direction: a name,
  an optional alias, a source (or `auto`) and one to four targets --
  *Spanish → English*, or *Spanish → English, French*. "Preset" rather than
  "profile", which suggests an account, or "pair", which stops being true at
  the second target.
- **Every preset is a command.** `providers::translate` adds a catalogue row
  per preset, found by its name, its alias, and the codes and names of its
  languages, so `es en` finds it, and ranked by use like anything else.
  Choosing one opens the view on it: the user types, and chooses nothing.
  The plain *Translate* opens the languages used last, from a preset or
  chosen in the view -- remembered for the daemon's lifetime, not written
  to disk.
- **Presets start the languages; they do not lock them.** The first build
  made the preset itself the view's one choice, stepped with `Tab`. Used,
  that was a cage: a language no preset named could not be reached from the
  view at all. So the view has two choices, *From* (detection, or any
  language) and *To* (any language, or a set of several that a preset
  names), and a preset is only where they start. *Switch to …* in the
  action panel still applies a whole preset at once.
- **The choices are scopes, which the vocabulary gained.** A view's events
  were all about rows or the query, and an empty list has no row to carry an
  action, so `List` gained `scopes` -- Raycast's search-bar dropdown, more
  than one of them: an id, a title, options, and the one selected -- and
  `Event` gained `ScopeChanged`, naming the scope and the option. The
  launcher draws each as a pill at the end of the search field, "From
  Spanish", styled like the quicklink argument pill. A click on a pill, or
  `Tab` / `Shift+Tab`, opens its options in the list's place; there `Tab`
  moves to the other scope, the arrows and `Enter` choose and `Esc` closes.
  It is not the action panel, which sizes itself to every row it has: a
  scope has thirty options, so the picker is a virtualised list of the
  panel's rows capped at `layout::view::PICKER_ROWS`. The keys are bound in
  a `Scoped` key context that the search row has only while the view
  declares scopes; GPUI ranks bindings by context depth first, so `Tab`
  still types a quicklink's argument in the root list (I-14). The pill
  changes when the option is taken, and the view's render confirms it. A
  view that declares no scopes is never sent the event, so World Clock and
  Characters did not change. Changing a target keeps the text of rows whose
  language stayed and empties the others, so a French translation never
  sits under a German tag.
- **Presets live in `config.json`,** under `translate.presets`, read
  leniently: `to` takes one language or a list, a language is a code or a
  name, and a preset with no name, a repeated name, no target or a language
  outside the table is skipped with a log line while the others load.
  Targets past four are dropped. The table is the thirty languages DeepL
  translates, all of which Google does too, so changing `service` never
  turns a working preset into a skipped one. With none usable, one is
  implied, *Any Language → English*. A changed preset re-indexes the root
  list. *Save as Preset* from the view was not built: the settings page
  covers it, and the view's own choices now cover the one-off case.
- **A settings page, written through a store.** The *Translation* tab has the
  Quicklinks page's shape: the service and the presets in a list, the
  selected one's form beside it, a dialog before a delete. Languages are
  dropdowns of the table, so the page cannot write a preset the loader would
  skip. `translate::store` edits `config.json` as a document -- only the
  `translate` section changes, a file that does not parse is refused, and
  the write is a rename -- and the existing watcher does the rest. That made
  `serde_json`'s `preserve_order` a requirement rather than an accident:
  without it every save re-sorted the whole file alphabetically. The full
  build already had it through gpui; `lumen-core` now asks for it. The DeepL
  key is never shown back: the field starts empty, names the last four
  characters of the saved one, and leaving it empty keeps it.
- **One row per target.** The translation is the row's title, drawn on one
  line, and the target language a tag. The section title is always present
  -- "From Spanish", "Detecting language…", "Detected Spanish" -- so no
  header appears or vanishes as answers arrive, and a preset with three
  targets has three rows from the first keystroke (I-10). The launcher gained
  one general fix for this: a re-render keeps the selection on the row it was
  on, by the view's own id. Before, every render put it back on the first
  row, and rows filling in one by one would have pulled it away from the
  user each time.
- **The actions follow the row.** `Enter` copies the translation, line breaks
  kept, and closes the launcher; the panel adds *Copy Original*, *Copy All
  Translations* (one per line) and *Switch to …* for each other preset. A
  failed row offers *Try Again*, first, so `Enter` retries. **No connection
  is said as such.** The raw error for a machine without internet is
  "error sending request", which tells nobody to check their Wi-Fi. So
  `translate::failure` sorts errors: a connection that never opened --
  refused, or a name that did not resolve -- is `Failure::Offline`, drawn as
  *No internet connection* with a caution tag, and the section title says
  *Offline* when every row is. A timeout is not offline: the service was
  slow, and sending the user to check a working network is worse than
  saying so. Nothing retries on a timer; typing or `Enter` does, which keeps
  a daemon without a network from polling one. Two planned actions were
  dropped: *Swap
  Languages*, because a view cannot set the text of the search field and
  swapping without the text is half a swap; and opening a long translation
  as a `Detail`, because `Esc` in a view clears the query rather than
  stepping back from a detail, so it would have thrown the text away. Long
  translations are truncated on screen and copied whole. *Paste to Active
  App* is left out: nothing in Lumen pastes into another application yet.
- **Offline is frozen.** Local neural translation exists (OPUS-MT models
  run through `candle`, or Mozilla's Bergamot, which Firefox uses offline),
  but a model is 15 to 300 MB per language pair against an installed budget
  of 40 MB. If it is ever taken up, it is a third `Translator`, whose models
  are downloaded per pair on request, loaded when the view opens and freed
  when it closes; nothing above changes for it. It is not planned.
- **Measured before it was built, and one number moved.** The baseline was
  built before the first line of the feature. Idle, window hidden, never
  opened: CPU 0.004 % before and 0.000 % after, working set 72.4 MB and
  71.8 MB, private 86.1 MB and 85.0 MB, 31 threads both times -- unchanged.
  The binary did not stay the same size, as the plan expected: 15,537,664
  bytes to 15,874,048, **+336 KB**. The client was already linked, but only
  its GET path was; POST, request headers and JSON bodies, plus the feature's
  own code, were not. That is 0.8 % of the 40 MB budget, and accepted. Not
  measured: idle *after* using the translator, which needs the view driven
  by hand, and is what the thirty-second release of the client exists for.

### F8 decisions

Raycast has Timers, Pomodoro and Focus as three things, and that split is an
accident of its store: the first two are extensions by different authors. To
the person using them they are one intention -- *time this* -- with more or
less attached. So Lumen has **one concept, a timer**, and every other feature
is an optional field on it.

| Field | Empty | Filled |
| ----- | ----- | ------ |
| Duration | counts up: a stopwatch | counts down |
| Name | "25 min timer" | the row's title and the notification's |
| Cycles and breaks | one run | work and breaks alternate: Pomodoro |
| Blocked apps and sites | nothing | Lumen refuses to launch them: focus |

"Pomodoro" and "Deep work" are then **presets** -- saved timers with a name and
an alias -- not modes. `timer`, `focus` and `pomodoro` all find the same place.

- **Nothing ticks.** The budget is 0.02 % CPU idle, and a timer that wakes the
  daemon every second to count is the obvious way to break it. The daemon
  sleeps once, until the nearest deadline, and re-plans when anything changes.
  The only per-second work is redrawing the *Timers* view, and only while it
  is on screen. This was measured before the first line: idle on `main`, window
  hidden, CPU 0.000 %, working set 71.4 MB, private 87.3 MB, 31 threads, binary
  16,057,856 bytes. `lumen timer 25m` exists partly so the same script can
  measure a daemon with timers running.
- **A deadline, not a counter.** A running timer stores the wall-clock moment
  it ends, in milliseconds since the epoch; a paused one stores what is left.
  So a timer survives the daemon restarting and the machine sleeping, and a
  timer that ended while nothing was running is reported when the daemon comes
  back. The state is `timers.json` in the data folder, written by rename.
- **The decisions are a value, the daemon carries them out** -- F7's shape.
  `timers::Clock` takes `now` as an argument and never reads a clock, so
  starting, pausing, expiring and the view's rows are all unit tests.
  `lumen-app/src/timers.rs` owns the one sleep, the notification and the file.
- **A duration is an answer.** `25m`, `1h30`, `90s`, `1:30` or `timer 10m` is
  recognised as it is typed, like the calculator, and leads with *Start 25 min
  timer*. Text after the duration names it: `25m write chapter 3`. A unit is
  required -- `25` alone is a number, and the calculator's.
- **One view, `ViewSource::Timers`,** in the existing vocabulary: rows with the
  time left as a text accessory and *Paused* as a tag. No new primitive.
- **Presets live in `config.json`,** under `timers.presets`, read leniently
  like translate presets, edited on a *Timers* settings page through a store
  that changes only that section. Each is a root row found by name and alias.
  The running state is not configuration and stays in the data folder.
- **Sound is the system's.** Windows ships `Alarm01-10.wav` and
  `Ring01-10.wav`; `PlaySoundW` plays one once or in a loop. No crate, no
  bundled audio, no installed size. `trait Platform` gains `play_alert` and
  `stop_alert`, with the Linux side a `todo!()` like the rest of F5 (I-2, I-9).
  *Ring continuously* loops until *Dismiss Ringing Timer*.
- **Focus blocks Lumen, not the machine.** While a focus timer runs, launching
  a listed app, or opening a listed domain from a quicklink, a web search or a
  typed URL, is refused with the timer's name and time left, and *Open anyway*
  as a second action, which is counted. The app can still be opened outside
  Lumen: a nudge, not a wall. That needs no platform hook, no admin rights and
  no browser extension, and it is a check in `core` before `perform`.
- **The slices,** one acceptance criterion each:
  1. Type `25m`, press Enter, get a notification when it ends; see, pause,
     resume and stop it in *Timers*.
  2. Named presets with aliases, and the *Timers* settings page.
  3. Alert sounds, *Ring continuously*, *Dismiss Ringing Timer*, a preview.
  4. Stopwatches.
  5. Cycles, and the Pomodoro preset.
  6. Focus.

  Running timers at the top of the root list, with the time left in the row,
  wait until slice 1 has been lived with: it is the one part that changes
  `RootSearch`, and the view may turn out to be enough.

Slice 1 is built. What the building settled:

- **One sleep, re-planned, never cancelled.** GPUI's timer is a one-shot
  WinRT `ThreadPoolTimer`, which costs nothing while it waits. Every change
  bumps a plan number and sleeps afresh; the superseded sleep still wakes,
  once, and does nothing. Cancelling would need a handle per sleep for no
  saving anyone could measure.
- **A summons catches up.** The sleep is relative time and a deadline is wall
  time, so a machine that sleeps through a deadline may wake the daemon late.
  Every *show* and *toggle* reports what has ended before the launcher draws,
  so the view never shows a timer that is over, and a notification is at
  worst late, never lost. A timer that ended while the daemon was not running
  is reported when it starts.
- **The view redraws when a face turns over.** A face reads whole seconds
  rounded up, so it changes when a timer's remaining milliseconds cross a
  multiple of a thousand -- not on the wall clock's second, since a timer
  starts whenever it was started. `timers::view::next_tick` asks for exactly
  that moment, plus 5 ms so the redraw lands after the change, not before it.
  With nothing running, an open view wakes for nothing either.
- **Ids are never reused,** so an action from a view that has not re-rendered
  yet cannot land on a newer timer; one on a timer that has already ended
  does nothing.
- **Silence on start, a word on stop.** Starting closes the launcher, and the
  *Timers* row is where to look; a notification per start would be noise.
  *Stop Running Timer* runs with the launcher gone, so it names the timer it
  stopped, or says nothing was running.
- **`lumen timer 25m tea`** sends the same command over the socket, and says
  so when no daemon is listening rather than starting one that would not
  receive it. It is how `measure-idle.ps1 -Timers 25m,1h,90m` starts timers.
- **Measured, and it held.** Idle with the window hidden, on `main` and then
  with slice 1 -- the second with three timers running (25, 60 and 90 min,
  checked in `timers.json` afterwards, because the first attempt passed the
  list as one string and measured no timers at all):

  | | `main` | slice 1 | slice 1, 3 timers |
  | - | - | - | - |
  | CPU | 0.000 % | 0.000 % | 0.000 % |
  | Working set | 71.4 MB | 71.6 MB | 71.7 MB |
  | Private | 87.3 MB | 85.8 MB | 87.7 MB |
  | Threads | 31 | 31 | 31 |

  The binary went from 16,057,856 to 16,164,352 bytes, **+104 KB**.

Slice 2 is built:

- **Two places, on purpose.** A preset -- a name, an optional one-word
  alias and a duration -- is a root row, found by either and ranked by use,
  and starts a timer under its name. The quick durations (2, 5, 10 and 30
  minutes unless changed) are rows only inside *Timers*, under *Quick
  Start*, below *Presets*: common lengths are one `25m` away in the root list
  already, and four more rows there would be four rows in every search for
  "timer". An empty `quick-start` list stays empty; a missing one is the
  defaults.
- **Starting from the view keeps it open.** The new timer appears under
  *Running* at once, which is the confirmation; starting from the root list
  closes the launcher, as every root action does.
- **Typed in the launcher's grammar.** A duration in `config.json` or on the
  page is `25m`, `1h30` or `1:30`, read by the same parser, and must be a
  duration alone -- `25m tea` is refused rather than half-read.
- **One file-editing module, not two.** The translate store's document
  handling moved to `lumen_core::section` before the timers store was
  written, so both pages follow the same rules by construction. The Timers
  page shares the Translation page's key contexts (`PresetList`, and
  `PresetForm`, renamed from `TranslateForm`), so moving through presets is
  one set of keys.
- **Presets are resolved once per change,** not per redraw: the open view
  redraws every second, and resolving logs every preset it skips.
- **An alias is a keyword,** as for translate presets. Typing it finds the
  preset through the matcher; it does not lead the list the way a
  quicklink's alias does. Whether it should is something to learn from use.

Slice 3 is built:

- **The system's sounds, through `trait Platform`.** `alert_sounds`,
  `play_alert` and `stop_alert` joined the trait with their Linux stubs in
  the same commit (I-9). On Windows they list and play `%SystemRoot%\Media`'s
  `Alarm*` and `Ring*` files with `PlaySoundW`, which is asynchronous, loops
  on request, and plays one sound at a time -- so "one alert however many
  timers are ringing" is the system's behaviour rather than code. A name is
  never a path: anything with a separator or a dot is refused before the
  file system is asked.
- **Ringing is a state, not a sound.** With `"ring": "until-dismissed"` an
  ended timer stays in the clock as *Ringing* -- first in the view, `Enter`
  dismisses it -- and the loop stops when the last ringing timer is
  dismissed, stopped or restarted. Several ending together are one alert.
  *Dismiss Ringing Timer* in the root list silences them all, and says so
  when nothing was ringing. A ringing timer is not saved across a restart:
  it was announced when it ended, and an alarm that resumes at the next
  login is an alarm about nothing.
- **Choosing is hearing.** *Preview Alert Sounds* is a view: `Enter` plays a
  sound once, *Use as Alert Sound* writes it, and the *In use* tag moves at
  once. The settings page's *Alert sound* dropdown plays what is chosen.
  A preview replaces a ringing loop -- one sound at a time -- so leaving the
  view puts the loop back. `"alert-sound": null` is silence: the
  notification alone. The default is `Alarm01`.
- **Ring is a dropdown, Once or Until dismissed,** not a checkbox: the only
  checkbox is built into the quicklink form, and two named choices read as
  well as a tick.
- **Measured again,** because `winmm.dll` is a new library in the process.
  With three timers running, four runs: 0.013 %, then 0.000 % three times
  (this build twice, slice 1 once, back to back). The first was straight
  after a four-minute release build and did not reproduce. Working set
  71.8-72.4 MB, 31 threads. The binary went from 16,315,904 to 16,381,952
  bytes, **+64 KB**.

Slice 4 is built:

- **A stopwatch is a timer with no duration,** kept in the same clock and
  the same `timers.json`, with ids from the same sequence. Running, it
  stores when it last started and what it had counted before; paused, what
  it has counted. It has no deadline, so it never wakes the daemon, and it
  keeps counting across a restart because the count is wall-clock time.
- **Started like a timer.** *Start Stopwatch* is a root row, and
  `stopwatch standup call` is an answer that starts one under that name --
  the same shape as `25m name`. It shows under *Stopwatches* in *Timers*,
  counting up, with Pause or Resume on `Enter`, and Reset and Stop in the
  panel. *Stop Running Timer* stays about timers.
- **The face rounds down,** the other way from a countdown's: a stopwatch
  started half a second ago has counted nothing yet. The open view wakes
  when a running stopwatch reaches its next whole second, and not at all
  for a paused one.
- **No laps.** A lap is a list that grows, a row has one line, and nobody
  has asked for them.

Slice 5 is built:

- **Pomodoro is a plan on a timer, not a mode.** A timer may carry a
  `Cycle`: work, break, rounds, an optional long break. Each phase is an
  ordinary countdown, so pause, restart, the one sleep and persistence did
  not change. When a phase's deadline passes, `Clock::expire` moves the
  timer to the next phase instead of ending it -- `Ending::Next` beside
  `Ending::Done` -- with a notification (*Take a break*, *Back to work*,
  *Long break*) and the alert sound once.
- **One set, then done.** `rounds` work phases with a break between, then
  the long break, then the timer ends like any other, ringing if asked
  (*Set complete*). A cycle that went round forever would still be going at
  midnight.
- **Counted from the deadline, not the wake.** The next phase ends one phase
  length after the last one ended, so a late wake does not stretch the plan.
  A machine that slept through several phases catches up through all of
  them and says once where the cycle is now.
- **Skip is the one new action**: end this phase now, start the next -- a
  break not needed, work done early. Skipping the last phase stops the set.
- **Built in, and a preset field.** *Start Pomodoro* is always in the root
  list: 4 × 25 min, 5 min breaks, then 15 min. A preset becomes a cycle by
  having a `short-break` (with `long-break` and `rounds`, 1-12, default 4),
  on the settings page as three optional fields. A break, a long break or
  rounds without a break is refused rather than guessed at.
- **Verified end to end,** not only in tests: a 3-second, two-round cycle in
  `timers.json` went work, break, work, done under the real daemon.
- **Repeating is a choice, made three ways.** A plan may `repeat`: after
  the long break -- or after an ordinary break, when there is no long one --
  the set starts again at work 1 of N, announced as *New set*, until
  stopped. Off by default, as one set then done is still the safer thing
  to leave running. On per preset (*After the set*: End or Start again, in
  settings), per start (`Ctrl+K` on *Start Pomodoro*, or on any cycling
  preset, offers the other ending), and while it runs (*Repeat the Set* /
  *End After This Set* on its row in *Timers*). A `timers.json` written
  before the field existed reads as not repeating.
- **Laps stay out.** Asked about and declined: they are the one piece that
  would need new interface, for a use -- timing splits -- that none of the
  intended ones have.

Slice 6 is built, and with it F8:

- **One list, a switch per timer** -- chosen over lists per preset. The
  *Blocked apps* and *Blocked sites* are one pair of lists in `config.json`
  (`blocked-apps`, `blocked-sites`), on the settings page's General row. A
  timer has focus on or off: `focus 45m write chapter 3` in the root list,
  *Start Focus Session* (25 min), *Start with Focus* in `Ctrl+K` on a typed
  duration or a preset, *Focus* on a preset in settings, or *Turn Focus
  On/Off* on the running timer. In a cycle it holds during work and lifts
  for breaks; paused, it still holds -- pausing is not a way out.
- **Checked in the launcher, known in advance.** Whether `Enter` would open
  something blocked must not wait on anything (I-3), so the daemon pushes a
  `focus::Guard` into the launcher whenever the timers or the lists change,
  `None` when no focus timer holds, and the check is a lookup. An app is
  matched by the name the root list shows, its editions included (`Discord`
  blocks *Discord PTB*, not *Discordia Notes*); a site by the host of the
  link as it would open, subdomains included, which covers quicklinks, web
  searches and typed URLs alike. A search *for* a blocked site on another
  engine is not blocked: it opens the engine.
- **A refusal is a view, not a dead key.** The launcher stays up and says
  what is blocked, by which timer, and how long is left. `Enter` is *Back to
  Focus*; *Open Anyway* runs what was asked -- the link in the browser it
  would have used -- and is counted on the timer, shown as "opened anyway 2
  times" on its row. Nothing is recorded in the usage history for either.
- **Not a wall, by design.** The same app opened from the Start menu opens.
  No system hook, no admin rights, no browser extension; `trait Platform`
  did not change.
- **Verified with a real daemon:** a focus timer carried over in
  `timers.json` with `Discord` listed armed the launcher's guard at start
  and disarmed it when it ended. The refusal itself is behind `Enter` and
  was not driven from here.
- **Found on the way:** the *After the set* dropdown added with repeat was
  not a Tab stop, did not mark the form edited and did not follow the theme.
  Every dropdown on the page now goes through one list that does all three.

After living with it, three things were changed before F8 merged:

- **A duration in a form is three numbers.** `25m` and `1h30` are right in
  the launcher, where typing is the point, and wrong in a form, where a
  person wants to see which box is the minutes. `DurationInput` is three
  short fields -- hours, minutes, seconds -- used for a preset's duration,
  break and long break. An empty field is zero and a field may overflow
  (`90` minutes). What is saved is still written in the launcher's grammar
  (`duration::written`: `1h30m`), so `config.json` stays readable by hand
  and reads back to the same numbers.
- **A preset shows what matters now.** Name, alias, duration, focus and
  *Cycle*; only with *Cycle* on do break, long break, rounds and *After the
  set* appear. The form is `TimerForm`, its own entity with its own key
  context, so the launcher can host the same one (below).
- **Focus has its own row.** The blocked apps and sites were in *General*
  beside the alert sound, which they have nothing to do with. The list is
  now *General*, *Focus*, then the presets.
- **Presets are made where timers are started.** The launcher hosts the
  same `TimerForm` the settings page does, in place of the list, as it
  hosts the quicklink form: *Create Timer Preset* in the root list, *Edit
  Preset* in `Ctrl+K` on a preset's row, and *Save as Preset* in `Ctrl+K`
  on a typed duration (`25m write chapter 3` fills in the name and the
  length) and on a running timer in *Timers* (its name, length, cycle and
  focus). One action, `OpenTimerForm`, carries the preset to start from and
  the name it replaces. The launcher's `Mode::Form` now holds either form,
  and follows the timer form's height as *Cycle* adds its rows. A save goes
  back to the root list with the preset's name typed, so the confirmation
  is its row.
- **What is running is visible without asking.** Rows at the top of the
  root list were considered and declined: they push applications down,
  appear and vanish as timers end (I-10), and redraw every second while
  the user types. A floating always-on-top window was declined too: a third
  permanent window redrawing every second while shown, against the idle
  budget, for what Raycast does with the macOS menu bar. Instead:
  - **The footer.** In the root list, where the result count was, one line:
    what is ringing, else the running timer ending soonest, else a paused
    one, else a stopwatch -- `12:03 · Pomodoro · Break  +2` -- truncated
    rather than moving the keys. The daemon pushes a copy of the clock to
    the launcher when it changes, and the launcher redraws when a second
    turns over **only while its window is active**. Hidden, nothing ticks.
  - **A hotkey for the view.** `Alt+Shift+Space` -- the launcher's key,
    shifted -- shows the launcher on *Timers*, and hides it if that is what
    it shows. `timers.hotkey` in `config.json`, `null` for none, read at
    start like the launcher's own key (a change waits for a restart, and is
    reported). `trait Platform::start_gesture_listener` takes the second key
    and reports `GestureEvent::Timers`, in both implementations at once
    (I-9); on Windows it is a second `RegisterHotKey` in the same listener
    thread, and a key that is taken is logged without costing the
    launcher's. `lumen timers` does the same from a terminal, which is what
    a Hyprland bind will run (F5). It is not in the settings window: the
    launcher's own hotkey is not either, and a key-capture field is its own
    piece of work.
  - **Measured, with a false alarm.** Idle with three timers, the first
    readings were 0.011 %, 0.002 % and 0.007 % against slice 1's 0.000 %.
    Bisected against the slice 3 build in one batch: slice 3 read 0.000 %
    and then 0.013 %, this build 0.000 % with timers and 0.002 % / 0.000 %
    without. The difference is the odd 15.6 ms scheduler quantum on a
    machine that is never perfectly idle, not a tick -- a once-a-second
    wake would be sixty of them a minute. All within 0.02 %. Working set
    72.8-73.6 MB, 31 threads. Binary 16,630,272 bytes.
- **Three bugs found in use, each reproduced on screen before the fix:**
  - The footer's timer line replaced the result count while searching. It
    now shows only while the query is empty; a search shows its count.
  - Clicking another row in settings did nothing after an edit: the page
    refused to leave a form with unsaved changes and said so only in the
    footer, which read as a dead mouse. It now asks, in a dialog -- *Keep
    editing*, *Discard*, or *Save* and then go where the click asked.
  - A preset's alias found it, but seventh, below files that happened to
    contain the letters. A timer preset's exact name or alias now leads the
    matches, as a quicklink's does (`RootSearch::replace_timer_presets`).
    That fix only reordered the matcher's top 64, and a two-letter alias
    such as `io` matches dozens of names better than a keyword does, so
    the preset was cut before it could lead -- `zq` worked only because
    it is rare. An exact name or alias is now taken from the index when
    the matcher did not return it; quicklink aliases had the same gap.

---

## 7. Deviations from the plan

Every one of these is a place the implementation knowingly differs from the PDF.
They are listed so the difference is a decision on the record rather than a
discrepancy someone finds later.

### 7.1 Invocation uses `RegisterHotKey`, not Raw Input

*Plan (section 7):* Raw Input as the single event source, deriving both
invocation and the hold gesture from it.

*Here:* `RegisterHotKey` for invocation, Raw Input for the hold gesture, both on
one message-only window with one message loop.

*Why:* Raw Input cannot suppress a keystroke. A launcher bound to `Alt+Space`
that does not suppress leaves the keystroke to reach the focused application,
where `Alt+Space` opens the system menu — the launcher would appear with a
window menu open behind it. What is load-bearing in AD-16 is narrower than "one
mechanism": a lone held modifier does not fit the accelerator model, but
invocation fits it perfectly. Splitting by *concern* rather than by mechanism
keeps the property section 7 was protecting (not two coexisting event loops) and
still never uses `WH_KEYBOARD_LL`.

### 7.2 The application index reads `shell:AppsFolder`

*Plan (section 7):* Start Menu shortcuts on disk, plus packaged applications via
WinRT's `PackageManager`.

*Here:* one enumeration of the `shell:AppsFolder` virtual folder.

*Why:* two sources means two notions of identity, two icon paths, and a
deduplication problem when a packaged application also ships a shortcut. The
shell already merges them, and the AppsFolder is exactly the list behind "All
apps" — which is the operative definition of "100 % of installed applications"
in the F1 acceptance criteria. Each item yields a display name plus a parsing
name that serves as both launch target and icon source.

### 7.3 `DwmExtendFrameIntoClientArea` is required before the backdrop

Not mentioned in the plan, and discovered during F0: setting
`DWMWA_SYSTEMBACKDROP_TYPE` alone is *accepted and silently ignored*. DWM only
composites a system backdrop behind frame it owns, so the frame must first be
extended across the whole client area. Without it the window is merely
see-through — desktop content shows through sharply rather than being composited.

This is precisely the shape of failure R-1 anticipated, which is why all three
rungs of the backdrop ladder (`system-acrylic`, `legacy-acrylic`, `solid`) are
implemented and selectable from configuration rather than chosen once at
compile time.

### 7.4 GPUI's deferred placement must not be triggered

`Window::activate_window()` causes GPUI to apply a placement it stored for a
window created hidden, and that placement lands *after* our `SetWindowPos`,
dragging the launcher to the rectangle it was constructed with. Taking the
foreground is therefore left entirely to the platform layer
(`AttachThreadInput` + `SetForegroundWindow`), and the UI only calls
`window.focus(&handle)` to point GPUI's focus tree at the view.

### 7.5 The appearance animation omits the scale

F2 asks for "fade plus a 0.98-to-1.0 scale over roughly 120 ms". The fade is
implemented; the scale is not, because GPUI 0.2.2 has no transform for `div`
elements, and faking it by animating width or height is exactly what the
overlay animation rules forbid (it forces per-frame layout recalculation).

The fade is driven by GPUI's animation element rather than by measuring elapsed
time inside `render`: a repaint requested from within a render pass is not
guaranteed to arrive, and when it does not, the window is stranded at whatever
opacity the first frame computed.

### 7.6 Geometry tokens live in code, not in the theme file

I-5 says no colour, spacing, radius or type size may appear literally in UI code.
I-11 says a theme may define only colour and type family. Both hold because there
are *two* token sources with different authors: colour and type family come from
the theme file, which anyone may write; geometry comes from `lumen_core::layout`,
which only the project may change. What I-5 forbids — a literal `px(38.)` buried
in a render function — stays forbidden either way.

### 7.7 The window methods take a role rather than gaining overlay twins

F3 says the overlay "is a new platform surface, not a UI feature… It widens
`trait Platform`." It does — but as a `WindowRole` parameter on the four window
methods rather than as `show_overlay` / `hide_overlay` / `attach_overlay` /
`set_overlay_bounds`.

The method count stays where AD-04 wants it, the differences between the two
surfaces stay expressed as data rather than duplicated code, and "two warm
windows, not one" becomes something the boundary states outright instead of a
convention two call sites happen to follow.

### 7.8 Lumen's own sheet is generated from the keymap table, not from the registry

F3 asks that Lumen's entries "are not written by hand: they are generated by
enumerating GPUI's global action registry". The intent — the sheet cannot drift
from the bindings that are installed — is honoured, but the source is the
binding table in `lumen-ui/src/keymap.rs`, which feeds both `cx.bind_keys` and
the generated sheet.

The registry alone is not sufficient: it can report that `lumen::SelectNext` is
bound to `down`, but it has no human-readable description of what the action
does, and deriving one from the type name puts "SelectNext" on screen where the
user needs "Move down". The registry is still consulted, by
`verify_against_registry`, to report any registered action the table has
forgotten.

### 7.9 Providers distinguish a catalogue from an answer

Not in the plan, and found by running it. F3 says only "each one behind a
`trait Provider` in `core`", which leaves open how a provider's results reach
the list.

The first attempt ranked every provider result through the fuzzy matcher, like
applications. That silently discards every calculation: matching compares the
query `12*7` against the result's title `84`, which do not resemble each other
at all, so the row scored nothing and never appeared.

So a provider contributes in one of two ways. A **catalogue** exists independently
of the query -- Lumen's commands, later the snippets of F3 -- joins the
application index and is matched exactly like an application. An **answer** is
computed from the query, already *is* what the user asked for, and is shown
without matching, above the fuzzy guesses.

### 7.10 The settings window is created lazily

I-4 says the window is created once at startup and thereafter only shown and
hidden. That invariant exists to keep *invocation* instant, and it is honoured
for the launcher and the overlay, which are both summoned by a gesture.

Settings is opened deliberately and rarely. Paying idle memory for a third warm
window to save a few hundred milliseconds twice a month is the wrong side of the
budget in section 5, so it is created on first open -- and from then on kept and
only hidden, exactly like the others.

### 7.11 The action panel replaces the list rather than floating over it

F2 asks that every action be reachable from the keyboard; it does not say the
panel has to be an overlay. The first attempt made it one, and it does not
survive contact with the layout.

The launcher window is sized exactly to its content -- search field, rows,
footer -- so there is no spare room for a floating element. And an absolutely
positioned element with only `right` and `bottom` set gives taffy no way to
derive a height: it lays out at zero and is never painted. The element rendered
correctly and was simply invisible, which took a log inside the render function
to establish.

Replacing the list reuses the one region whose height is already known to be
right, and it reads better as an interaction anyway: the panel is a mode,
Escape leaves it, and nothing is hidden behind it.

### 7.12 A hotkey conflict does not stop the daemon

Global hotkeys are first come, first served on Windows. If the configured
combination is taken, Lumen logs which key failed and where to change it, then
keeps running and stays reachable over the socket. Refusing to start would turn
"pick a different key" into "the application is broken", with the explanation
visible only to someone who thought to run it from a terminal.

### 7.13 The clipboard privacy layer skips rather than redacts

The usual shape of this feature is to store the entry and mask it in the UI.
That is the wrong place to draw the line, because the secret is then on disk in
an unencrypted SQLite file and the protection is a rendering decision -- one
bug, one `SELECT`, or one backup away from being no protection at all.

So `lumen_core::clipboard::privacy` decides *before* the write, and a rejected
entry produces no row. There are two halves and only one of them can live in
`core`: the platform half honours the three Windows clipboard formats that mean
not-for-history (`Clipboard Viewer Ignore`,
`ExcludeClipboardContentFromMonitorProcessing`, and a zero in
`CanIncludeInClipboardHistory`), because only the platform can see them; the
`core` half declines anything copied out of a known password manager and
anything matching a credential format -- PEM blocks, AWS and GitHub and Slack
and OpenAI and Anthropic and Google and Stripe keys, JWTs, and URLs with
embedded credentials.

The rejection is logged at `debug` and without the text. A privacy layer that
writes the secret to the log has moved the file, not solved the problem.

The cost is that a false positive loses an entry silently. That is the right
side of the trade, and it is why the false-positive cases are tests rather than
an afterthought.

### 7.14 Clipboard history is an index of its own, not a `Provider`

Everything else in F3 fits the provider contract: a catalogue that joins the
application index and is matched with it. The clipboard does not, for two
reasons that are both about ranking rather than about results.

It changes constantly. Folding it into `candidates` would mean re-preparing
several hundred application haystacks every time the user copies anything,
which is exactly the work the per-frame budget exists to keep out of the way.

And it must not appear in the root list. With an empty query the index is shown
in frecency order, so a hundred catalogue entries with no usage history would
interleave alphabetically with the applications: open the launcher, see what
you copied last Tuesday. The history is something you search, not something you
are shown before you have asked for anything.

So the view holds it as a second index, ranked separately, only when there is a
query, and capped so that a common letter cannot fill the list with pastes. The
provider module survives as the thing that says what a clipboard row looks
like, which is all it was ever really doing.

Refreshing it is the daemon's job and happens when an entry is recorded -- the
only moment anything knows the history changed -- rather than on a timer or
when the window opens, so the snapshot is already correct by the time anyone
searches it and no query runs that nobody asked for.

### 7.15 A layout is applied to the captured handle, never to "the foreground"

Activating a row hides the launcher and hands the foreground back before the
action runs. Any window-management feature that then asks the system which
window is in front is reading the answer to a question it asked too late: the
hand-back is asynchronous from the point of view of everything watching it, and
losing that race means resizing a window the user did not choose.

So `Action::ArrangeWindow` travels back to the daemon like a builtin, and the
daemon applies it to the `FocusOwner` it captured *before* showing the
launcher -- the same handle it is about to restore focus to. It reads that
handle first and hides second, because hiding consumes it.

### 7.16 The tiling arithmetic lives on the boundary, not in `core`

Everything else about what a result *does* lives in `lumen-core`, so the layout
maths looks like it belongs there too. It cannot: `core` depends on
`lumen-platform` and not the other way round, and a layout is a value the
platform has to be handed.

That turns out to be the better place anyway. A half is a half on Windows and
on Hyprland, the arithmetic is the part that is easy to get subtly wrong, and
putting it in `lumen_platform::tiling` means it is tested with no compositor
anywhere near it -- including the cases that actually bite: a work area whose
width divides by neither two nor three, whether the thirds meet without a seam,
and whether a window carried between monitors of different sizes arrives inside
the destination. `core` keeps what is genuinely its own, which is what each
layout is called.

### 7.17 Tiling compensates for the invisible resize border

Since Windows 10 a window's resize border is drawn outside what the user
perceives as its edge. `GetWindowRect` reports the outer rectangle and DWM
composites the inner one, so placing a window at exactly half the work area
leaves a visible strip of desktop down the middle and lets the window overhang
the screen by the same amount -- around seven physical pixels at 100%, more
when scaled.

`DWMWA_EXTENDED_FRAME_BOUNDS` reports what is actually drawn. The difference
between the two rectangles is the padding a target is expanded by. It is four
lines, and it is the whole difference between a tiling feature that looks
deliberate and one that looks approximate.

A window DWM will not describe is placed as asked rather than not placed at
all.

### 7.18 No window is touched from inside a GPUI callback

A subscription callback runs while GPUI holds a mutable borrow of the `App`.
Showing, hiding, moving or resizing a window sends window messages
*synchronously* to our own window procedure, on the same thread; GPUI's
procedure then tries to take the borrow it cannot have, and the message is
dropped.

Nothing reports this. The only sign is a line in the log reading
`RefCell already borrowed`, with no target and no indication of what it was
about, produced by an internal `log_err` on a `try_borrow_mut` several frames
away from the cause. It had been happening on every show and every dismiss
since F1 and was found by reading the log during unrelated work, which is
exactly how long a silent failure survives.

What gets dropped is size and position notification for a window that is being
resized at that moment -- the launcher hugs its content, so it is resized on
almost every keystroke.

So every subscription handler that touches a window now goes through
`off_the_callback`, which spawns the work onto the foreground executor: same
thread, same order, no borrow held. The three handlers are the launcher's
(dismiss, resize, arrange), the overlay's fade-out, and the settings window's
close.

### 7.19 File search walks folders rather than querying the system index

Windows keeps an index of its own, reachable through `ISearchQueryHelper` and
OLE DB. Not using it is a deliberate choice with two reasons, and the second is
the one that settles it.

The first is cost: a COM interface with no safe binding and an OLE DB query
path, for results the user has already told us where to find by configuring
their folders.

The second is I-3. Root search never waits on anything, so a query to the
system index would have to be answered from a cache anyway -- at which point it
is a bounded walk with more moving parts in front of it. A bounded walk of
Desktop, Documents, Downloads, Pictures, Video and Music takes about 220 ms on
a background thread and never touches the frame.

The honest description of what this is, and it is the one in the README: Lumen
finds the file you were working on, not every file on the machine.

### 7.20 The file-search cap is a measurement, not a preference

`files.limits.max-entries` defaults to 5 000, which looks arbitrary and is not.
Ranking is a linear pass over prepared haystacks, and the worst case is a
one-character query where nothing is filtered out early. The benchmark says
1.06 ms at 5 000 entries and 7.42 ms at 20 000 -- and the per-frame budget is
8 ms for *everything*, render included.

So the first implementation, which capped at 20 000, was wrong: it worked,
indexed in 1.3 seconds, and would have spent the whole frame budget on one
list. The benchmark was extended to 20 000 before the number was chosen, which
is what the working rule about writing the measurement first is for. The cost
of raising it is in the README next to the figures rather than buried in a
commit message.

### 7.21 Secondary indexes are searched but never browsed

Clipboard history and file search are both large, both change without the user
touching the launcher, and neither belongs in the root list -- with an empty
query the index is shown in frecency order, so entries with no usage history
sort alphabetically among the applications. Open the launcher, see what you
copied last Tuesday and a folder called `Archive`.

So they are *asides*: separate candidate lists, ranked only when there is a
query, each capped at eight rows so a common letter cannot fill the list with
one kind of thing. Keeping them out of `candidates` also keeps the application
index cheap to hold -- merging would mean re-preparing several hundred
application haystacks every time the user copies something.

### 7.22 The glyph is chosen from the row's kind, in the UI layer

The obvious place to put an icon is on the [`Item`](../crates/lumen-core/src/item.rs),
next to the one an application already carries. It is the wrong place twice
over.

`Item::icon` is an `IconSource` -- a thing the *platform* resolves to pixels,
by extracting it from an executable or looking it up in an icon theme. A
bundled glyph is neither; it is a file compiled into this binary, painted as a
mask in a colour the theme supplies. Putting it in the same field would mean
the platform boundary carrying a value only the UI can interpret.

And it would make every provider responsible for setting one, which is a rule
that gets forgotten exactly once and then ships. `Kind` already says what sort
of row this is, and the mapping from kind to glyph is a presentation decision
that belongs in the UI layer (AD-03). A test asserts every `Kind` resolves to a
bundled asset, so adding a kind without a glyph fails the build rather than
drawing an empty square.

The order the row draws in is: the real icon if it has arrived, the bundled
glyph if there was never going to be one, and *nothing* while extraction is
still in flight -- because showing a glyph and replacing it a frame later is a
flicker, and a row that is about to have its own icon can wait.

### 7.23 The icon set is eight files on one grid, not an icon font

Every glyph is a `20 × 20` viewBox with its drawing inside the inner `16 × 16`,
stroked at 1.5, round caps and joins, `currentColor`. That is F2's "optically
aligned on a 16 and 20 px grid", and the two-unit margin is the point of it: a
shell-extracted application icon is drawn to its own edges, and a stroked glyph
at the same nominal size reads as larger, because its corners are further from
the centre with no shading to soften them.

Horizontal and vertical edges sit on half-units so a 1.5 stroke lands on whole
pixels at 100% scaling rather than straddling two.

A test reads the files back and asserts the viewBox, the stroke width and
`currentColor` on all eight. Those are the three things that make one icon look
like it belongs with the others, and they are exactly the three things an
edit six months from now will get wrong.

### 7.24 I-8 is enforced for two budgets of six, and says so

I-8 reads "The performance budgets below are measured in CI. A regression
breaks the build." Two of the six are, and four cannot be.

**Enforced.** Ranking, by `crates/lumen-core/tests/budgets.rs`, and installed
size, by comparing bytes in the workflow. Both fail the build.

**Not enforced, and why.** Gesture to first frame needs a compositor presenting
to a real display. Idle CPU and idle memory need a session that is genuinely
idle. Cold start needs a GPU. A hosted runner has none of those. A figure taken
there would be a measurement of the runner, not of Lumen, and it would sit in a
table looking exactly like evidence.

So the daemon measures those four itself, on whatever machine it is running,
and logs them at `info`; the README records what they were and on what
hardware. Someone who wants to check them runs the daemon and reads the log,
which is a weaker guarantee than CI and a stronger one than a green tick over a
number nobody can reproduce.

**The ranking gate asserts a shape, not a stopwatch reading.** A shared
runner's clock speed varies with the host, so "under 8 milliseconds" is either
tight enough to fail on a busy afternoon or loose enough to catch nothing. What
actually threatens this budget is somebody making ranking super-linear -- an
inner loop over the index, a sort inside the match, a per-candidate allocation
that grows -- and that is a change of shape, which is measurable anywhere:
quadruple the index and the cost should roughly quadruple. The test asserts
under 8× for 4× the entries (linear is 4, quadratic would be 16), plus a
ceiling fifty times the measured figure to catch a constant-factor
catastrophe.

### 7.25 The first frame is drawn before anyone asks for it

The first frame a GPUI process paints costs far more than every frame after it:
pipelines created, shaders compiled, the glyph atlas filled with the first run
of text. Measured here, the first summons took 101 ms and the second 24 ms
against a p95 budget of 80 ms; from the third onwards it was 8 ms. The budget
was being missed exactly once per launch, which is the once that forms an
impression.

So once the daemon is already summonable it shows the overlay -- which never
activates and never takes focus, which is what makes it usable for this -- far
outside the virtual screen, waits for one frame, and hides it again. GPUI's
renderer state is per process, so the launcher inherits what that frame built.
First summons: 20 ms.

It happens *after* `report_ready` and is not awaited. Cold start is "until the
first search responds", the daemon already responds, and paying 100 ms of
startup to save 100 ms on a keystroke that may not come for an hour is the
wrong trade. Someone quick enough to beat it gets exactly what they would have
got anyway.

### 7.26 An aside keeps three quarters of its score

The cap in 7.21 stops an aside from filling the list. It does not stop one from
taking the *top* of it, and with file search in place that is what happened:
typing `co` returned six files before `Command Prompt`, because
`COMPLETE.png` matches two letters exactly as well as an application does.

A launcher is a launcher. Asides are ranked with a discount -- six eighths of
the score they earned -- which is enough to break a near-tie in favour of the
application and not enough to bury a clear match: `colonialismo` still puts the
file first, because by then it is not close. The discount is applied to the
score rather than by sorting asides below applications, which would bury them
permanently and make file search useless.

The same query, before and after: `co` went from six files and no applications
to `Command Palette`, `Command Prompt`, `Component Services`,
`Computer Management` and `Control Panel`.

### 7.27 The resize event carries a height, not a row count

The launcher hugs its content, so the view tells the daemon when its size
changed. That event carried a *row count*, which was right while there was one
kind of content.

The action panel is not made of the same rows -- a result row is tall enough
for a title and a subtitle, an action row holds one line -- so a panel of two
actions was drawn into a window still sized for eight results, with six rows of
nothing beneath it. Converting one into the other in "equivalent result rows"
would round, and rounding leaves exactly the band of empty space the change is
meant to remove.

So the event carries the height, `Config` grows a `panel_height` beside
`window_height`, and the view answers one question -- how tall am I -- rather
than two.

### 7.28 An extension's view takes over the launcher rather than opening a window

The obvious shape for F4 is a window per extension. It is the wrong one.

An extension's results *are* the launcher, showing somebody else's rows. A
separate surface means a second search field, a second focus owner, a second
thing to dismiss and a second place for the geometry to drift -- and it makes
the transition from "I typed a command" to "I am in its list" a window swap
instead of a change of contents.

So the launcher gains a mode. `Mode::View` replaces the list's region and
nothing else: the search field and the footer stay the launcher's, and the
extension supplies rows. That split is exactly the one AD-07 describes -- an
extension chooses what to show and never how the chrome around it looks -- and
making it structural means it cannot be negotiated later.

Escape unwinds one layer at a time, as it already did: the action panel, then
the view's query, then the view, then the launcher. And a view does not survive
the launcher being hidden. Coming back a day later to somebody else's list,
with no memory of opening it, is the kind of state that makes an interface feel
haunted.

### 7.29 The view renderer uses `list`, not `uniform_list`

The root list is uniform: every row is `ROW_HEIGHT` whether or not it has a
subtitle, which is I-10 and which makes `uniform_list` -- cheaper, and
constant-time to scroll -- the right element.

A view has section headers, and a header is deliberately shorter than a row: it
is a label, not an entry, and giving it a full row's height makes a two-item
section look like a four-item one. That rules `uniform_list` out, so views use
`list`, which virtualises with variable heights.

The consequence is a sizing rule that had to be written out rather than assumed:
the window cannot be `total.min(cap)`, because with mixed heights the cap lands
mid-row -- for the bundled example, two thirds of the way through the last row
it shows. Whole rows are accumulated until the next will not fit. I-10's promise
that a partial row is never left clipped is the same promise either way; only
the arithmetic differs.

### 7.30 Host filtering keeps the extension's order, not the score's

A list defaults to [`Filtering::Host`], and the default is the one the frame
budget can promise: the same matcher as the root list, in the same frame,
waiting on nothing. `Filtering::Extension` exists for a search that hits a
network, and it is the caller's problem that it is slower.

When the host filters, the surviving items keep the order and the sections the
extension gave them rather than being re-sorted by score. The extension chose
that order -- and the sections are *labelled*, so reordering across them would
put rows under headings that no longer describe them. A section that loses
every item loses its heading too, because a heading with nothing under it is a
heading about nothing.

### 7.31 Step 3 is written down but not wired up

`lumen_core::view::Event` exists and the launcher builds a real one when a view's
action is run -- `ActionInvoked { item: Some("pr-395"), action: "copy-url" }` --
and then logs it and drops it, because there is nowhere to send it.

That is deliberate and it is the plan's build order: the tree moves across a
channel only after the renderer is finished, and nothing in the protocol is
stabilised until a *second* extension has been written against it, because the
first one lies -- it fits perfectly because the protocol was designed while
looking at it.

The events are in the contract file rather than in a later commit for a
different reason: AD-06 says the contract is one artefact. Splitting it across
two commits is how it becomes two artefacts that have to be kept in step.

### 7.32 Text input is one component with a real input handler

The F1 search field was a `String` with a drawn caret, and it had the gaps that
implies: no IME, no dead keys, no caret movement, no selection. Changing one
digit of a calculation meant deleting back to it. The settings window had a
second copy of the same field with the same gaps.

Both are now one `TextField` that implements GPUI's `EntityInputHandler` with
its own shaped-text element. Typed text arrives as text from the system, so a
dead key followed by a letter gives one accented letter and an input method
can compose. That also removes the last hand-written keystroke inspection from
the launcher, so I-14 no longer needs an exception. The editing arithmetic
(graphemes, words, selection, UTF-16 offsets for the input method) is in
`lumen_ui::edit`, apart from GPUI, and tested there.

Two key contexts, not one: `Home`/`End` jump through the list in the
launcher's `SearchField` and move the caret in a settings `TextField`.

### 7.33 The configuration reloads live, except three settings

Editing `config.json` used to need a restart, which is exactly the friction
AD-05 removed for themes. The daemon now watches the file and applies position,
size, row count, theme and file search at once, re-placing an open launcher.

`hotkey`, `backdrop` and `clipboard` are the exceptions. Each is wired into
something made once at startup: the registered hotkey, the window's DWM
attributes, the clipboard listener thread. Re-creating those live is possible,
but it is three new code paths for settings that change once. So they are
reported as waiting for a restart, and the daemon keeps holding the values
actually in effect (`Config::merge_live`), so the warning keeps appearing until
the restart happens.

A config file caught half saved fails to parse and the running configuration
is kept. Startup still falls back to defaults on a broken file, as before.

### 7.34 The application index follows installs by watching, not by polling

A full `AppsFolder` enumeration was measured at 600–1000 ms of CPU, so
re-enumerating on a timer or on every summons would break the idle and
gesture budgets. Instead `Platform::watch_applications` waits on kernel change
notifications for the two Start Menu folders and the per-user package key in
the registry, then signals once the changes have been quiet for 1.5 s.

It signals a second time 5 s later. The shell rebuilds `AppsFolder` from those
sources asynchronously, and a new shortcut was measured taking 2.5–4 s to be
listed, so the first re-index usually still sees the old list. The launcher
drops a re-index whose result is identical to the current one, so neither
signal moves the selection unless something actually changed.

### 7.35 gpui is vendored, with one patch that parks its frame clock

With Lumen's own timers gone (the file watchers push since `5506d19`), idle CPU
was measured at 0.04 %, and nearly all of it was one thread inside gpui. Its
Windows backend runs a `VSyncProvider` thread that calls `DwmFlush` at the
refresh rate for the life of the process and invalidates every window, visible
or not: 60 to 144 round trips into dwm.exe a second, to redraw nothing. No
gpui release changes this; 0.2.2 is the latest.

`vendor/gpui` is the published 0.2.2 crate, imported unmodified in its own
commit (minus the examples), with the patch on top as a separate, reviewable
diff. Every changed line is marked `LUMEN PATCH`. The thread now checks whether
any window is visible and, if none is, blocks on an event. The window procedure
sets that event on `WM_WINDOWPOSCHANGED` with `SWP_SHOWWINDOW`, which every way
of showing a window sends, before the window paints. A one-second recheck is
the safety net, not the mechanism: if a wake were ever missed, a visible window
would stall for at most that long instead of for good.

Measured with `scripts/measure-idle.ps1` and `scripts/measure-gesture.ps1` on
the same machine, before and after:

| | Before | After |
| --- | --- | --- |
| Idle CPU, 60 s hidden | 297 ms (0.041 %) | 0 ms (below the 15.6 ms tick) |
| Gesture → first frame | p50 8 ms, p95 20 ms | p50 8 ms, p95 19 ms |

The first frame of a shown window never depended on this thread, because
showing a window paints it. The frames after it do, and by then the thread
is awake.

I-6 still holds. `[workspace.dependencies]` pins `=0.2.2`, and
`[patch.crates-io]` only applies while the vendored crate is that exact
version. Upgrading gpui is now two deliberate steps: re-import the new
release unmodified, then re-apply the marked patch and re-measure. If an
upstream release parks the thread itself, the vendored copy is deleted.

### 7.36 The budgets are tightened to guard what was measured

The first budgets were set against Raycast and a Tauri launcher before any of
Lumen existed, and it now sits far inside them: 8 ms to the first frame against
40, 71 MB against 150, and after 7.35 no measurable idle CPU against 0.2 %. A
budget that loose cannot fail, so it guards nothing. The regression that 7.35
removed measured 0.04 % and would have passed even a 0.05 % budget.

| Budget | Was | Now | Measured |
| --- | --- | --- | --- |
| Gesture → first frame | p50 < 40, p95 < 80 ms | p50 < 20, p95 < 40 ms | p50 8, p95 19–20 ms |
| Idle resident memory | < 150 MB | < 100 MB | 64–72 MB working set |
| Idle CPU | < 0.2 % | < 0.02 % | 0–0.009 % |

The new numbers keep roughly twice the measured value as headroom. The idle CPU
budget is stated as a share of the whole machine over 60 s, the way
`scripts/measure-idle.ps1` measures it. Windows accounts CPU time in 15.6 ms
ticks, so on twelve cores 0.02 % is nine ticks a minute: coarse enough to be
stable, and fine enough to catch a frame clock left running.

Cold start stays at < 400 ms against 340–357 ms measured. About 280 ms of it is
gpui creating the platform before any Lumen code runs: D3D11 and
DirectComposition devices (~130 ms), the DirectWrite system font collection
(~105 ms), `OleInitialize` (~36 ms) and a drag-and-drop helper Lumen never
uses (~17 ms). Cutting into that is deep surgery in the vendored crate, for a
cost paid once per login that is already 2.5 times faster than Raycast's, so
the budget is left where it is rather than tightened to a number only gpui
could meet.

Installed size stays at < 40 MB. It is enforced in CI, where a byte count
cannot be noisy, and there is no pressure on it yet.

### 7.37 Step 3 runs over stdio, not WebAssembly, and F4 stops there

This supersedes 7.31. The event that was built and dropped there is now sent:
`crates/lumen-app/src/extensions.rs` starts an extension's process when one of
its commands is chosen, and `lumen_core::protocol` is the envelope that
carries views one way and events and capability requests the other, as
newline-delimited JSON over stdio. Nothing starts a process to build the index
(AD-08), so root search still never waits on one (I-3).

The plan's T2 is WebAssembly. Step 3 used an ordinary subprocess instead,
because AD-06 makes the transport the reversible part: the durable contract is
`lumen_core::view`, and `protocol.rs` is written to be replaced wholesale. What
would *not* be cheap to change later is the capability boundary, so that was
built now: every capability an extension uses goes back through the host and
`trait Platform` even though a subprocess could reach it directly (I-7, AD-09).
There is no sandbox, and the code says so.

Two extensions were written against it before anything was called stable, as
step 4 asks, and they corrected the protocol three times. F4 is frozen at that
point in favour of daily use; the WebAssembly runtime and T3 are not started.

One consequence was reviewed and kept. `Platform::spawn_extension` returns a
`std::process::Child`, a concrete type rather than a platform-neutral handle.
That reads like a leak through the boundary AD-04 keeps small, but it goes no
further than `lumen-app`: the launcher only ever sees the `ViewSink` it is
handed. A WebAssembly runtime would run in-process under wasmtime and never go
through `spawn_extension` at all. Abstracting the child would add an interface
with one implementation to serve a runtime that does not use it.

---

## 8. Risk register

| # | Risk | Phase | Status |
| - | ---- | ----- | ------ |
| R-1 | Real transparency unworkable; the system backdrop does not show | F0 | **Hit and mitigated.** See 7.3. Three backdrop modes are implemented and configurable. |
| R-2 | Breaking changes in GPUI between versions | all | `gpui` pinned exactly (I-6). F0 assertions re-apply the window styles rather than assuming GPUI's. The vendored patch (7.35) is re-applied and re-measured on every upgrade. |
| R-3 | Insufficient GPUI documentation | all | Mitigated by reading the crate source directly; it ships its `examples/`. |
| R-4 | Windows assumptions leaking into the core | F1–F4 | I-1 and I-2, cross-checked in CI from the first commit. |
| R-5 | GPUI does not implement `wlr-layer-shell` | F5 | Accepted as a planned task with its own estimate. |
| R-6 | The extension protocol becomes coupled to its first runtime | F4 | AD-06, AD-09. |
| R-7 | Uncontrolled growth of the UI primitive vocabulary | F4 | Every new primitive requires explicit justification. |
| R-8 | Licence contamination from inadvertent reuse of GPL code | F0 | Decided (section 2). Study architecture, do not copy implementations. |
| R-9 | Silent performance degradation over months | F1+ | Partially mitigated: ranking cost and installed size fail the build in CI (7.24). The other four budgets are measured by hand with `scripts/measure-*.ps1` against thresholds tight enough to catch a regression (7.36). |
| R-10 | Loss of motivation from deferring the aesthetic work | F2 | Live token reload landed early, in F1 rather than F2. |
| R-11 | Community themes break as the interface evolves | F2+ | Semantic tokens, default resolution from the base theme, versioned schema. Validated with two in-house themes. |
| R-12 | WebAssembly compilation friction | F4 | Consciously accepted. |
| R-13 | The global keyboard hook is unhooked by timeout or flagged by antivirus | F3 | **Avoided entirely.** The overlay gesture uses Raw Input, not `WH_KEYBOARD_LL`, and the default hold key is right control rather than Super precisely so no suppression -- and therefore no hook -- is ever required. |
| R-14 | Foreground detection returns the container, not the real application | F3 | **Half solved, half declared out of scope.** Packaged applications are resolved by descending past `ApplicationFrameHost` to the first foreign visible child window. Browsers and Electron applications still report their container; the title rung of the sheet fallback chain exists for them. |

---

## 9. The warning that will weigh most

> The characteristic risk of assisted development on a project like this is not
> incorrect code: it is **volume**. It is very easy to accumulate functionality
> faster than it can be polished, and this project is defined precisely by
> polish. The invariants and the budgets exist to act as a brake. When they
> conflict with moving faster, they win.
