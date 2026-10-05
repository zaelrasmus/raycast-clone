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
| AD-12 | Themes are pure data, no code, no runtime | Zed's model. Semantic colour tokens only; a colour token that names a component turns every new component into a break for every existing theme. Geometry tokens name the launcher's parts, which do not multiply (F15). |
| AD-13 | Geometry is themeable through a style, within bounds, and layout is not themeable at all | *Revised in F15.* Was: themes control colour and typography, never geometry. A palette sets colour and the type family; a style sets bounded geometry tokens and picks closed variants; nothing sets the structure. The calibration is protected by ranges and checked relations instead of by being out of reach. |
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
| **F5** | Port to Linux and Wayland | **Frozen.** Stubbed; the cross-check (I-2) is paused until F5 resumes (7.38). The work, method by method, is in [`F5-LINUX.md`](F5-LINUX.md) |
| **F6** | Quicklinks and web search: saved links opened in a chosen browser, with a `{query}` placeholder that makes "search Google" one quicklink among others | **Done.** Step 1: file, placeholders, alias and fallback in root search, browser choice. Step 2: an argument pill on `Tab`, a *Create Quicklink* form in the launcher, and a Quicklinks page in settings. Step 3: site icons, and the browser picked from a dropdown |
| **F7** | Translator: a *Translate* command that turns what is typed into another language, online first, offline later | **Done.** Google and DeepL behind `trait Translator`; presets from `config.json` as root rows, edited on a *Translation* settings page; any source and target chosen in the view through *From* and *To* scopes. *Save as Preset* from the view was not built. **Offline translation is frozen.** See F7 decisions |
| **F8** | Timers: countdowns started from the launcher, named presets with aliases, alert sounds, stopwatches, cycles (Pomodoro) and focus (blocking launches through Lumen) -- one concept, not three features | **Done.** Timers, presets, alert sounds and ringing, stopwatches, cycles (Pomodoro, once or repeating) and focus, as one concept. Laps declined. See F8 decisions |
| **F9** | Currency: a *Currency* view that converts an amount, or an expression, into every currency or the ones filtered by code, name or country; and `10 usd in cad` answered in the root list | **Done.** The view, a searchable *From* picker (Translate's pickers search too), `in`/`to` filters and a written source, pins, root answers from the cache, and a *Currency* settings page. Historical rates are a door left open, not built. See F9 decisions |
| **F10** | Dictionary: a *Dictionary* command that explains a word from any language, and *Translate* showing a single word's alternatives by part of speech | **Done.** A detail pane in the vocabulary; *Dictionary* on English Wiktionary for 58 word languages, explained in English, the language detected or chosen; Translate's alternatives; each offers the other; favourites and history. See F10 decisions |
| **F11** | Snippets, the rest of the way: a *Search Snippets* view with tags and a detail pane, pasting into the application the launcher was opened over, dynamic placeholders, and a *Create Snippet* form with a Markdown editor | **Done.** The view, pasting with the clipboard put back, Raycast's placeholders with arguments and `{cursor}`, and the form with a Markdown editor and a placeholder picker. Expansion is deferred. See F11 decisions |
| **F12** | Clipboard history, the rest of the way: a *Clipboard History* command that opens its own view, with text, links, colours, files and images | **Done.** The command, the view with a *Type* filter and a pane, files captured, colours drawn as swatches, `Enter` pasting; images captured, kept as files by hash and shown in the rows and the pane; a settings page for the limit and for clearing the history. See F12 decisions |
| **F13** | World clock and weather: any city's time and weather, compared with where the user is, from a bundled catalogue of places browsed by continent, country and city | **Done.** World Clock: the view over 34,152 bundled cities with a drill-down pill, both row layouts, *My Places*, `time in vancouver` and `3pm vancouver` in the root list, and a settings page with the working-hours overlap. Weather: Open-Meteo, a 15-minute cache, the list, `weather in vancouver` in the root list, the report at full width or beside the list, and a settings page. See F13 decisions |
| **F14** | The calculator, the rest of the way: functions and constants, units and their conversions, percentages, dates, currencies inside an expression, and a *Calculator History* | **Done.** Functions, constants and angles; 130 units in 13 families; percentages, with `%` a percentage and `mod` the remainder (7.39); dates; money inside any expression, dated by its rates; the answer as a card in the place of two rows (7.40); *Calculator History* and a *Calculator* settings page; numbers written and read as the region does, or plainly. See F14 decisions |
| **F15** | Design: the look opened to the user. Geometry and variants in a style, chosen apart from the palette; the empty root list in sections; favourites; the settings window redesigned | **Done.** Styles with 38 bounded tokens and fourteen variants, reloaded live, sizing the window with config overrides; the search icon, the footer's mark and surface, the window's own edge; Carbon (after Raycast), Graphite (after Asyar) and Ink (after Vicinae), measured from screenshots, and following releases unless edited; a fonts folder and OpenType features; sections from `config.json`, built once and kept; favourites; every match listed; the settings window after Raycast's, with an *Appearance* page; `LUMEN_HOME` for test runs. Letter spacing is built as a `text.tracking` token (7.42, `LETTER-SPACING.md`); Graphite tracks at -0.02, and Carbon's and Graphite's type sizes were measured again. See F15 decisions |
| **F16** | File search the rest of the way: scopes and exclusions, folders as results, an index kept on disk and matched in memory, a *Search Files* view -- see [`FILE-SEARCH.md`](FILE-SEARCH.md) | **In progress:** slices 0-2 built; the index on disk with a hot layer in memory and no ceiling (c3a), *Search Files* in two phases (c3b), a scan root never waits on (c3c); idle with 405 000 entries 80.8 MB working set, 0.004 % CPU (c4); typed paths list their folder in root and the view (6d); extra application folders listed and watched as applications (7) |

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
- **A file or folder is a link too.** A link that is a path -- `D:\`, `D:/`,
  `\\server` or `file:` (`quicklinks::is_path_link`) -- is taken literally,
  braces and all, with no scheme required: it never reaches a browser's
  command line. Its `open_with` holds `app:<id>` (`APP_PREFIX`) rather than a
  browser name, so files written before keep their meaning; without one, or
  with the preferred browser the launcher filled in, the shell opens it with
  its default. Its icon is the file's own (`IconSource::ShellItem`), never a
  favicon. `trait Platform` gained `apps_for`, the applications that open a
  path with the default first, and `open_path_with`, in both implementations
  at once (I-9; Linux lists none until F5). On Windows they are the shell's
  recommended `SHAssocEnumHandlers`, the default moved first by the program
  `AssocQueryStringW(ASSOCSTR_EXECUTABLE)` names, and a file is opened by
  `IAssocHandler::Invoke` on its data object, the only way to hand a file to a
  packaged application. A folder is offered Explorer alone. A handler's id is
  its program's path, which for a packaged one names its version; a stored id
  that no longer matches finds the one handler whose program has the same
  file name.
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

### F9 decisions

The shape is Raycast's *Currency Exchange* extension (MIT, by xeric): open
it, pick the currency you have, type an amount, read it in every other
currency. Its source taught the one thing that matters: **the network is
not on the typing path.** It fetches one table of rates, base USD, once a
day, and every conversion after that -- CAD to AED included -- is a
division. So unlike F7, a currency view is not a request per keystroke,
and the design is about the table: where it comes from, where it is kept,
and how old it is allowed to be.

- **A table, fetched at most once per publication.** A rate source returns
  every rate against one base, when it was published and when the next
  table is due. The table is kept in `currency.json` in the data folder
  with those two moments, and opening the view with a table that is not
  yet due makes no request at all. A table that is due is still shown at
  once, with its real date, while one refresh runs on a thread; its answer
  redraws the view. Nothing is scheduled: a daemon that is never asked for
  a conversion never fetches one, so the idle budget does not move.
- **Keyless by default.** `open.er-api.com/v6/latest/USD` is
  ExchangeRate-API's open endpoint: no key, 166 currencies, a new table
  daily, a `429` for twenty minutes to whoever asks too often -- which a
  once-a-day cache never does. Its terms ask for attribution, which the
  view gives in the line that carries the date: *Rates of 28 Sep 2026 ·
  ExchangeRate-API*. So the command works on first use, as Translate does.
  UniRateAPI answered `401 Missing API key` without one, so it cannot be
  the default.
- **A source behind a trait, chosen by the user.** `currency.service` in
  `config.json` picks one: `open` (the default), `exchangerate-api` with
  `currency.api-key` (the same data, the keyed endpoint and its quota), or
  `unirate` with the same key field. No fallback between them, as in F7:
  the user chose who is asked. The key is never logged (`Debug` by hand)
  and never shown back. The cached table records its source, so changing
  service fetches afresh rather than showing one service's rates under
  another's name. A *Currency* settings page edits the section through a
  store, like *Translation*.
- **Historical rates are left a door, not built.** The keyless endpoint has
  no history (`/v6/history` is a `404`); ExchangeRate-API and UniRateAPI
  sell it. Frankfurter (ECB data) is keyless with history back to 1999,
  but covers thirty currencies. So the source contract asks for rates
  *latest* or *on a day*, the cache is keyed by both, and every source
  built now refuses a day. A dated query (`10 usd in cad at 2025-01-02`) is
  then one more source and one more clause in the parser, not a redesign.
- **A built-in view source, not an extension.** `ViewSource::Currency`
  speaks the `lumen_core::view` contract in process, under
  `Filtering::Extension`, as Translate does (I-13). The decisions are a
  value: `currency::session::Session` takes events and tables and returns
  effects -- render, fetch, copy, remember -- and is unit-tested;
  `lumen-app/src/currency.rs` owns the thread, the client and the file.
  The client is F7's blocking `zed-reqwest`, dropped when the fetch ends:
  one request a day does not need a pool kept warm.
- **What is typed:** `<amount> [<code>] [in|to <filter>]`.
  - The amount is the calculator's arithmetic, so `1+2`, `(3*4)/2` and
    `10%3` work -- `10 mod 3` since F14, where `%` became a percentage
    (7.39). The calculator's gate -- an operator must appear -- is
    what keeps the root list quiet, and it is wrong in a view whose whole
    purpose is a number, so the gate and the evaluator are separated and
    the view asks for the evaluator alone: `10` is an amount here.
  - A code after the amount is the source for this query, over the one
    picked in *From*: `10 cad in eur` converts Canadian dollars whatever
    the picker says. The picker is the default, the text is the exception.
  - The filter after `in` or `to` narrows the rows with the root list's
    matcher, against each currency's code, name and **every country that
    uses it**, from ISO 4217 (published 2026-09-17): `in canada` finds CAD,
    `in spain` finds EUR, and `in c` ranks word starts -- CAD, CHF, CNY --
    first, exactly as root search would. An unfinished `10 in` shows every
    currency rather than none.
- **One row per currency, in the screenshot's shape.** The icon is the
  region's two letters (`Icon::Text`, since Windows ships no flag emoji),
  the title the code, the subtitle the amount with its symbol where one is
  common (`$`, `CA$`, `€`, `£`, `¥`, ...) and the code otherwise, the
  accessory the currency's name. The rows are the table's order, by code,
  with the source left out; the set of rows changes only when the filter
  does, never while a table arrives (I-10). `Enter` copies the plain
  number -- no symbol, no grouping -- because it is pasted into forms.
- **Amounts in the currency's own units.** Two decimals for most, none for
  the yen or the won (ISO 4217's minor units), grouped by thousands. An
  amount below one gets the decimals it needs to show three significant
  digits, so ten dong in dollars is `$0.000379`, not `$0.00`.
- **The date is a section subtitle,** which the vocabulary gains: `Section`
  had only a title, and the screenshot's dimmer second half is exactly a
  subtitle. One optional field, in the one definition (I-13), drawn in the
  same header row so no height changes (I-10). The date is the table's
  publication date in UTC, which is when it is published.
- **The source is a scope, and the picker learns to search.** *From* is
  the scope Translate introduced, with 166 options. Arrowing through 166
  is not choosing, so typing while the picker is open filters its options
  with the same matcher -- `can`, `yen`, `japan` -- and closing it gives
  the search field back its query. Translate's thirty languages get the
  same for free. The chosen source is remembered in `currency.json`, across
  restarts: the currency someone holds does not change daily.
- **Keywords are scored even when the title matches, root list included.**
  Searching the picker by country exposed a gap in the shared matcher: a
  keyword was consulted only when the title did not match at all, so any
  scattered title match won -- `spain` found the Saint Helena pound, through
  the S, P, A, I, N of "SHP Saint", before the euro. The better of the two
  scores now counts, and the keyword discount is 30 rather than 40, because
  the scattered match scores 101 and the exact keyword 140. An equally good
  title still wins. The ranking budget in CI was re-run and holds.
- **Pins.** *Pin* in the action panel keeps a currency in a section of its
  own above the rest, remembered with the source. The filter applies to
  both sections.
- **Offline is F7's sorting, and it matters less.** With a table in the
  cache the view works offline and the date says how old it is. Only with
  no table at all is *No internet connection* a row, with *Try Again*.
- **The root list answers `10 usd in cad`, from the cache alone.** F7 kept
  translation out of the root list because it is a request per keystroke.
  A conversion is not: it is a division against a table in memory, so a
  root answer costs what the calculator's does, and I-3 holds as long as
  the provider never fetches. The launcher holds the table the daemon last
  loaded; the provider reads it and nothing else. It answers only when
  the query is unambiguous -- an amount, a known source code, `in` or `to`,
  and a target that is exactly a code, a currency name or a country --
  and says both sides: *14.16 CAD*, with *10 USD = 14.16 CAD · rates of
  28 Sep 2026* beneath. Recognising such a query with no table, or a table
  that is due, asks the daemon for a refresh in the background; the answer
  appears when it lands, and the keystroke in hand is never held for it.
- **The slices,** one acceptance criterion each:
  1. *Currency* opens a view; typing `1+2` shows every conversion from
     USD with the table's date and source; `Enter` copies. Keyless fetch,
     the cache, and offline.
  2. A searchable *From* picker, remembered across restarts.
  3. `in <filter>` with the root matcher and country names, and a code
     after the amount as this query's source.
  4. Pins.
  5. `10 usd in cad` answered in the root list from the cache.
  6. The service and key in `config.json` and a *Currency* settings page.

  The idle baseline is measured before slice 1, and again after 5.
- **Measured.** With `scripts/measure-idle.ps1`, 60 s hidden, on the same
  machine: before slice 1, 0.002 % CPU, 73.7 MB working set, 87.1 MB
  private; after slice 5, 0.000 %, 73.5 MB, 87.5 MB. Nothing is scheduled,
  so there is nothing to cost. The table is read with the index, off the UI
  thread, and a root conversion asks for a fetch at most once per showing
  of the window, so a failing one is not retried on every key.

### F10 decisions

Three Raycast extensions were read before planning: *Web Dictionaries*
(MIT, by DrChai), *Easy Dictionary* (MIT, by isfeng) and *Simple
Dictionary* (MIT, by CT-7567). Each answers the same wish -- "what does
this word mean" -- in a different shape, and each taught one thing:

- *Web Dictionaries* gets the translation, the alternatives by part of
  speech and the definitions **from one Google request**, by asking
  `translate_a/single` for more fields (`dt=bd`, `dt=md`, `dt=ex`). The
  translation row comes first, the dictionary under it, and a selected
  sense opens beside the list.
- *Easy Dictionary* is one box where **the input decides**: a word gets a
  dictionary card, a sentence gets translations. It fits its users, who
  learn English from Chinese, and a bilingual dictionary is the dictionary
  they want.
- *Simple Dictionary* reads `freedictionaryapi.com`, a keyless service
  built on English Wiktionary: senses grouped by part of speech, examples,
  IPA and inflected forms, for 279 languages. Its history and favourites
  are taken, in one view rather than its three commands.

What was checked on this machine, with curl, before writing this:

- `clients5.google.com/translate_a/single?client=dict-chrome-ex`, the host
  F7 already talks to, returns `dict` (alternatives by part of speech),
  `definitions` and `examples` for `display`. The `/t` path Translate uses
  returns the translation alone. **Not yet checked with the blocking
  client**, and F7 found that Google judges the TLS client, not the request.
- Google's definitions are **English words only**: `display` has them,
  `mostrar` has none.
- English Wiktionary explains **every language in English**: `significado`
  is *significant; significance*, `bonjour` is *greetings; hello*, each
  with IPA. Coverage is uneven -- 1.4 M English words, 760 K Spanish, far
  fewer for small languages -- and senses come in Wiktionary's order, which
  puts `significado` the adjective before `significado` the noun.

The decisions:

- **Two commands, not one.** *Translate* answers "how do I say this";
  *Define* answers "what does this mean". They differ in shape: Translate
  has two languages and up to four targets per preset, Define has one
  language; Translate may be DeepL, which has no dictionary; and senses
  under three target rows would be noise. Merging them was Easy
  Dictionary's answer, and it is the right one for a bilingual learner's
  tool, which Lumen is not. Each command offers the other in its action
  panel -- *Define "display"*, *Translate "display"* -- so the second
  question is one key away from the first.
- **Translate gains a word's alternatives, at no extra request.** When the
  text is a single word and the service is Google, the rows below the
  translation are its alternatives by part of speech -- *verb: mostrar,
  exhibir, desplegar...* -- from the same response, by moving from `/t` to
  `/single` on the same host. With DeepL, nothing is added. This waits on
  the blocking client getting the answer curl got; if it does not, it is
  dropped, not worked around.
- **Define reads English Wiktionary, keyless.** `freedictionaryapi.com`
  first, behind `trait Dictionary`, so another source is one more
  implementation. Google's definitions were rejected as the source because
  they cover English words alone. The data is Wiktionary's, under CC BY-SA,
  so the view attributes it the way F9 attributes its rates. The service's
  own terms and limits are to be read before slice 2.
- **A word has two languages, and they are kept apart.** The *word
  language* is the one the word belongs to -- `mostrar` is Spanish -- chosen
  in a *From* scope, as in Translate. The *explanation language* is the one
  the definition is written in, and it is the app's: English today, since
  Lumen has one language. Wiktionary is organised the same way -- each
  edition explains every language in its own -- so when Lumen has a Spanish
  interface, a Spanish explanation is another edition behind the same
  trait, not a redesign. To keep that true, the explanation language is
  an argument of the trait from the first commit, read from one function
  that returns English; it is part of every cache key; and nothing in the
  parser assumes English. Which source serves other editions is not
  settled: Wiktextract, the extractor behind this data, parses several,
  and that is to be checked when the time comes.
- **A list with a detail pane, which the vocabulary gains first.** A
  definition is a sentence and an example another; a row is one line
  (I-10). All three extensions show the selected sense beside the list,
  and Lumen cannot: `View` is a list *or* a detail, and
  `Event::SelectionChanged` exists "so a detail pane can follow it" with no
  pane to follow. Pushing a `Detail` is not the answer, for the reason F7
  gave: `Esc` in a view clears the query. So an item may carry a detail --
  the existing `Detail`'s Markdown and metadata -- and a list may ask for
  the pane, in the one definition (I-13). The pane's width is a layout
  token (I-5), the rows keep their height, and moving the selection
  changes the pane's content, never the window's size (I-10). A view that
  asks for no pane is unchanged, so World Clock, Characters, Translate and
  Currency did not change -- though Translate's long translations are the
  pane's next use. Restricted Markdown has no tables, so pronunciations
  are a list or metadata rows, not Simple Dictionary's table.
- **Define's view.** A section per part of speech (*Noun*, *Verb*), a row
  per sense: the definition as the title, its first example dimmed as the
  subtitle. The pane shows the headword, the sense in full, every example,
  the pronunciations and the forms. `Enter` copies the selected
  definition. A word the source does not have is said as such, with the
  word language named, since uneven coverage makes it common for small
  languages; offline is F7's `Failure::Offline`.
- **Nothing waits on the network,** as in F7: the lookup is sent after
  typing pauses, answers carry a generation and stale ones are dropped,
  and the client is the blocking one already linked, built on first use
  and dropped after. Root search does not change (I-3); `define hola` as
  an inline prefix is deferred for F7's reason.
- **Favourites and history, in Define's empty state.** Before anything is
  typed, the view shows *Favourites*, then *Recent*; choosing a word opens
  its definition. Simple Dictionary makes them three commands; here they
  are what an empty search shows. Both live in `dictionary.json` in the
  data folder, like `currency.json`: a file, not a database, because they
  are a few hundred rows written once per lookup, not per keystroke.
  - A **favourite** is F9's pin under another name: *Add to Favourites* in
    the action panel. It keeps the **whole entry** -- senses, examples,
    pronunciations, forms, the word language and the explanation language
    -- so it opens offline and survives the source changing or dropping
    the word.
  - **History** keeps the word and its word language only, the last
    hundred, newest first, and looks it up again when opened. A list of
    every word looked up is a record to guard, as F6 found for typed URLs:
    the panel offers *Remove from History* and *Clear History*, and
    `dictionary.history` in `config.json` turns it off.
- **Urban Dictionary is a quicklink, not a source.** Web Dictionaries
  offers it as a second engine: slang written and voted on by its users.
  Its API (`api.urbandictionary.com/v0/define`) answers but is
  undocumented, its content is unmoderated in ways Lumen cannot filter,
  and it is English alone -- while English Wiktionary already carries
  common slang (`rizz` is there, marked *slang*). Whoever wants it adds
  `https://www.urbandictionary.com/define.php?term={query}` as a quicklink
  (F6): one mechanism, not two, and the choice stays the user's. It is not
  in the example file.
- **Left out.** Text to speech (Lumen plays no audio but alert sounds),
  looking up the selected text in another application (reading it is a
  `trait Platform` change with a Linux counterpart, I-9) and exam tags.
- **Open: what *Detect* means for a word.** `freedictionaryapi.com` needs
  the word language in the URL. Asking Google which language a word is
  would send it to a service the user did not choose for Define, which
  F7's no-fallback rule forbids. Wikimedia's own endpoint
  (`en.wiktionary.org/api/rest_v1/page/definition/{word}`) answers every
  language for a word in one request, but without pronunciations or
  forms. Until this is decided, *From* starts on English and has no
  *Detect*. *(Decided after slice 6: see below.)*
- **The slices,** one acceptance criterion each:
  1. The vocabulary gains the detail pane, and the renderer draws it;
     every existing view looks and behaves as before.
  2. *Define* opens a view; typing an English word shows its senses by
     part of speech, the pane follows the selection, `Enter` copies.
     Not found, and offline.
  3. The *From* scope: a word from any language the source has, explained
     in English.
  4. *Translate* shows a single word's alternatives with Google, after the
     blocking client is checked against `/single`.
  5. *Define* and *Translate* offer each other in the action panel.
  6. Favourites and history in Define's empty state, kept in
     `dictionary.json`; history can be cleared and turned off.

  The idle baseline and the binary size are measured before slice 1, and
  again after 6.

What changed on the way, and what was learned:

- **Views could not scroll to a row they had never drawn.** Found on the
  first screenshot of *Define*: arrowing down moved the selection and the
  pane, not the list. GPUI's `scroll_to_reveal_item` adds up the heights it
  has *measured*, and a row below the fold has never been drawn, so it
  counts as nothing and the list decides the row is already in view. It
  was true of every view with more rows than the window; the pane only
  made it visible. `measure_all()` would have fixed it by drawing every row
  on each render, against the 8 ms frame budget in a 166-row Currency list.
  Instead `view::reveal` works the offset out from the rows' heights, which
  are known exactly (I-10), and moving up to a section's first item shows
  its header too.
- **A built-in view can open another on a word.** F7 noted that a view
  cannot set the search field. That stays true: `Launcher::set_query` is
  the daemon's, used when *Define* opens *Translate* (or the reverse) and
  when a favourite is chosen. An extension still cannot type over the user.
- **The pane takes the window's full height** when the rows fit, so two
  senses still leave room to read them; when they do not, the list ends on
  a whole row as before.
- **The word languages are a table of 58,** the living languages with
  about ten thousand entries or more in English Wiktionary. Historical
  ones and "Translingual" are left out: a picker of 279 is not choosing.
- **Lookups are case-sensitive at the source.** `Display` has no entry,
  `display` does, and a German noun is only found with its capital. So a
  word is asked for as typed, then in lower case if that finds nothing. A
  word the source does not have is a `200` with no entries, read as
  *not found*; an answer that is not an object is an error, so a broken
  reply is never shown as "no such word".
- **Alternatives use `/single` for one word only.** Checked with the
  blocking client first, as F7 taught: `200`, three times running. Text of
  more than one word stays on `/t`, which keeps line breaks. The terms are
  cut to eight per part of speech; Google lists thirty. The alternatives
  are a section under the translations, so the list grows once when they
  arrive; they are kept while the same word is asked again, and dropped as
  soon as the text is more than a word.
- **History records a word when it is used,** not when a lookup answers:
  copied, opened, translated, saved, or left on screen when the view
  closes. Recording answers would fill it with `dis` and `disp` on the way
  to `display`. `dictionary.history: false` in `config.json` stops it and
  clears what was kept: a list nobody can see is one nobody can erase.
- **Favourites are among the answers from the start,** keyed by word, word
  language and explanation language, so choosing one asks nothing and
  works offline.
- **Define became *Dictionary*.** Used, "Define" read as an odd verb for a
  command; the root row, the placeholder and the cross-links say
  *Dictionary* and *Look up*, and "define" stays a keyword. Code followed:
  `ViewSource::Dictionary`, `providers::dictionary`.
- ***Detect* is one request to the same source.** The service answers
  `entries/all/{word}` with every language that has the word -- found by
  trying it, not in its documentation -- so detecting needs no second
  service and F7's rule holds. Only languages in the table count. The
  hard part is choosing, because a word is rarely in one language:
  `significado` is in 3, `gift` in 9, `hola` in 18, `no` in 90. The view
  picks the language with the **most senses** (`display` is English with
  11, `significado` Spanish with 6); a **tie** goes to a language in the
  user's history or favourites, then to English. The choice is said -- the
  section title leads with the language, the pane says *(detected)* and
  *Also in* -- and the others are *Show in ...* in the action panel, from
  what the view already has. Choosing a language in *From* still sets it
  outright. *Detect* is where the view starts, and is remembered like any
  other choice; *Show in ...* is not, so the next opening still detects.
- **Measured.** With `scripts/measure-idle.ps1`, 60 s hidden, same machine:
  before slice 1, 0.002 % CPU, 72.9 MB working set, 87.2 MB private, 31
  threads, binary 17,104,384 bytes; after slice 6, 0.000 %, 73.8 MB,
  88.5 MB, 31 threads, binary 17,334,784 bytes (**+225 KB**). No new
  dependency: the client is the one Translate already uses, built on first
  use and dropped thirty seconds after the last lookup.

### F11 decisions

Snippets exist since F3: `snippets.json`, a name, a text and keywords,
matched in the root list and copied on `Enter`. Raycast's snippets were read
again as the target -- *Search Snippets* and *Create Snippet* -- and they do
five things Lumen's do not: they **paste** into the application the user was
in, they carry **tags** and a **keyword**, they show the selected snippet
**beside the list** with its dates, they fill in **dynamic placeholders**
(`{clipboard}`, `{date}`, `{cursor}`, `{argument}`...), and they are written
in an **editor with formatting**. F11 is those five, and not a sixth.

The decisions:

- **Snippets leave the root list.** Two catalogue rows take their place,
  *Search Snippets* and, with the form, *Create Snippet*. A hundred snippets
  matched beside the applications is 7.14's problem again: the root list
  answers "open something", and a snippet is something you search for in its
  own view. The `SnippetProvider` survives as what says a snippet row looks
  like, as the clipboard's did.
- **Search Snippets is a list with the detail pane,** the vocabulary F10
  built, and nothing new in it. A row is the name, with the keyword as a
  neutral tag beside it; the pane is the text, then *Name*, *Keyword*,
  *Tags*, *Content Type*, *Modified* and *Created*. Sections are by date
  modified -- *Today*, *Yesterday*, *This Week*, *This Month*, *Earlier* --
  newest first, and a snippet with no dates, written by hand, is *Earlier*.
  The text is matched as well as the name, so a snippet is found by what it
  says -- and so the view filters itself rather than asking the host. The
  host's matcher is fuzzy, and fuzzy against a paragraph matches nearly any
  short query; host filtering keeps the view's order rather than the
  score's (7.30), so the loose matches would not even sink. A snippet is
  kept when every word typed appears in its name, keyword, keywords, tags or
  text.
- **The pane gains a verbatim body.** `ItemDetail::text`, drawn after the
  Markdown with every line break kept and nothing read as a marker. The
  restricted Markdown joins a paragraph's lines and has no escapes, so a
  signature would be one line and `a * b` would be emphasis; a vocabulary
  addition was the smaller change than escaping text to have it unescaped.
- **Tags are a scope,** *All Tags* and then every tag in use, sorted: the
  F7 pill, searchable as F9 made it. A tag is a word; a snippet has any
  number of them.
- **The file gains fields, schema 2.** `keyword`, `tags`, `created`,
  `modified` and `format`. Dates are RFC 3339 strings, because the file is
  one people edit and `2026-09-29T21:00:19Z` is readable where a count of
  seconds is not. A version-1 file loads unchanged, every new field
  optional (I-12's reasoning, for data). The file stays the source of truth
  and is written the way F6 writes `quicklinks.json`: refused when it does
  not parse, unknown fields kept, replaced in one rename.
- **One keyword, and the keywords stay.** `keyword` is Raycast's field: one
  word, unique across snippets, shown on the row and reserved for expansion.
  `keywords` is F3's list, matched and never shown. They answer different
  questions, and a file written for F3 must not change meaning.
- **`Enter` pastes.** Copying is the second action. Pasting is a new method
  on `trait Platform` (I-9): put the snippet on the clipboard, send `Ctrl+V`
  to the window captured before the launcher appeared -- never "the
  foreground", for 7.15's reason -- and put back what was there. The Linux
  implementation is a stub, as the rest of F5 is.
- **The clipboard is put back by default.** `snippets.paste` in
  `config.json` is `restore` or `keep`. With `restore`, the snippet is
  marked with `ExcludeClipboardContentFromMonitorProcessing`, which 7.13
  already honours, so it never reaches the history; everything that was on
  the clipboard is restored, not only its text. When to restore is the hard
  part: the target reads the clipboard whenever it gets round to handling
  the keystroke, and restoring before that pastes the old contents. How
  that moment is detected is slice 2's to find out and to write down here.
  With `keep`, the snippet stays and enters the history like any copy.
- **The text is Markdown, drawn as Markdown source.** The editor's text is
  the file's text: the toolbar wraps the selection in `**`, `_`, `~~`, `++`
  or `[...](...)`, and the editor styles what it shows from a parse, the
  markers kept visible and dimmed. Because the text on screen *is* the
  source, the cursor, the selection, undo and the input method work on one
  string with nothing to map. A true rich-text editor -- a document model,
  selections across styled spans, HTML read back from a paste -- was the
  alternative, and is several phases of work for the markers not showing.
  Hiding them while the cursor is elsewhere (Typora's live preview) can be
  built on this later; it cannot be built on a rich-text model's ruins.
- **A pasted Markdown snippet carries both forms.** The clipboard gets HTML
  for the applications that read it (a mail, a document) and the text with
  the markers taken out for those that do not (a terminal). `format` says
  which snippets are Markdown: the form writes `markdown`, and a snippet
  without the field is `text` and is pasted exactly as written, since a
  command with a `*` in it written for F3 is not emphasis. Underline is not
  Markdown; `++underline++` is the common extension and is what is used.
- **Placeholders are parsed at load,** as F6's templates are, so a typo is
  reported against the snippet that has it. The engine is in `core` and
  pure: the clipboard and the time are arguments. The set is Raycast's that
  can be kept honestly: `{clipboard}` and `{clipboard offset=N}` from the
  history, `{date}`, `{time}`, `{datetime}` and `{day}` with `format` and
  `offset`, `{uuid}`, `{cursor}`, `{argument name=... default=...}`,
  `{snippet name=...}`, and the modifiers `uppercase`, `lowercase`, `trim`,
  `percent-encode` and `json-stringify`. A literal brace is `{{`.
- **Local time comes from `jiff`.** It is already in the build (F9) without
  time zones; `{date}` and the pane's *Today, 9:00 PM* need the user's.
  `jiff`'s own system time zone, with the database bundled on Windows,
  rather than a `trait Platform` method, because a UTC offset from the
  platform is wrong across a change to daylight saving time, which is
  exactly what `offset=+7d` crosses. What it costs in binary size is
  measured, not assumed.
- **Arguments are asked for in the search field,** as F6 asks for a
  quicklink's query: a pill per argument, three at most, `Tab` between them,
  and nothing is pasted until `Enter`.
- **`{cursor}` moves the caret back** after the paste, by sending `Left`
  once per character after it. Characters as the target counts them, which
  is a grapheme almost everywhere and a line break as one; where it is
  wrong the snippet is still pasted whole, and nothing is lost but the
  caret's place.
- **Left out.** `{selection}`: reading the selected text means sending
  `Ctrl+C` to someone else's application and hoping. An icon per snippet:
  the icon set is eight files on one grid (7.23), and a snippet is not a
  ninth kind of thing. Images and files in snippets.
- **Expansion is deferred, and the door is written down.** Typing a keyword
  anywhere and having it replaced needs every keystroke in every
  application, which this project has refused to take with a
  `WH_KEYBOARD_LL` hook (R-13). There is a way that does not contradict it:
  the Raw Input sink already registered for the hold gesture (7.1) sees
  every key without a hook and without the timeout that silently removes
  one. It cannot suppress a key, and expansion does not need to: it types
  `Backspace` over the keyword and pastes. The costs are real -- Raw Input
  reports keys, not characters, so layouts, dead keys and input methods are
  the work; matching must fit I-15, so a fixed ring buffer and no
  allocation; and it is still a program reading every key -- and they are a
  phase of their own, after this one is polished. Until then `keyword` is
  matched and shown, and the paste covers the need.
- **The slices,** one acceptance criterion each:
  1. Schema 2 read, and *Search Snippets* opens a list with the pane, date
     sections and the tag scope; `Enter` copies. Snippets are no longer in
     the root list.
  2. `Enter` pastes into the application the launcher was opened over, and
     `snippets.paste` decides whether the clipboard is put back.
  3. Placeholders: the clipboard, dates and times in local time, `{uuid}`,
     `{snippet}` and the modifiers.
  4. `{argument}` in the search field, and `{cursor}`.
  5. A multi-line text area, drawing Markdown source styled, with the
     toolbar. Emoji are checked in it, drawn in colour and typed with
     `Win+.`.
  6. *Create Snippet*, and *Edit*, *Duplicate* and *Delete* from the view,
     in the form from Raycast's two columns; a Markdown snippet pastes HTML.

  The idle baseline and the binary size are measured before slice 1, and
  again after 6.

What changed on the way, and what was learned:

- **A snippet that cannot be read is said, not dropped.** Found by review
  of slice 1: with more fields, one mistyped by hand (`"tags": "work"`)
  failed the whole file, and a file that failed looked like *No snippets
  yet*. The file is now read one snippet at a time; one that fails is
  listed under *Could not read* with serde's reason, and a file that does
  not parse shows its error as the empty state.
- **The moment to restore is when the target reads, found by delayed
  rendering.** A window of Lumen's owns the clipboard and announces the
  snippet without supplying it; Windows asks that window for it
  (`WM_RENDERFORMAT`) when somebody reads it, and the window that has the
  clipboard open then is the reader. The clipboard is put back 250 ms after
  the target's read, for the other formats an editor asks for in turn.
  Everything that was on it is kept as bytes, except GDI handles (the DIB
  beside a bitmap is kept, and the bitmap comes back from it) and OLE's own
  formats, which point into the process that copied. Measured on this
  machine: Notepad and a terminal read within the same second the key is
  sent, and the clipboard came back.
- **The restore was first undone by Lumen itself.** Found by logging each
  step, after a manual test said the old clipboard did not come back: the
  restore ran, and then the window, closing, received `WM_RENDERALLFORMATS`
  and wrote the snippet again. It now supplies the text only while it is
  still owed.
- **The reader is the target's program, not its process.** In an Electron
  application the window belongs to one process and a different one reads
  the clipboard (`Chrome_MessageWindow`), so the read went unseen and the
  restore waited for the two-second fallback. A reader running the same
  executable as the target counts. When something else entirely reads first,
  the target's read cannot be seen -- the text is already supplied, and
  reading it asks nothing -- so the clipboard is restored on the fallback
  without an error. Only when nothing reads at all is the paste reported as
  dropped, which is what a window running as administrator does (UIPI
  discards the keystrokes silently); the snippet is then left on the
  clipboard.
- **Not every program pastes on `Ctrl+V`.** Some terminals pass it on to the
  program running in them; the one this was tested in pastes on
  `Ctrl+Shift+V`. `snippets.paste-keys` names the keystroke by executable,
  over a short built-in list (WezTerm, Alacritty, mintty), and `trait
  Platform` gained `window_application` to say which program owns the
  captured window, off the UI thread.
- **Placeholders are Raycast's syntax, read from its manual, and what is not
  one is text.** The design said a typo would be reported; the manual says
  an unrecognised placeholder is left alone, and for snippets that is right,
  because they are often code: `function() { return x; }` and a JSON object
  must paste as written. So anything in braces that is not a valid
  placeholder -- unknown name, parameter or modifier, a bad offset, a
  `format` beside a `locale` -- is pasted exactly as written, and a
  placeholder that cannot be filled (a snippet that does not exist, a
  locale the system does not know) is too. There is no `{{` escape. Modifiers
  are a pipe, `{clipboard | trim | uppercase}`, with `raw` accepted and
  changing nothing in a snippet; offsets are `offset="+2y +5M"`, `m` minutes
  and `M` months; `{snippet}` goes one level deep and refuses a snippet that
  refers to others, which also stops one that names itself.
- **Dates are written by the system.** Raycast's `{date}` follows the
  system's regional settings and `locale="fr-FR"` picks another, which needs
  every locale's names and orders. Windows has them: `trait Platform` gained
  `format_date`, over `GetDateFormatEx` and `GetTimeFormatEx` -- the short
  date and the time without seconds, as the taskbar clock writes them, so
  `{date}` on this machine is `30/09/2026`, not Raycast's `30 Sep 2026`.
  Bundling ICU4X instead was several megabytes for what the system already
  has. A custom `format` is a Unicode pattern (`EEEE, MMM d, yyyy`, `'text'`
  quoted) written in core, with English names, since a pattern and a locale
  are never given together; Raycast's ten examples are its tests. Raycast's
  Unicode extensions in a locale (`en-US-u-hc-h23`) are not Windows locale
  names and are refused.
- **The calculator is both a placeholder and a modifier.**
  `{calculator expression="12*7"}` for a fixed sum, and `| calculate` to
  evaluate what another placeholder produced -- `{clipboard | calculate}` --
  with the launcher's own calculator. A value that is not arithmetic is left
  as it was.
- **Arguments are asked for one at a time, not side by side.** The design
  said a pill per argument with `Tab` between them, which is Raycast's
  look; that is a new multi-field component in the search bar, with its own
  focus, selection and input method handling. Instead `Enter` on a snippet
  with arguments empties the search field and makes it the first answer:
  the placeholder names the argument (and its default), a choice's
  `options` are the list, filtered as typed, and the pane shows the snippet
  filled in so far -- other placeholders as written, since the clipboard
  and the time are read at the paste, not before. `Enter` moves on, and on
  the last pastes; a required argument left empty, or a choice not among
  the options, does not advance. *Back to Snippets* in the action panel
  returns to the list with the search as it was; `Esc` clears the answer,
  then leaves the view, as in any view. It is all in the session, over the
  view, the field and the pane that were already there. Raycast's rules
  hold: three arguments at most, a name shared by several placeholders is
  asked once, an unnamed one is an argument of its own, and `options` were
  added, which the design had missed.
- **`{cursor}` is Left, counted in graphemes.** After the paste keystroke,
  in the same `SendInput` call, Left once per grapheme after the cursor --
  an emoji of several code points is one press, a line break is one --
  which is how editors move. The target reads the clipboard while handling
  the paste, before the first Left reaches it, so the order holds. A second
  `{cursor}` is text, and one more than 2,000 presses from the end leaves
  the caret at the end. In a snippet inserted with `{snippet}`, `{argument}`
  and `{cursor}` are text: only the snippet being pasted asks and places.
- **The editor is one parse read three ways.** `snippets::markup` reads the
  Markdown source into a small tree once; the editor's styling (markers
  dimmed, content bold, italic, underlined, struck, code or link), the
  plain text a terminal gets and the HTML a document gets are three walks of
  that tree, so they cannot disagree about what is emphasis. `snake_case`,
  `* bullet` and `a * b` are text, as in `crate::markdown`. The toolbar's
  buttons wrap the selection in markers and take them off when pressed
  again; they have keys too (`Ctrl+B`, `I`, `U`, `E`, `Ctrl+Shift+X`,
  `Ctrl+K` for a link, `Ctrl+\` to clear), bound in the editor's own
  context, which is deeper than the launcher's and so wins while it has
  focus.
- **The editor shares the single-line field's model, not its layout.**
  `EditBuffer` never assumed one line, so moving, selecting, deleting and
  the input method are the same code. What is new is layout: GPUI's wrapped
  lines say where each character is (`position_for_index`) and which is
  under a point (`closest_index_for_position`), which is enough for the
  caret, the mouse, the input method's candidate window, and Up, Down, Home
  and End by visual line -- no patch to the vendored GPUI. Undo is new too:
  a run of typing is one step, and any other edit is a step of its own.
  The styling is read when the text changes, not per frame.
- **A plain-text snippet stays plain.** New snippets are Markdown. One
  written for F3 opens without styling or toolbar and is saved as text,
  because a command with a `*` in it is not emphasis.
- **The form is Raycast's two columns,** a third kind of form in the
  launcher beside the quicklink's and the timer preset's, so the title row,
  the resize and `Esc` came with it. `Enter` in the editor is a new line,
  so `Ctrl+Enter` saves, and the footer says so. After a save the launcher
  opens *Search Snippets* with the snippet's name typed, so what was saved
  is what is shown. Tags are one field, separated by commas.
- **The file is written as `quicklinks.json` is:** a file that does not
  parse is refused, unknown fields and entries are kept, and it is replaced
  in one rename. Names and keywords are unique; `created` is kept and
  `modified` stamped, to the second. *Edit*, *Duplicate* (named *copy*,
  without the keyword, which would clash) and *Create* are in the view's
  action panel, and *Delete* asks first, in the list's place, with the
  snippet in the pane.
- **A Markdown snippet pastes as HTML and as text.** The paste offers both
  formats by delayed rendering, and either being read counts as the target
  taking it; `HTML Format` carries its own byte offsets, tested. The plain
  text has the markers taken out, and `{cursor}` is counted in it: the
  caret's place is marked with a private-use character before the markers
  go and found again after, so a cursor inside a span lands right. A copy
  is the plain text.
- **Toolbar buttons grow to their label.** Found on the first look: they
  were fixed squares, and *Link* spilled onto its neighbours.
- **Placeholders are picked, not remembered.** Raycast's editor lists them
  when `{` is typed; so does this one, and a `{ }` button at the toolbar's
  far end opens the same list. What is typed after the `{` filters it, by
  title, keyword or name; choosing one replaces the `{` and the filter with
  the placeholder's syntax, the caret left inside `name=""` where there is
  something to type. A space, a brace or moving the caret away closes it,
  and what was typed stays text. While it is open the editor carries a
  second key context whose bindings are declared after its own, so Up,
  Down, Enter, Tab and Escape go to the list: GPUI prefers the later of two
  bindings at the same depth, and that is what makes the list modal without
  a key being compared by hand (I-14). The rows have no icons, unlike
  Raycast's: the icon set is eight files (7.23).
- **Measured.** With `scripts/measure-idle.ps1`, 60 s hidden, same machine:
  before slice 1, 0.007 % CPU, 73.5 MB working set, 89.1 MB private, 31
  threads, binary 17,357,312 bytes; after the picker, 0.007 %, 73.7 MB,
  86.1 MB, 31 threads, binary 18,161,664 bytes (**+786 KB**). No thread is
  added while hidden: a paste has a thread of its own for as long as the
  target takes to read, and none is kept. The growth was not broken down;
  it includes `jiff`'s time zones, now compiled in, and the editor.
- **A paste is never logged with its text.** `Paste`'s `Debug` prints the
  length only, because the daemon logs an event that failed with `{:?}`, and
  a snippet can be a password.

### F12 decisions

Clipboard history exists since F3, matched in the root list as an aside
(7.14). Raycast's *Clipboard History* was read as the target: a command that
opens a view of its own, entries in sections by date, a *Type* filter, and
the selected entry beside the list with where it came from and what it is.
It keeps five kinds -- text, links, colours, files and images. F12 is that,
in two slices: everything but images, then images.

The decisions:

- **Entries leave the root list.** A *Clipboard History* row takes their
  place, as *Search Snippets* took snippets' (F11). The `Clipboard` aside is
  gone; files are the only one left. The history is still refreshed by
  nobody: the view reads it each time it opens, on the background executor,
  so the recorder no longer tells the launcher anything.
- **The view is *Search Snippets*' shape,** with nothing new in the
  vocabulary but the swatch. Sections by when the entry was copied, with the
  same periods (the helpers moved to `lumen_core::dates`); *All Types*, then
  *Text*, *Links*, *Colors* and *Files* as a scope; the pane is the content
  verbatim, then *Source*, *Type*, what the type has to say (*Characters*
  and *Words*, *Hex*, a file count) and *Copied*. The session filters, for
  snippets' reason: every word typed must appear in the entry.
- **Only what the system distinguishes is stored.** Schema 2 adds a `kind`
  column, `text` or `files`, and uniqueness moves to the pair, since a path
  copied as text and as a file paste differently. A link or a colour is text
  that reads as one, decided in `clipboard::kind` when the entry is read:
  a rule improved later applies to the whole history, with no migration.
  The rules are strict -- the whole entry, on one line, is a link or a
  colour -- because a false positive puts a swatch on something that is not
  one, and a false negative only costs a plainer row.
- **Files are captured as `CF_HDROP`, before text.** A program that copies
  files may name them in text too, and the files are what was copied. They
  are stored as paths, one per line, and pass the privacy layer like text.
  Pasting one offers `CF_HDROP` and the paths as text, so Explorer takes
  the file and an editor takes its path.
- **The clipboard is read once per change, patiently.** Found on the first
  try-out: copies from Notepad stopped being recorded. Reading files and
  then text opened the clipboard twice, and every monitor reads it the
  moment it changes -- Windows' own history among them, which makes the
  copying program render its delayed text while holding it. The second
  opening lost that race and the copy was dropped without a word. Now
  both are read in one opening, retried for up to 200 ms on the watcher's
  own thread, and a read that still fails is logged.
- **A colour is drawn as itself: the one exception to I-5.** `Icon::Swatch`
  carries a colour that is the row's *content*, not a choice about how the
  row looks, so no theme token could hold it. Its size and radius are
  `layout::view::SWATCH_*`, and its border is the theme's, so a colour close
  to the background still shows its edge. An unparseable value draws
  nothing. No other colour enters the vocabulary through this door.
- **`Enter` pastes, and the paste stays on the clipboard.** Raycast's
  behaviour, and the reason to open the view at all. It is F11's paste, not
  restored, and marked with `ExcludeClipboardContentFromMonitorProcessing`
  so the recorder does not record it again with the target as its source;
  the entry is moved to the top instead, keeping its own. *Copy to
  Clipboard* does the same without the keystroke. `Platform::set_clipboard`
  is the new method (I-9), and `Paste` gains `files` and
  `keep_out_of_history`.
- **The action panel is *Paste*, *Copy to Clipboard*, *Open* and *Delete
  Entry*.** *Open in Browser* for a link, *Open File* for a single file.
  Deleting one entry does not ask, as in Raycast: it is one paste, and it
  is gone from a history that forgets on its own anyway. Clearing the whole
  history is not offered yet.
- **Measured, slice 1.** With `scripts/measure-idle.ps1`, 60 s hidden:
  0.009 % CPU, 73.6 MB working set, 89.0 MB private, 31 threads -- F11's
  numbers within noise, and no thread added: the watcher's was already
  there. Binary 18,233,344 bytes, **+66 KB** over F11's close. Cold start,
  five alternating runs each: F12 462 ms median, `main` 467 ms. Both are
  over the 400 ms budget, so that breach predates F12 and is its own
  problem, not this phase's.
- **Images are kept as files, named by the hash of their pixels.** Schema
  3 adds `width`, `height` and `bytes`; the row's `text` is the hash, and
  `clipboard-images/` beside the database holds `<hash>.png` and a
  thumbnail at the icon resolution. A history of screenshots as BLOBs is a
  database that grows by megabytes per copy and a WAL that rewrites them.
  The hash is of the pixels, not of the PNG, so one picture copied from two
  programs is one entry, and uniqueness is the existing `(kind, text)`.
  Files are removed by a sweep after rows go -- pruned, deleted or cleared
  -- rather than per row: one pass is right however they went, and it also
  collects what a crash between a file and its row left. A file younger
  than a minute is never swept, since its row may be about to be written.
- **A picture is read after files and text.** An office program copies a
  paragraph as text and as a picture of it; the text is what was copied.
  `CF_DIBV5` when there is one, for its transparency, else `CF_DIB`, copied
  out while the clipboard is open and decoded after it is closed. The DIB
  parser is pure and tested: 24 and 32 bits, `BI_RGB` and `BI_BITFIELDS`,
  both row orders, the masks some programs repeat after a V5 header, and a
  32-bit alpha that is zero everywhere read as opaque. A palette or 16-bit
  picture is refused rather than guessed at. Pictures past 40 megapixels
  are not recorded. Only the source is checked for privacy: a picture has
  no text to recognise a secret in.
- **Encoding happens off the UI thread and outside the lock.** The
  recorder writes the PNG and the thumbnail on the background executor,
  then takes the history's lock only to insert the row, so the view is
  never stalled behind a screenshot being compressed. Pasting one writes
  `CF_DIBV5` and the stored PNG as `PNG`, which a browser or an office
  program reads in preference; Windows makes `CF_DIB` and `CF_BITMAP` from
  the V5 for everybody else.
- **A picture in the vocabulary is the host's alone.** `Icon::Image` and
  `ItemDetail::image` are paths, and both are `#[serde(skip)]`: a view an
  extension sends cannot carry one, so the host never reads a file because
  an extension named it (I-7), and a test says so. The icon square and a
  fixed-height box at the top of the pane are reserved while it loads
  (I-10).
- **Pictures are drawn from a bounded cache that is emptied on hide.** The
  icon cache keeps everything forever, which is right for a few hundred
  small icons and wrong for screenshots. `ImageCache` decodes off the UI
  thread -- thumbnails at the icon resolution, the pane's picture fitted
  within 1024 pixels -- keeps 24 MB of decoded pixels, evicts the least
  recently drawn past that, and frees each evicted texture from GPUI's
  atlas with `drop_image`. The launcher empties it when it hides and when a
  view is left, deferred to when no window is being drawn, so a picture is
  never held while nobody is looking (the idle budget).
- **A picture can be pasted as where it lives.** Asked for on the first
  try-out, by someone whose terminal reads a picture by its path. The
  original path is rarely on the clipboard: a screenshot is only pixels,
  and a file copied in Explorer is already a *File* entry that pastes its
  path. A browser, though, puts HTML beside the picture, and its `<img src>`
  -- resolved against `SourceURL` when relative, `http` and `https` only --
  is where the picture lives. Schema 4 keeps it as `origin`, when it passes
  the same secret checks as text. *Paste URL* and *Copy URL* offer it, and
  a picture without one offers *Paste File Path* and *Copy File Path*, to
  Lumen's copy. `Enter` still pastes the picture: text beside the picture on
  every paste would have a chat program paste the path instead.
- **A picture is fitted by absolute height, and a thumbnail is its centre.**
  Found on the first try-out. GPUI's `img` takes the picture's aspect
  ratio, and with `size_full` it kept the pane's width and let the height
  follow: a tall screenshot ran down the pane under the metadata. The box
  now has an absolute height and clips. A row's thumbnail is the picture's
  centre square, cropped when it is decoded, because a tall picture fitted
  whole in the icon square was a sliver.
- **The recorder waits for the clipboard to settle.** Found while testing
  slice 2, and older than it: a .NET program's copy failed, in *that*
  program, while the daemon ran. OLE copies in two steps -- announce, then
  flush -- and reading at the announcement asked the program to render
  while it was retrying to open the clipboard the watcher held. The watcher
  now reads 100 ms after the last change, as Ditto does, and names the
  source at the change. Measured with .NET copies: 9 of 18 failed before,
  1 of 36 after, 0 of 18 without the daemon. Reduced, not proven gone.
- **A *Clipboard History* page in settings: the limit, and clearing.**
  The limit is `clipboard.retention.max-entries`, written through the
  `section` writer like every other page, from 1 to 2000 -- the view reads
  the whole history when it opens, so the cap is the view's, and a hand
  edit past it is clamped. It now applies live: of the `clipboard` section
  only `enabled` waits for a restart, because only it decides whether the
  listener starts. Lowering the limit deletes the oldest entries past it at
  once, which the page says before it is saved. *Clear History* is not a
  setting and is not in the file: it counts what would go, then a dialog
  says it -- the entries, the pictures and their size on disk, that it
  cannot be undone, that the clipboard itself is untouched and recording
  goes on -- and only Enter on it clears.
- **Measured, slice 2.** 60 s hidden: 0.004 % CPU, 73.8 MB working set,
  87.6 MB private, 31 threads -- slice 1's numbers within noise. Binary
  19,451,392 bytes, **+1.2 MB** over slice 1: PNG encoding, resizing and
  SHA-256 are now linked in. Still half the installed-size budget, but it
  is the largest step of any phase and is written down as one. The script
  measures a launcher that was never opened, so it does not show what the
  emptied cache saves after a session with pictures; that was not measured.

### F13 decisions

Two Raycast extensions were read as the target: *World Clock* and
*Weather*. The World Clock extension Lumen ships (F4) knows seven zones by
their Windows names, and the question it cannot answer is the one this phase
exists for: *how is that place -- a city, a country, a continent -- compared
with mine?* For a remote meeting, an event somewhere else, a colleague in
another zone. Time and weather are one concept over one set of places, not
two features.

The decisions:

- **The World Clock extension is retired.** It was F4's first example,
  and two *World Clock* rows in the root list would be one too many. New
  installs get *Characters* alone as the protocol's example; a copy already
  installed is the user's and is never deleted, so it stays until its
  folder is.
- **One catalogue of places, two views.** *World Clock* and *Weather* are
  built-in view sources over the same places and the same favourites.
  Favouriting a city in one puts it first in both: one list, *My Places*.
- **A favourite is kept by name, not by number.** `places.json` in the
  data folder lists keys -- `CA/British Columbia/Vancouver` -- because a
  city's index in the catalogue moves whenever GeoNames is read again. A
  key the catalogue no longer knows is skipped in the list and kept in the
  file, so a city renamed upstream is not silently forgotten by the next
  save. *My Places* follows the list's rule: filtered by what is typed and
  by the pill, in the order added, and never shown twice.
- **The places are bundled; only the weather is fetched.** GeoNames'
  `cities15000` (CC BY 4.0: every city over 15,000 people, 34,152 of them
  in 356 zones), reduced to name, ASCII name, country, region, IANA zone,
  coordinates and population, and bundled compressed: 2.2 MB of text, 836 KB
  in the binary through `flate2`, already in the build through `png`.
  `scripts/make-places.ps1` rebuilds it from GeoNames; the file is
  committed, not downloaded at build time. Finding a city, its zone and its
  coordinates never touches the network, so the clock is wholly offline and
  the weather needs no geocoding service.
- **The zone rules are jiff's bundled IANA database.** Already in the
  binary since F9 (`tzdb-bundle-platform`, 2026c at this writing), so it
  costs nothing, and the user's zone and the remote one come from the same
  source and cannot disagree. Windows' ICU copy is fresher -- Windows Update
  keeps it -- and was declined: it is a second source for the same answer,
  and bindings to reach it. Governments change their rules several times a
  year, so the database is refreshed with `cargo update -p jiff-tzdb`,
  independent of jiff's exact pin; `Cargo.lock` records which. The pane
  does not show the version: jiff does not expose it, and `jiff-tzdb`'s
  constant would name the bundled copy on a Linux build that reads the
  system's instead. The current time is the system clock, which is as
  accurate as the taskbar's.
- **Ranked by population.** There are two Vancouvers, several Madrids and
  a Los Angeles in Spain. The largest place of a name comes first; the
  subtitle names the region and country so the others can be told apart.
- **Every word narrows, against any level.** A place is a city, its
  region, its country and its continent, and every word typed must match
  one of them: `california` lists Los Angeles and San Diego, `america
  canada van` finds Vancouver, `bogota` finds Bogotá. Matching ignores
  accents, and countries answer to their common aliases -- `usa`, `us`,
  `uk`, `korea` -- from a small hand-written table.
- **The list is at the level the words name.** A continent alone lists its
  countries, one row each with its city count and its range of offsets;
  a country lists its cities, largest first; anything else lists cities.
  Thirty thousand rows are virtualised cheaply and read by nobody, so the
  level is what keeps `europe` useful.
- **Drill-down is a pill, and the pill is part of the query.** `Enter` on
  a country or continent row puts it in a pill before the search field --
  `Europe › Spain` -- and clears the text for a filter within it. `Escape`
  clears the text first, then takes one level off the pill, then leaves the
  view; `Backspace` in an empty field takes one level off, as it takes the
  argument pill off in the root list (F6). Words typed at the top still go
  straight to a city, with no pill. The pill and the words are one state,
  so they cannot disagree. The vocabulary gains what this needs, and
  nothing else: `List::path`, the pill's levels, drawn by the host, and
  `Event::Back`, sent when a level is taken off.
- **A row answers the comparison.** The icon is the country's two letters
  (`Icon::Text`, as in F9), the title the city, the subtitle region and
  country; on the right their time, the difference from the user's zone and
  *Yesterday* or *Tomorrow* when their date is not the user's. A country
  with several zones shows its range -- `UTC−4 … −10` -- not one time.
- **Two row layouts, a setting, not a guess.** *Difference first* leads
  with `2 h behind`; *Time first* leads with `09:14` and dims the
  difference. Which reads better is a matter of feel, so it is a live
  setting (7.33) the user can switch while trying both, not a decision made
  here. The difference is written in words (`2 h behind`, `Same time`) or
  signed (`−2 h`), also a setting. Each layout has fixed columns: switching
  it changes the row, and nothing moves while results change (I-10).
- **Working-hours overlap is optional, and only with *Difference first*.**
  Off by default. On, the same hours are assumed there as here -- nine to
  five is nine to five in Vancouver too -- and the row shows when both
  days meet, as a span of the user's own day (`12:00–17:00`), or *No
  overlap*: the column is there either way. Their day is tried on either
  side of the user's midnight, so a long day finds Seoul's morning in
  Santo Domingo's evening. A row holds three accessories
  (`MAX_ACCESSORIES`), so with the overlap on the day leaves its tag and
  joins the time (`01:14 Thu`) rather than one being dropped. The pane
  says it in words: *Both at work 12:00–17:00 your time*.
- **The *World Clock* settings page.** Row, difference, overlap and
  working hours, written to the `clock` section only, through `section`'s
  rules. `Select` has no disabled state, and one page did not justify
  adding it to every dropdown: with *Time first* chosen, the overlap row
  says *Only with Difference first* in its place and Tab passes it by. The
  hours are `HH:MM`, 24-hour, as written in the file whatever the system's
  clock; ones that do not read as a day are refused with why, and hours
  not shown are kept as they were rather than judged. The next World Clock
  opened reads what was saved.
- **The pane is the whole answer, whatever the layout.** The exact time and
  date, the difference, the UTC offset and zone abbreviation, the next
  daylight-saving change, sunrise and sunset (computed from the
  coordinates).
- **Home is the system's zone, and a city is confirmed once.** The
  difference is always against the system clock and zone, which needs no
  answer from anybody. Weather needs a home city: suggested from the system
  zone, confirmed once, never read from Windows' location service, which
  asks for a permission and knows more than Lumen needs.
- **The root list answers the clock, offline.** `time in vancouver` and
  `vancouver time` are the time there, how far it is and its offset. A
  time is converted: `3pm vancouver` is three in Vancouver's afternoon, in
  the user's zone; `3pm in vancouver` is three in the user's, in
  Vancouver's; `15:00 vancouver in tokyo` goes from one to the other, and
  names both days when they differ. A time needs `am`, `pm` or a colon, so
  `3 vancouver` is not one, and a place needs three letters, so `time in
  v` answers nothing. The place is the city the view would list first.
  These are answers (7.9), costing what the calculator does.
- **The places are read when time is first asked about, not at start.**
  The root list answers from the catalogue only if it is already in memory
  (`places::loaded`); a question about time before then has the launcher
  read it on the background executor and rank again when it is there, as
  F9 does with a table of rates. The keystroke is never held (I-3), and a
  daemon nobody asks the time of never holds 34,000 cities. Times are
  written by the platform, as in the view, through a function the launcher
  hands `RootSearch`, since `core` calls no system itself (I-1).
- **A clock glyph and a `Clock` kind.** The command and the answers share
  a fifteenth glyph, on the set's grid (7.23): a face and two hands, which
  the timer's stopwatch is not.
- **The root list answers the weather without waiting for it.** F9's
  pattern: `weather in vancouver`, `weather vancouver` and `vancouver
  weather` are recognised only when the city is an exact match, so typing
  it asks nothing until it is whole. Cached (15 minutes), the answer shows
  at once; otherwise the row appears at once, its height reserved, saying
  *Loading…*, one request goes out in the background and the row fills in
  when it lands. The keystroke is never held. The word *weather* is
  required: typing a city alone never sends anything anywhere.
- **Weather is Open-Meteo.** Keyless, structured, WMO condition codes,
  current, hourly and daily in one request by coordinates, sunrise and
  sunset included. The screenshots the phase was designed from are
  wttr.in's, which is rate-limited and returns prose. The moon's phase is
  computed, not fetched. A city's forecast is cached for 15 minutes and
  fetched only when asked for: nothing is scheduled, so the idle budget
  does not move.
- **The weather report has two layouts, a setting.** *Report*, the
  default: `Enter` on a city opens its report at full width, as in the
  screenshots -- the current conditions, then details, then the daily
  forecast -- and `Escape` returns. *Pane*: the report beside the list,
  with a compact forecast (`Thu ↑16 ↓9`), designed rather than squeezed.
  The data is the same; only the drawing differs.
- **A forecast is asked for when typing settles, a few at a time.**
  Opening the view asks at once for the home city and *My Places* that are
  not fresh, in one request: Open-Meteo answers for several coordinates at
  once. Typing asks nothing per keystroke: each change is a generation,
  the driver waits 400 ms and reports back, and only the latest generation
  asks -- for the first eight cities listed, again in one request. A row
  says *Loading…* while it is out, and a failure (*Offline*,
  *Unavailable*) is not asked again by itself, only by *Refresh*. Countries
  and continents have no weather of their own; they are levels to go into.
- **Weather is fetched by the coordinates the catalogue has**, kept in
  metric and converted when written, so a change of units needs no
  request. The cache is `weather.json` in the data folder, the newest 64
  places, merged by whoever fetched -- the view or the root list -- after
  reading the file again.
- **A city is named whole before the root list asks anything.**
  `Catalogue::named_exactly`: the city the list would put first, when every
  word typed is a whole word of its name, its region or its country.
  `weather in vancouver` and `weather in london canada` ask; `weather in
  van` and `weather in vancouv` do not. A failed city is not asked again on
  every keystroke, and one city is asked at most once per showing.
- **One home city for both views, and the clock never lies.** Found after
  slice 7: Weather's home could be changed and World Clock's could not.
  *Home* had been one word for two things -- where the user lives, which
  Weather leads with, and what their clock says, which every difference is
  from and which no setting can change. So home is one value,
  `places.home`, edited from either settings page, and World Clock applies
  one rule: a home that keeps the system's time *now* names *Your time*
  (Santiago de los Caballeros keeps Santo Domingo's), and one that keeps
  another is a row of its own under it -- *Home*, with how far it is --
  rather than a name over a clock that is not its. By default both are the
  zone's largest city, so the two views agree; when they cannot, the clock
  says why instead of hiding it. A home chosen as `weather.home` before
  this is still read, and moved by the next save.
- **Home is the largest city in the system's zone until a setting names
  another.** The design said confirmed once; a first-run question is a
  dialog the view does not have, so the *Weather* settings page (slice 7)
  is where it is chosen, and the zone's guess is right for most people.
- **Attribution.** Open-Meteo's data and GeoNames' are CC BY 4.0: the view
  says *Forecasts by Open-Meteo* over the home city, and the README names
  both.
- **The report is a list, entered as one more level of the pill.** The
  screenshots are a list -- a row for now, rows of details, a row a day --
  so *Report* draws one in the list's place, with no new vocabulary: the
  city joins the pill (`Europe › Spain › Madrid`), and `Escape` or
  `Backspace` take it off exactly as they leave a country. Typing in a
  report searches for another place, as the screenshot's field says.
  Opening one empties the field and the search behind it, so coming back
  never shows results for words no longer on screen. `weather in ...` from
  the root list opens the report whatever the layout: one city is what
  was asked for. Every row of a report acts for its city.
- **The pane is the report, compact.** The same data beside the list,
  following the selection: the selected city is asked for when the
  selection settles, as typing is. The week is a line a day -- `Thu
  ↑16° ↓9° Light rain` -- not the report's rows squeezed. In the pane
  `Enter` copies, since the pane already shows what the report would.
- **The *Weather* settings page: units, layout, home.** Written to the
  `weather` section only. Home is *Automatic* -- the zone's largest city
  -- or one of *My Places*: searching 34,000 cities is the view's job, and
  a settings page that read the catalogue to list a few would hold it in
  memory for nothing. The choices are labelled from their keys, so opening
  settings never reads it. A home no longer among *My Places* stays a
  choice of its own rather than being lost by the next save. Units apply
  to the root list's answers live; the view reads the section when it
  opens (7.33).
- **Nine condition glyphs, and a glyph per detail.** Clear by day and by
  night, partly cloudy, cloudy, fog, drizzle, rain, snow and storm, on the
  icon set's grid (7.23). The details were first left as labelled values
  with no glyph, to keep the set small; found on the first try-out, the
  icon square they reserve (I-10) read as icons that failed to load. So
  each detail has one, as in the screenshots: five new ones -- feels like,
  UV, pressure, visibility, coordinates -- and the sun, the moon and the
  cloud reused from the conditions. An empty square that looks broken is
  worse than five more files. The screenshots' glyphs inside the values
  (sunrise, sunset) are not copied: an accessory is text, and `↑`
  and `↓` say it.
- **Left open, not built:** a time-zone converter for a whole meeting
  (several people, several zones); hourly weather beyond the report's day;
  alerts; a *My Places* dashboard -- the favourites alone, with time and
  temperature on one row -- asked for and deferred. Each is a field in the
  pane or a view of its own, not a redesign.
- **Updating the time zones without a new Lumen is a door left open.**
  Asked for: the rules change between releases, and a user should not have
  to wait for one. jiff can read a zone database from a folder at run time
  instead of the bundled one, so a settings button that fetches a newer
  one is possible. What it needs first: a source of *compiled* zone files
  (IANA publishes the rules as source), a check of what was downloaded
  before it is trusted, and a fall back to the bundled copy whenever the
  fetched one is missing or unreadable.
- **The slices,** one acceptance criterion each:
  0. Browsing -- the words, the levels, the pill, *My Places* -- moved to
     `places::browse` before Weather, so the two views share it rather
     than copy it.
  1. *World Clock* opens a view over the bundled catalogue: cities, the
     levels and the drill-down pill, both row layouts and both difference
     formats (from `config.json`), and the pane.
  2. *My Places*: favourites, first in the list, filtered with it.
  3. The clock in the root list: `time in vancouver`, `3pm vancouver`.
  4. A *World Clock* settings page -- layout, difference format, overlap,
     working hours -- and the overlap column.
  5. *Weather*: the list, Open-Meteo, the cache, and `weather in
     vancouver` in the root list.
  6. The weather report, in both layouts, and the condition glyphs.
  7. A *Weather* settings page -- units, layout, home city.

  The idle baseline is measured before slice 1, and again after 5 and 7.
- **Measured, before slice 1.** With `scripts/measure-idle.ps1`, 60 s
  hidden: 0.002 % CPU, 73.3 MB working set, 87.4 MB private, 31 threads.
  Binary 19,557,376 bytes.
- **Measured, slice 1.** Binary 20,542,464 bytes, **+985 KB**: the places
  are 836 KB of it. A render of the view -- finding the places and building
  up to 200 rows -- takes 1 to 1.5 ms in a release build for most queries
  and 2.5 ms for `a`, the worst there is, which matches 30,000 cities.
- **Measured, at the close.** 60 s hidden: 0.004 % CPU, 73.6 MB working
  set, 87.3 MB private, 31 threads -- the baseline's within noise, and no
  thread added: nothing is scheduled, and the places are read only when
  asked for. Binary 20,922,880 bytes, **+1.37 MB** over the baseline: the
  places are 836 KB of it, Weather and its nine glyphs 272 KB, World
  Clock's slices 2 to 4 the rest. The idle measurement after slice 5 was
  folded into this one: Weather schedules nothing either.

### F14 decisions

Raycast's calculator is SoulverCore, the engine of the Soulver app: a closed
Swift framework for macOS, so its behaviour is the target and none of its
code. What it answers in the root list, as typed: `sqrt(2)^pi / log(1000) *
sin(45 deg)`, `5 km in mi`, `25% off 399`, `days until 25 dec`, `100 usd +
20 eur in cad`, and a *Calculator History* of what was used.

The decisions:

- **The hard part is still declining.** F3's calculator banned every
  letter, which kept `code` and `7-zip` out of it for free. Units,
  functions and phrases bring letters in, so the gate becomes a recogniser
  that classifies a query -- arithmetic, conversion, percentage, date --
  or declines it, and a parse that leaves an unknown word declines. A
  conversion needs an operator or an explicit `in`, `to` or `as`: `25m` is
  still a timer's (F8) and `25m in s` is the calculator's. A corpus of
  queries other answers own -- applications, timers, currencies, times,
  weather -- is a test, written before the first unit.
- **Our own engine, not a crate.** `fend` and `numbat` were read. Both
  evaluate far more than this needs, and both lose the gate: thousands of
  unit names, single letters among them, are thousands of ways to steal an
  application's row. Neither reads `25% off 399` or `days until 25 dec`, so
  that layer would be written anyway, and an arbitrary-precision
  `2^10^7` is a keystroke that does not come back. The recursive-descent
  parser F3 started grows instead, in `lumen_core::calc`; the unit table is
  data, tested like data.
- **A value has a unit.** A number, or a quantity: a magnitude, a unit, and
  the unit's dimension. `sin(45 deg)` works because a degree is an angle.
  Units are a table -- names, symbols, a factor to the base, and an offset
  for temperature only -- so a new one is a row and a test. Symbols are
  case-sensitive (`Mb` and `MB`, `m` and `M`), names are not. SI and IEC
  data units are apart: `GB` is 10⁹ bytes and `GiB` 2³⁰.
- **`%` is a percentage.** See 7.39.
- **Dates are jiff's,** already in the build. `days until 25 dec`, `weeks
  since 1 jan`, `today + 3 weeks`. A date without a year that has passed is
  the next one. Numeric dates (`12/05`) are declined: which is the day
  depends on where the reader lives. Today is passed in, so tests do not
  depend on the day they run, and read only when a date is typed. A day
  moves by the calendar -- `31 jan + 1 month` is the end of February --
  and only by whole days, weeks, months or years: a day has no hours.
  Spans of the calendar are written as words, `85 days`, `1 week`,
  because that is how a date's answer is read. A weekday is never today:
  `until friday` on a Friday is a week.
- **Numbers are `f64`, rounded where they are written.** Ten significant
  digits, trailing zeros dropped. Input is capped, nesting is capped,
  `!` stops at 170, and a result that is not finite is no answer. This
  runs on every keystroke, inside the 8 ms frame.
- **History is what was used.** F10's rule: an entry is kept when a result
  is copied or pasted, not as it is typed, or history would be `1`, `12`,
  `12*` on the way to `12*7`. `calculator-history.json` in the data folder,
  the newest first, a repeat moved to the top, at most
  `calculator.history-limit` entries -- 20 by default, `0` for none.
- **Currencies are a dimension, from the table in hand (last slice).** A
  currency's factor is the cached table's rate, so `100 usd + 20 eur in
  cad` and `25% off 399 usd in eur` are one grammar with `5 km in mi`.
  The *Currency* view stays: browsing every currency is not one answer.
  Which words are currencies is the static catalogue, so the gate's verdict
  never depends on whether a table is loaded; only the value does. The
  calculator asks for a table exactly as F9's root answer does --
  `wants_rates`, once per showing, nothing held -- and replaces that
  answer rather than doubling it; F9's root tests move to the calculator
  and pass there before the old path goes. A table is used until it is
  due, because the services publish once a day and asking sooner returns
  the same one. The row always dates its rates. With no table it says
  *Fetching exchange rates…*; if the fetch fails it falls back to the
  table kept, dated, and with none it says it could not fetch them.
- **Money, as built.** `Value::Money` is not a unit: the parser converts
  the right side into the left's currency, through the table, before
  `+`, `-` or `/` sees two, and with no table it finishes the parse one
  for one and marks the answer pending, so the row can say why it has no
  number. One currency throughout -- `25% off 399 usd` -- needs no table
  and says none. The provider declines money and the root list answers
  it, because the root list holds the table; `calc::answer` is the one
  engine for both. The row is the calculator's (`Kind::Calculation`), so a
  copied answer in money is kept in its history like any other, and it
  still copies the number alone, as F9 did. Its subtitle is what was
  typed and the rates' date -- `10 usd in cad · rates of 28 Sep 2026` --
  where F9 wrote both sides out, which `100 usd + 20 eur in cad` has no
  room for. A conversion into the same unit or currency restates a value
  and is not an operation: `10 usd in usd`, as F9 declined it, and `5 km
  in km` likewise. A failed fetch now reaches the launcher
  (`Launcher::rates_failed`), and asking again clears it. A currency the
  table has no rate for declines the query rather than half-answering it.
- **The slices,** one acceptance criterion each:
  0. This entry, and 7.39.
  1. Measurement first: the calculator in the benchmark, and the corpus of
     queries it must decline, passing against F3's calculator.
  2. The engine moved to `calc`, behaviour unchanged.
  3. Functions and constants: the screenshot's `0.700197505`.
  4. Percentages, and `%` changes meaning.
  5. Units: length, mass, time, temperature, data, angle; `in`, `to`, `as`.
  6. More units, as data: frequency, power, energy, speed, area, volume,
     pressure.
  7. Dates.
  8. History, kept.
  9. The *Calculator History* view and a *Calculator* settings page.
  10. The answer drawn as the screenshot's card, expression and result.
  11. The close: measured against the baseline.
  12. Currencies inside an expression.
- **Measured, before slice 1.** With `scripts/measure-idle.ps1`, 60 s
  hidden: 0.002 % CPU, 74.1 MB working set, 86.0 MB private, 32 threads.
  Binary 20,970,496 bytes. Ranking (`cargo bench -p lumen-core`, saved as
  `f14-before`): 451 µs for a typical query over 5,000 entries.
  The calculator alone (`--bench calculator`, the same baseline): 15 ns to
  decline, 1.7 µs to answer `(1+2)*3/4-5`, 6.6 µs for the longest hostile
  input. F14 holds every query, hostile or not, under 50 µs: under 1 % of
  the frame.
- **A corpus found an overlap that was already there.** Slice 1's corpus
  of owners (`tests/calculator_corpus.rs`) failed on its first run, on
  F13's own example: `15:00 vancouver in tokyo` is also a fifteen-minute
  timer named *vancouver in tokyo*, since a clock reading is a duration to
  F8. Not the calculator's to settle; the corpus states it exactly, so the
  fix, when it comes, fails there and removes the row.
- **Numbers are written as the region writes them, unless asked not to.**
  Raycast has no setting of its own: its calculator follows the system's
  number format. Lumen does the same by default -- `calculator.number-format`
  is `system` -- and `plain` keeps a point and no grouping, as F3 wrote
  them; the *Calculator* settings page switches it. The separators come
  from `Platform::number_separators` (`GetLocaleInfoEx` on Windows; plain
  on Linux until F5), read at start and when the setting changes, and set
  in core with `calc::set_separators`: core reads no regional setting
  itself (I-1). They apply to what is read as well as what is shown, since
  `2,000` is two thousand in the United States and two in Spain. A
  grouping separator counts only between groups of three, so `1,5` to an
  American is declined, not guessed at. A result is shown grouped --
  `1,800,000` -- and copied without grouping -- `1800000` -- because the
  number field it is pasted into reads that; Raycast copies the grouping,
  and its users ask it not to. Snippets paste the copied form. Money keeps
  F9's own writing. A change in Windows' region applies on the next start,
  as Raycast's does. It costs a second write per answer: about 2 µs for
  arithmetic and 9 µs for the longest hostile input, from 1.2 and 4.8.
- **That overlap is kept, on purpose.** After the try-out: a clock reading
  and a place are two fair questions, and the list already lets the user
  choose. Both rows are offered -- the timer and the time there -- and
  the corpus lists the query as an intended overlap, not a known one.
- **Measured, at the close.** The baseline and the close measured again
  side by side, twice each, 60 s hidden: baseline 0.009 % CPU, 74.1--74.4
  MB working set, 85.6--86.9 MB private; the close 0.000--0.011 % CPU,
  74.2 MB working set, 86.5--87.8 MB private; 32 threads both. One tick
  either way is noise -- the close read 0.000 % once, under the
  baseline's 0.009 % -- and nothing is scheduled: the history is written
  on use, and the rates are fetched only when money asks for them.
  Binary 21,165,568 bytes, **+195 KB**. Ranking (`--bench search`)
  within noise of `f14-before` at every size. The calculator, per
  keystroke: 42 ns to decline an application's name, under 1.2 µs for
  arithmetic, a conversion, a percentage, a date or the screenshot's
  expression, 2.6 µs for `100 usd + 20 eur in canadian dollar`, and
  4.8 µs for the longest hostile input -- under a thousandth of the
  frame, and under the 50 µs held to from slice 1. Gesture and cold
  start were not measured again: neither path reaches the calculator.

### F15 decisions

Lumen's look was one look: the geometry was the project's, in constants, and a
theme could change colour and the type family and nothing else (AD-13). F15
opens the look to the user. The references were Raycast on Windows, Vicinae
and Asyar: a taller search field with larger type, taller rows, a filled
selection without a border, the subtitle beside the title instead of under
it, the footer's keys drawn as keycaps, and the empty list in sections. None
of that is a colour, and most of it is not even a number.

The decisions:

- **Two layers, chosen apart.** A *palette* is what a theme file is today --
  colours, appearance and the type family -- and keeps its schema, its folder
  and its config key (`theme`). A *style* is new: geometry and variants, in
  `styles/`, named by `style` in `config.json`. Either can change without the
  other, so a Raycast style with a Gruvbox palette is two settings rather than
  a file somebody has to merge by hand. A preset is a named pair. The resolved
  `Theme` the UI sees carries both, so a component still receives one value.
- **Tokens and variants, not layout.** A style sets numbers from a closed list
  of tokens and picks variants from a closed list of variants. It does not
  describe a tree. A layout language would make the UI tree exist twice
  (I-13), make every element name public API that I-12 then freezes, and
  leave window sizing, virtualisation and the budgets to whoever wrote the
  theme. Going from closed to open later is a migration; the other way is a
  breakage.
- **Bounded, and resolved like colours.** Every token has a range, and the
  relations the constants asserted at compile time (a row's radius under half
  its height, the inset inside the gutter, the icon inside the row, the type
  inside its line) are checked when a style loads. A value out of range is
  clamped and a broken relation reverts the tokens involved to the base, each
  with a warning, never an error: a style written for a later Lumen still
  loads (I-12). The base style is the constants as they were, so the launcher
  draws the same pixels until a style says otherwise.
- **Only the launcher's surface is themeable.** The search field, the rows
  and their icon, the section headers, the calculator's card and the footer --
  in the root list and in a view, which share the geometry by AD-07. The action
  panel, the forms, the overlay and the settings window keep their constants.
  A token is added when a surface needs one, not ahead of it.
- **Tokens name parts, not features.** AD-12 forbids a colour token named for
  a component, because components multiply. The launcher's parts do not: a
  new feature draws inside a row, it does not add one. So a geometry token
  names a part (`row.height`, `footer.height`) and a new token is safe by the
  same construction as a new colour: missing from old styles, filled from the
  base.
- **Rows stay one height.** I-10 holds within a style: a row is `row.height`
  whether or not it has a subtitle, in every row of every list, and a section
  header is `section.height`. Changing style may resize the window, which is a
  resize and not a re-creation (I-4).
- **Sections on the empty list only.** With nothing typed, the root list is an
  ordered list of sections from `config.json`, each a title, what it shows and
  whether its header is drawn. What a section can show is a closed set --
  `favorites`, `commands`, `applications` -- that grows in code. The default
  is Raycast's: *Favorites*, *Commands*, *Applications*. An item appears once,
  in the first section that claims it; an empty section is not drawn, header
  included; what no section claims is not browsed but is still searched.
  Typing collapses the sections into one ranked list, under a *Results* header
  that can be turned off: ranking by kind would put a strong match under a
  weak one for being in the wrong section, and AD-03's order is the part of
  the launcher that must survive any presentation.
- **The empty list is computed once.** It depends on no query, so it is built
  when the index, the frecency or the favourites change, and opening the
  launcher ranks nothing. It is not capped: every application and every
  command is browsable. Whether the typed list keeps `MAX_RESULTS` is decided
  by measuring it against a 2,000-entry index, not by assuming. `MAX_RESULTS`
  never protected the frame anyway: `Search::rank` scores and sorts every
  candidate before it truncates.
- **Favourites are an ordered list of ids.** Any application or command can
  be one, from the action panel or by its keys (`Ctrl+Shift+F`, and
  `Ctrl+Alt+Up`/`Down` to move one), bound in the keymap (I-14). An id whose
  item is gone is kept, not shown, so an application reinstalled comes back
  where it was.
- **Measured first.** Ranking had a budget test; drawing had nothing. The
  frame cost while typing is taken with gpui's own frame measurement
  (`ZED_MEASUREMENTS`) by `scripts/measure-typing.ps1`, before anything
  changes, and again at the close. gpui measured only the frames its frame
  callback draws, and while keys arrive most are drawn by the key handler
  instead, before it dispatches the next key: the first run counted 19
  frames for 492 keystrokes. A second patch measures that draw too (7.35).
- **The baseline,** `f15-before`, release build, the machine's own index
  (260 applications, 5,000 files). Typing: 523 frames over six summonses,
  p50 1.6 ms, p95 2.24 ms, max 6.1 ms, twice, against 8 ms. Idle: 0.000 %
  CPU over 60 s, 73.8 MB working set, 86.1 MB private, 32 threads. Ranking
  (`--bench search`, medians): the whole root pipeline 293 µs for an empty
  query at 2,000 entries, 345 µs for one character; the matcher uncapped
  within noise of capped at every size, 357 against 354 µs at 2,000 and
  5.43 against 5.34 ms at 20,000 -- the cap saves nothing measurable.

The style schema, version 1. Base values are the constants as F14 left them.

| Token | Base | Range | What it is |
| --- | --- | --- | --- |
| `window.width` | 720 | 560--1000 | The window's width; `window.width` in `config.json` overrides it |
| `list.height` | 352 | 200--640 | How tall the list grows before it scrolls, ending on whole rows; `window.visible-rows` overrides it |
| `window.radius` | 10 | 0--16 | The window's corner |
| `search.height` | 56 | 40--96 | The search field's row |
| `gutter` | 14 | 8--32 | The left edge text lines up on |
| `list.padding` | 6 | 0--16 | Above and below the list; the space under the last row is at least `row.inset` |
| `list.padding-bottom` | unset | 0--16 | Optional: the space under the last row, exactly. Unset, it is the larger of `list.padding` and `row.inset` |
| `row.height` | 44 | 28--72 | Every row (I-10) |
| `row.radius` | 7 | 0--18 | The selection's corner |
| `row.inset` | 7 | 0--16 | The selection's distance from the window's edge |
| `icon.size` | 26 | 14--40 | The icon's reserved square |
| `icon.gap` | 12 | 4--24 | Between the icon and the title |
| `subtitle.gap` | 8 | 4--16 | Between the title and an inline subtitle |
| `section.height` | 28 | 16--48 | A section's header |
| `footer.height` | 38 | 28--64 | The footer |
| `footer.icon-size` | 14 | 10--24 | The footer's mark, when `footer.icon` draws it |
| `keycap.height` | 20 | 14--32 | A keycap, when the footer draws them |
| `keycap.padding` | 5 | 2--10 | Either side of a keycap's label |
| `keycap.radius` | 4 | 0--8 | A keycap's corner |
| `keycap.gap` | 4 | 0--12 | Between the keycaps of one chord |
| `text.query` | 17 | 12--28 | What is typed |
| `text.title` | 13.5 | 11--18 | A row's title |
| `text.subtitle` | 12 | 10--16 | A row's subtitle |
| `text.accessory` | 11.5 | 10--16 | What a row says at its right edge |
| `text.section` | 11.5 | 10--16 | A section's header |
| `text.footer` | 11.5 | 10--16 | The footer |
| `text.card` | 22 | 14--48 | The calculator card's two values |
| `text.tracking` | 0 | -0.05--0.1 | Letter spacing in em, on the launcher's root and inherited by its text; keycaps take none (7.42) |
| `weight.query`, `weight.title`, `weight.section`, `weight.card` | 400 | 400, 500, 600 | The bundled faces; anything else snaps to the nearest. `weight.card` is the card's values (7.43) |
| `rule.width` | 1 | 1--2 | The line under the search field and over the footer (7.43) |
| `card.rows` | 2 | 2--4 | Rows the calculator's card takes, read as whole rows (7.43) |
| `card.label-gap` | 6 | 2--24 | Between each of the card's values and its label (7.43) |
| `actions.width` | 440 | 280--720 | The action panel's popover; never wider than the window less `actions.offset-x` either side (7.47) |
| `actions.offset-x`, `actions.offset-y` | 8 | 0--64 | In from the edges `actions.anchor` names, across and down; right or down from the centre on a centred axis. The window keeps `actions.offset-y` clear above and below the popover (7.47) |
| `actions.max-rows` | 7 | 3--12 | The popover's rows shown before it scrolls, read as whole rows; its height is computed from them (7.47) |

| Variant | Values (base first) | What it changes |
| --- | --- | --- |
| `row.subtitle` | `stacked`, `inline` | Under the title, or after it on the same line |
| `selection` | `fill-border`, `fill` | Whether the selected row has a border |
| `footer.hints` | `text`, `keycaps` | `Actions  Ctrl+K`, or the keys drawn as keycaps |
| `keycap` | `filled`, `outline` | A keycap filled, or an outline the footer shows through (Raycast's) |
| `search.icon` | `none`, `search`, `logo` | Before the query: nothing, a magnifying glass, or Lumen's mark; at the rows' icon size and gap, so the query starts where their titles do |
| `footer.icon` | `none`, `logo` | Lumen's mark before the footer's status |
| `footer.count` | `shown`, `hidden` | *313 results*, where the footer has nothing else to say; none of the references shows it |
| `footer.escape` | `shown`, `hidden` | The *Dismiss* or *Back* hint for Escape; none of the references shows it |
| `card.header` | `hidden`, `shown` | A *Calculator* section header above the card, counted in the window's height (7.43) |
| `card.labels` | `plain`, `chips` | *Expression* and *Result* as muted text, or on a filled chip in `border.strong` |
| `card.divider` | `none`, `line` | A hairline in `border.strong` between the card's halves, broken around the arrow |
| `card.operators` | `plain`, `dimmed` | The expression's operators in the muted colour |
| `actions.anchor` | `bottom-right`, `bottom-left`, `bottom-center`, `top-right`, `top-left`, `top-center`, `center` | Where the action panel's popover stands between the search field and the footer (7.47) |

Two colour tokens joined the palette with them: `surface.footer`, behind the
footer (transparent in the base themes), and `border.window`, the window's
edge. The edge resolves from the palette's own `border.subtle` when it is not
set, not from the base theme's, so every palette written before it keeps one
colour for its edge and its separators.

The shipped themes and styles follow the releases unless edited
(`lumen_core::shipped`). They used to be written once and never again, so a
preset improved in a release never reached anyone who already had it -- which
is how Carbon's search icon first failed to appear. Lumen now keeps a
fingerprint of each file it writes, in `shipped.json`; at startup a file that
still matches gets this build's version, and one that does not -- edited --
is left alone. A file from before the fingerprints is left alone too, until
it equals a shipped version.
| `section.case` | `title`, `upper` | *Commands* or *COMMANDS* |

The icon's extraction resolution follows `icon.size`: three and a half times
it, never under the 96 pixels it was, so a style with larger icons does not
make them soft.

What was decided while building it:

- **The base style is the old look, pixel for pixel.** Checked by capturing
  the launcher from the baseline build and from each slice's -- the empty
  list, a search, the calculator's card, a search finding nothing -- and
  diffing at a threshold of one: identical, through slice 7. Sections change
  the default look on purpose after that; the style does not.
- **The root list is a `list`, like a view's (7.29).** A header is shorter
  than a row, so `uniform_list` could not hold one. The rows are a view's
  `Row`s, sized and revealed by the same arithmetic, so the root list and a
  view cannot drift apart (AD-07). The custom virtualiser kept in reserve was
  not needed: typing measured p95 1.96 ms on `list` against 2.24 ms on
  `uniform_list`.
- **The card stays above the list.** With headers drawn, a calculation reads
  card, *Results*, rows, as Asyar draws it. A *Results* section holding only
  the card draws no header.
- **`MAX_RESULTS` is gone.** Measured on the whole root pipeline: lifting it
  costs 6% on one character typed (345 to 366 µs at 2,000 entries, 922 to
  976 µs at 5,000), and a typical query does not move. The empty list, now
  cached, went from 293 µs to 10 µs at 2,000. The files aside keeps its cap
  of eight: that one is about what the list is for, not about time.
- **What *commands* means.** Everything in the index that is not an
  application: Lumen's own commands, scripts, quicklinks, presets, extensions'
  commands. Answers, files and fallbacks are never browsed and can never be
  favourites -- there is nothing to come back to.
- **Moving a favourite steps over the ones not drawn,** so an uninstalled
  application kept in the list never makes a move look like it did nothing.
- **The presets are measured, not eyeballed,** from the reference
  screenshots at 125 % -- edges and glyph heights scanned, cap heights turned
  into type sizes through Inter's 0.727 -- and named for what they are
  (Carbon, Graphite, Ink) rather than for the products they follow
  (AD-10's caution about other people's marks). Carbon follows Raycast,
  Graphite Asyar and Ink Vicinae. Graphite was first shipped as Raycast's:
  two of the reference screenshots had been taken for each other, and the
  preset was right for the one it was measured from. Carbon is measured from
  Raycast's, and brought the `keycap` variant -- Raycast outlines its keys --
  and a looser keycap rule: a keycap must be 1.2 times its label's type size,
  not a whole line of it, which Raycast's 18 pt caps around 13.5 pt keys are
  not. A test holds every preset to resolving exactly
  as written: none of its tokens may be clamped or reverted.
- **The Appearance page writes as you choose.** Every other page saves on
  Enter; this one has nothing to save, because seeing the launcher change is
  the point of choosing a look. An arrangement written by hand is shown as
  *Custom* and left alone.
- **Typography stays the palette's, and opens up.** Any installed family by
  name already worked -- gpui searches the system collection -- so what was
  added is a `fonts` folder for families that are not installed, read at
  startup, and `font_features` for a family's OpenType features. Raycast and
  Vicinae were checked rather than guessed: both set their interface in
  Inter, Raycast in a web view (its own engineering blog) and Vicinae
  bundling it since its pull request 1071. The difference on screen is the
  rasteriser, which no setting reaches. Letter spacing was the next lever,
  and gpui 0.2.2 had none; it was added as a patch (7.42).
- **No gap between sections.** Measured before adding a token for it: in
  Raycast the band a later header occupies is the first's, 40 screen
  pixels, and in Vicinae it is tighter. The extra air the captures seem to
  show is the first header sitting against the separator.
- **The size is the style's, the config can override it.** Measured against
  full-screen captures, all three references were about 6 % wider and 17 %
  taller than Lumen. Part of the height was a bug: the list's budget was a
  row count, so every section header cost a whole row, and the default list
  showed seven rows under one header where Raycast shows eight under two.
  The budget is now a height, `list.height`, still filled with whole rows
  (I-10). `window.width` and `window.visible-rows` in `config.json` became
  optional overrides, bounded to 560--1000 and 3--20: a person's screen is
  theirs to fit, and a preset should arrive the size it was measured at.
- **The settings window, redesigned after Raycast's.** Ten tabs no longer
  fitted 900 pixels, and each page drew its own label column. Now: a title
  bar; a sidebar with a search over the pages and an icon for each; and
  content in rounded cards of rows -- title and description on the left, the
  control on the right -- under section titles. Toggles replace checkboxes,
  dropdowns sit compact at a row's right edge. Measured from the reference
  at 1.22 screen pixels to the point: about 900 by 674, a 237-point sidebar,
  32-point page rows, cards 16 in from the edges, rows of 48 or 58.
  - **Palette and typography only.** Settings follows the active palette --
    colours, family, OpenType features -- so it always matches the
    launcher, and its geometry stays the project's (`layout::settings`). A
    launcher style is about a list; a form has far more parts than a style
    should own, and making them themeable would double the token surface
    for a window opened once a week.
  - **One set of components** (`settings_ui`): the card, the row, the
    section title, the toggle. A page composes them and draws nothing of its
    own geometry, so no page can drift from the others.
  - **The list editors are adapted, not copied.** Quicklinks, Translation,
    Timers and Shortcut Sheets keep a list and its editor; they take the
    cards' look inside the content column, under the sidebar's navigation.
    The forms inside them are unchanged, because the quicklink form is also
    the launcher's *Create Quicklink*.
  - **Built.** The nav replaces the tabs, with a search over the pages and
    `Ctrl+Tab` through them (keymap, I-14); Appearance, Calculator,
    Clipboard, Currency, Weather and World Clock are cards of rows, each
    field's old hint now its row's description; the list editors are a list
    card beside an editor card. Checked on screen in Carbon and in Daylight.
    The launcher was not measured again: nothing in its render path changed,
    and the one shared part, the dropdown, keeps its old drawing unless asked
    to be compact.
- **Test runs never touch the user's Lumen.** A capture script that swapped
  the theme in the real `config.json` and put it back had an escaping bug,
  wrote the file empty, and restored nothing; it was recovered from a copy
  some hours old. Since then `LUMEN_HOME` gives a daemon a folder of its own
  for `config`, `data` and `cache`, and a socket of its own, so a test daemon
  runs beside the installed one and reads and writes nothing of it. Scripts
  that check a look start one that way, and say at the end whether the
  user's config and daemon are as they were.
- **Measured, at the close.** Release build, both builds alternated. Typing:
  p95 2.00--2.01 ms against 2.24 ms. Gesture: p50 8 ms, p95 19 ms. Idle:
  0.002--0.009 % against 0.004 %, one to four 15.6 ms ticks either way;
  67.4--74.5 MB working set against 74.2; private memory unchanged at about
  88 MB. One thread more, 33 against 32: the styles folder's watcher, blocked
  until a file changes. Cold start 360--377 ms against 357--366 ms, a
  difference inside the spread between runs, under 400. Binary 21,371,392
  bytes, **+187 KB**.

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

*Superseded for the launcher's surface by 7.41.* The rest of the geometry --
the action panel, the forms, the overlay, the settings window -- still lives
here.

I-5 says no colour, spacing, radius or type size may appear literally in UI code.
I-11 says a theme may define only colour and type family. Both hold because there
are *two* token sources with different authors: colour and type family come from
the theme file, which anyone may write; geometry comes from `lumen_core::layout`,
which only the project may change. What I-5 forbids — a literal `px(38.)` buried
in a render function — stays forbidden either way.

One colour does not come from the theme: a copied colour's swatch in
*Clipboard History* (F12), because the colour is the content being shown,
not styling. It is confined to `Icon::Swatch`; see F12 decisions.

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

*Superseded by 7.45, which makes it a popover after all.*

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

*Superseded by F12:* the history is no longer matched in the root list at
all; it has a view of its own. The reasoning below is why it never joined
the index, and it still holds for files.

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

*Revised by F16 (slice 1).* Files are no longer ranked by the matcher on
a keystroke, so the cap and `files.limits` are gone. The file index holds
every entry the rules allow, up to a ceiling of 500 000 (`file_index::CEILING`),
and that number is a budget number too -- for idle memory, at about 30 bytes
of names an entry -- not for the frame. The frame is protected by the query
running off the UI thread, by a name scan that answers a million names in a
few milliseconds, and by one- and two-letter queries in root searching only
what was used ([`FILE-SEARCH.md`](FILE-SEARCH.md), "One or two letters
search what you use").

*Revised again by F16 (steps c3a and c3b).* The ceiling is gone. Every scope
is indexed to its end on disk, and memory holds a hot layer of names within
`files.hot-memory` (8 MB by default, 1 to 32): what was used, then folders and the shallowest files.
The budget is still idle memory's. `files.index-detail` (`complete`,
`balanced`, `compact`) decides only the word tables on disk, never the
ranking. The trade is root's alone: outside the hot layer, root finds an
entry by the start of one of its words, never inside one. *Search Files*
answers in two phases. The first is root's path at the view's limit, drawn
at once. The second scans the disk for the matches inside a word, is
cancelled by the next keystroke, and is appended below. So the view still
finds every entry ([`FILE-SEARCH.md`](FILE-SEARCH.md), "Step c3b").

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

*Revised by F16 (slice 1).* There are no asides left: the clipboard left in
F12 and the files leave now. The file index's rows are a *Files* section
appended after the results -- after every application and answer, before the
fallbacks -- filled when the index answers, still capped at eight
(`root::FILES_CAP`), and still never shown for an empty query. Its header
follows the results header.

*Revised by F16 (step 6d).* When the whole query is a path (`C:\Users\do`,
`~\Doc`), root lists that folder live instead: read from the disk off the UI
thread, shown alone in the *Files* section before the fallbacks, uncapped by
`FILES_CAP` (the listing has its own cap of 200), and never waited for (I-3).
See `FILE-SEARCH.md`, "Typed paths".

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

*Revised by F16 (slice 1).* The discount no longer applies; asides are gone
(7.21). Files arrive from another thread into a section of their own at the
end, so they never interleave with applications and a file never outranks
one -- not even `colonialismo`'s clear match. Accepted: a late row that jumped
above the selection would be worse than a perfect file match sitting lower.

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

The two Desktop folders are watched as well, without their subfolders,
because the index also lists applications whose installer left only a
Desktop shortcut (Modrinth is one). This has a known cost that was accepted
rather than fixed: `FindFirstChangeNotificationW` cannot filter by
extension, so *any* file created, renamed or deleted on the Desktop -- a
download, a screenshot, a temporary save -- signals a re-index, not just a
`.lnk` or `.url`. Coalescing turns a burst into one re-index, and a re-index
costs about 600 ms of CPU on a background thread with no visible change when
nothing Lumen lists moved. Removing the cost means moving the Desktop watch
to `ReadDirectoryChangesW`, which reports file names and could ignore
everything but shortcuts; that is the change to make if Desktop churn ever
shows up in a measurement.

The extra application folders (`apps.extra-folders`, F16 slice 7) are
watched the same way, each with its subfolders, on the same thread; a
missing one is skipped and at most 64 handles are waited on in all. Both
`Platform::enumerate_applications` and `Platform::watch_applications` take
the resolved folders (I-9, Linux stubs alike). The watch is started once
with the folders configured then; an edit re-indexes at once, but a new
folder is watched from the next start, since the thread has no stop and a
second one per edit would never end.

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

A second patch, in F15, measures nothing new and changes no behaviour: the
draw `Window::dispatch_key_event` makes before handling a key is wrapped in
the same `measure("frame duration", ...)` as the frame callback's, so
`ZED_MEASUREMENTS` sees the frames typing actually draws. It is draw only;
the frame callback's figure includes the present.

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

### 7.38 The Linux cross-check is paused until F5 resumes

I-2 asked for `lumen-platform-linux` to be cross-checked on every commit.
Run locally, the check builds a second copy of the dependency tree for
another target, and during F9 that filled the disk and cost the working
session. F5 is frozen, so the check was guarding a port nobody is working
on, at a cost paid on every commit.

It is paused, not dropped. It is not run locally, and the CI job is kept
but disabled (`if: false`), so turning it back on is one line. Until then
R-4 is guarded by I-1 alone: `lumen-core` still has no `#[cfg]` and no
direct system call, and review still holds it to that. The first thing F5
does is re-enable the job and fix whatever it finds.

### 7.39 `%` is a percentage, not the remainder

F3's calculator read `%` as C does: `10 % 3` was `1`. Everyone who reaches
for a launcher's calculator reads it as Soulver and Raycast do -- `20% of
50`, `25% off 399`, `399 + 25%` -- and those cannot be written while `%`
means something else. So from F14 `%` is a percentage, and the remainder is
written `mod`: `10 mod 3`.

It changes more than the root list. The *Currency* view's amount and the
snippets' `{calculator}` and `| calculate` (F11) are the same arithmetic,
and change with it; their tests change in the same commit. A bare `50%` is
the number `0.5` where an amount is asked for, and no answer in the root
list, where a lone number is none.

### 7.40 The calculator's answer is a card, in the place of two rows

I-10 asks for one row height. F14's calculator answer is drawn the way
Raycast draws it -- what was typed, an arrow, the result, each named
beneath, in type large enough to read across the room -- and that does not
fit in a row.

So the answer is a card `layout::CARD_ROWS` rows tall, drawn above the list
rather than in it: the list is a `uniform_list`, whose rows are one height
by construction, and it stays that way. The card takes the room of those
rows, not more: the window is as tall with a card as with the rows it
replaced, so nothing outside the list moves when one appears. Inside it,
the rest of the results start lower, which is what any answer appearing
first does. The card is the first result for selection and for `Enter`;
only the drawing and the scroll know it is not a row. Its height, its type
size and its gap are tokens (`layout::CARD_ROWS`, `layout::text::CARD`,
`layout::CARD_LABEL_GAP`), as I-5 asks. An answer in money names its
result by the rates' date, so a stale table shows on the card itself.

### 7.41 The launcher's geometry moves from constants into a style

7.6 kept geometry in `lumen_core::layout`, where only the project could
change it, and checked the relations between the values at compile time.
F15 makes the launcher's surface themeable (AD-13, revised), so those values
become a resolved `Metrics` read at run time: the base style is the
constants, a style file overrides some of them, and the compile-time
assertions become checks run when a style loads. I-5 is unchanged in
substance -- no literal measurement in a render function -- and the source
of the measurement is now the active style instead of a constant.

What this costs is a field read where a constant was folded in, a few
nanoseconds against an 8 ms frame, and the guarantee that a broken relation
never produces a binary, which becomes a guarantee that it never produces a
window: it is reverted to the base before anything draws. The base style is
checked by a test, so the project's own values still cannot be broken
unnoticed.

### 7.42 Letter spacing is applied after shaping, as a patch to gpui

gpui 0.2.2 has no letter spacing, and a style cannot ask for what the
renderer cannot do. `docs/LETTER-SPACING.md` planned the patch; this records
what was built and where it departs from the plan. Every changed line is
marked `LUMEN PATCH (docs/LETTER-SPACING.md)`, as 7.35's are.

The spacing is applied in shared code, after the platform has shaped the
line, so it does not live in `direct_write.rs` and Linux gets it with F5. In
`layout_line` each run's spacing is converted from em to pixels with that
run's font size, after shaping and after the `force_width` block, so the
grid snap still compares unspaced positions. A glyph moves by the sum of the
spacing of every glyph before it, matched to its run by glyph index, which
copes with right-to-left order; the line's width includes the spacing after
the last glyph, as CSS does. Caret, selection and hit-testing read the moved
positions. When every run has zero spacing the step returns at once.

Where it differs from the notebook:

- **A `LetterSpacing(f32)` newtype in em, not `Option<Pixels>`.** `TextRun`
  derives `Eq` and `FontRun` is a hash key, so it has a manual `Eq` and
  `Hash` that fold `-0.0` and NaN into zero. The fields on `TextStyle` and
  `TextRun` are plain, and zero means none. `FontRun` merging requires equal
  spacing, and the spacing is part of the layout cache key.
- **`LineWrapper` keeps its per-character caches unspaced** and
  `width_for_char` adds the spacing. Pooled wrappers are reset on checkout by
  `line_wrapper_with_letter_spacing()`. Wrapped lines go through
  `layout_line`, so they take the step too.
- **Ligatures are one glyph and move as one; font features are unchanged.**
  The notebook suggested turning `liga` off where tracking is set. That is
  not done, and stays an open point.
- **The macOS-only `FontRun` literals in `vendor/gpui/src/platform/mac/
  text_system.rs` tests are not updated.** Lumen does not build for macOS.

On the Lumen side `text.tracking` is a style token in em, base 0, range
-0.05 to 0.1, clamped like every metric; the schema has 30 tokens. The
launcher's root applies it (`styling::tracking`, which sets nothing at 0), so
the query, rows, headers and footer inherit it, and keycaps reset to none
(`styling::untracked`). It is not in `styling::typography`: settings and the
overlay do not read a style (7.41).

The presets were measured against new captures of their references, and the
gap was mostly type size, not tracking: Carbon moves to 13 pt for title,
subtitle and accessory, 11 for section headers and 12 for the footer, and
Graphite's titles to 12.8 pt. Only Graphite tracks, at -0.02; Carbon and Ink
keep 0. Measured words land within about a pixel of Raycast and 1.5-2.3 px of
Asyar; Ink stays 1.7-4.6 px wide of Vicinae, a size question left open.
Typing with tracking set costs nothing measurable: Graphite at -0.02 against
the same style at 0, alternated twice, p95 2.31 and 2.27 ms against 2.29 and
2.29 ms.

---

### 7.43 The calculator's card is opened to the style

Carbon's calculator did not look like Raycast's, and the card was fixed
structure: two rows, plain labels, no divider. Opening it the way F15 opened
the rest (I-11) adds four tokens and four variants, listed in the schema
tables; their base values draw the card as it was, so a style that sets none
of them is unchanged (I-12), and any style may set them. Carbon sets all of
them, measured on Raycast's capture at 1.25 scale: values 24.5 pt at 600
within a pixel of Raycast's, three rows under a header, chips, a divider,
dimmed operators, and a label gap of 21 that puts 41.5 px between a value
and its chip against Raycast's 42. Carbon's card is 143 px tall against 154:
rows are whole.

The line under the search field was drawn, but in the calculator view the
card's fill, which on Carbon is the colour of `border.subtle`, began 3 px
below it and swallowed it. `rule.width` thickens it and the footer's; Carbon's
1.6 is two physical pixels at 1.25, Raycast's. The card's divider does not
follow it: it is a hairline in `border.strong`, because `border.subtle` can
be the card's own colour.

The answer gained a second action, *Copy Expression*, and its first reads
*Copy Answer*: the footer advertises the action panel only when there is a
second action to reach.

### 7.44 Actions carry their group and glyph; the panel's keys come from the keymap

The action panel is to look like Raycast's: an icon and keycaps on every row,
and a rule between runs of related rows. What a row *is* belongs to the action,
so `item::Action` answers it: `group()` returns an `ActionGroup` -- *Primary*,
*Alternate*, *Copy*, *Manage*, *Favorite*, declared in the order the panel
shows them -- and `glyph()` names a bundled icon, by the name an extension's
`Icon::Glyph` uses, so `core` knows no path (AD-03). The UI's table is checked
against every name an action can return. A view's action may name an icon too
(`icon`, optional, so views already written still load: I-12, I-13); its
rows are one run, and a row without an icon keeps the square empty (I-10).

Shortcuts are not a field of the action. A displayed shortcut that is written
beside the row can disagree with what the key does, so the panel asks the
keymap which binding performs the action (`keymap::Shortcuts`, read from the
bindings `install` puts in place, rendered as keycap labels) and a row with no
binding shows none (I-14). A favourite shows the chord that names it; the first
row shows `Enter`, the second `Ctrl+Enter`.

`Ctrl+Enter` is a binding of its own, `RunSecondaryAction`: it runs the second
of the selected row's actions -- a new window for an application, a private
one for a link -- and does nothing when there is none, outside the root list or
while a picker, a browser list or a quicklink's argument is open. The panel
still opens on that second action, which is why the key and the panel agree
(*it opens on the first since 7.45*).

Three verbs changed to say what they open: *Open Application*, *New
Application Window* and *Open Command*. The footer reads the first one's.

### 7.45 The action panel is a popover over the list

*Supersedes 7.11.* Raycast's panel floats at the bottom-right of the list with
the list still visible behind it, and that is the look this project is after.
7.11 gave up on it for two reasons, and both have an answer now.

**The window had no room.** It is sized exactly to its content, so a popover
taller than a short list had nowhere to stand. While the popover is open the
window is `max(the height the list would have, search + inset + popover + inset
+ footer)` (`Config::panel_height`). With an ordinary list that is the list's
height: the window does not change size at all and the list is not laid out
again, so nothing moves beneath the popover. It grows only when the list is
shorter than the popover, and goes back when it closes. The list is laid out to
its own height (`Launcher::underlying_height`), not the grown window's, so it
never stretches. The picker, which still replaces the list, keeps the old
measure as `Config::picker_height`.

**An absolutely positioned element had no height.** With only `right` and
`bottom` set, taffy derives none. The popover now has an explicit height, from
its rows (`layout::action_panel::height`: the header, the padding and outline,
the rows shown -- capped at `MAX_ROWS` -- and the rules among them) and an
explicit width. It sits in a transparent backdrop that is itself given the
region's explicit height, inside a `relative` wrapper of the same.

The backdrop occludes the mouse, so a click on the list behind closes the
popover and cannot run the row under it, and the rows behind do not react to
the pointer while it is open. The popover occludes too, which is what stops a
click on one of its rows reaching the backdrop. Past `MAX_ROWS` the rows scroll
in a container tracked by a `ScrollHandle`; the rules are children of that
container, so the highlighted row is revealed by its child index
(`action_panel::child_index`) and not by its position among the actions.

It is drawn as Raycast draws it: a muted header naming the target, a 16 px icon
at the start of each row (the square kept when there is none: I-10), the keys
as keycaps or as text according to the style's `footer.hints`, and a rule where
the group changes. The entries keep their order. It opens on the first row,
which is what Enter would run and now shows with its key; the old reason for
skipping it -- a panel whose Enter repeated Enter -- went when the panel stopped
being a place to reach a second action (`Ctrl+Enter` is).

The popover is opaque whatever the theme's `surface.elevated` alpha, since it
floats over text. It adds no colour token: its outline is `border.strong`, the
rules `border.subtle`, the highlight `selection.background`. Its geometry is in
`layout::action_panel`, shared with the picker's rows.

### 7.46 The action panel searches its own actions

Under the rows, below a rule, the popover has its own field, "Search for
actions…": a second `TextField`, not the query borrowed. Ctrl+K moves focus
into it, empty; the query keeps its text, and since the query field no longer
has focus nothing typed reaches `rerank`. The panel's field is subscribed apart
and only narrows the panel. Focus comes back to the query in one place,
`settle_panel_focus`, called at the top of `render`: whenever the panel is
closed, by whichever of its many paths (Escape, Ctrl+K, an action run, a click
outside, a re-rank), the query is refocused and the panel's search emptied. A
new way of closing it therefore cannot leave the keys going to a hidden field.
The root still has no `track_focus`; the field sits inside the
`Launcher` context, so Up, Down, Enter, Escape and Ctrl+K resolve there as
they do from the query.

The search uses the root list's matcher (`RootSearch::filter`, as the picker
does) on the rows' titles, but keeps the panel's order, not the ranking: the
groups decide the rules, and a rule is drawn only between two shown rows of
different groups. Each change puts the highlight on the first row left; no
match shows a muted "No matching actions" row, and Enter does nothing. The
browser page is searched by the same field, from empty. Escape closes the panel
in one press; it does not clear the search first.

The popover's height is still computed from every row, plus
`layout::action_panel::SEARCH_HEIGHT`, so typing never resizes the window:
the rows area keeps the unfiltered height (capped at `MAX_ROWS`) and the rows
left sit at its top.

### 7.47 The action panel's placement is the style's

Raycast puts the popover at the bottom-right; other launchers put theirs
under the search field or in the middle. Where it stands, how wide it is and
how many rows it shows before scrolling are now style tokens, with the
constants of 7.45 as their base: `actions.anchor` (a variant: `bottom-right`,
`bottom-left`, `bottom-center`, `top-right`, `top-left`, `top-center`,
`center`), `actions.offset-x` and `actions.offset-y` (0--64, base 8),
`actions.width` (280--720, base 440) and `actions.max-rows` (3--12, base 7).
They clamp and fall back as every other token does; a style that sets none
places the popover as it was (I-12).

The offsets always point inward: from a right edge the popover moves left,
from a bottom edge up, and on an axis the anchor centres it moves right or
down. With one convention for every anchor, the popover fits in any region its
size plus twice the offset on each axis, so `Config::panel_height` keeps
`actions.offset-y` above and below it whatever the anchor -- a top anchor is as
clear of the search field as a bottom one is of the footer -- and the width is
narrowed to the window less `actions.offset-x` either side
(`Metrics::actions_width`), since `config.json` can set the window's width.
The corner is computed by a pure function, `ActionsAnchor::origin`, and set as
`left` and `top`.

There is no height token. The popover's height is still computed from its rows
(`layout::action_panel::height`, now given the style's row cap), so no style
can give it a height that clips a row (I-10). The rows themselves, the header,
the field and the padding keep their constants: they are the picker's too, and
a style chooses where the popover stands, not what it is (I-11).

### 7.48 The region between the chrome gives, and every view's height is the style's

**One flexible part.** The window is never exactly the height the launcher
asks for: its border is inside it, the platform rounds its size to whole
pixels, and it catches up with a new height a frame late. Something has to
take up the difference, and it must not be the chrome. The search field and
the footer are `flex_none`; the region between them is the one element that
gives, `flex_1` with no height of its own, and the body inside it keeps its
height and is clipped to it. The popover floats over the region, unclipped,
so its shadow is not cut.

The chrome was made rigid first (db02304), so that the footer kept its
height while the window grew for the popover; but only the popover's wrapper
was allowed to shrink, and only while the popover was open. In a view the
region was then rigid too, and a body that cannot shrink -- a list beside its
detail pane -- was taller than the room the window actually had, and pushed
the footer out of it. A test now reads the source and fails if the search
field or the footer can shrink, or if the region is given a height.

**A view's height follows the style.** A view no longer hugs a fixed number
of rows; its height comes from the same tokens as the root list's
(`view::rows_height`, a pure function):

- A list is as tall as its rows, accumulated whole up to `list.height`
  (`Config::list_height`), as the root list is (7.29): a short list still
  shrinks to its rows.
- A detail view, and a list beside its detail pane while its rows fit, are as
  tall as a full root list (`Config::full_list_height`, the whole rows that
  fit in the list's height), not a fixed three rows or the raw cap, which
  need not be whole rows.
- A view that is loading, and one whose list is empty -- nothing sent, or a
  search that matched nothing -- are as tall as a full root list too. Both
  were three rows; Clipboard History therefore opened short, said
  *Starting…*, and grew when its rows arrived, and a search inside a view
  that matched nothing shrank the window under the user's typing. A loading
  view has no rows, so it takes the empty rule and needs no height of its
  own.

So a view never jumps between loading and loaded, and a style's height
reaches views the way it reaches the root list.

The root list keeps its own rule: with nothing found it collapses to the
search field and the footer. That is not the same jump. The root list has no
loading state, and its height already follows its results as they change;
collapsing when there are none is deliberate, and unchanged.

### 7.51 An ellipsis needs the width it was cut to, as a patch to gpui

Text styled `truncate()` never drew its ellipsis; it was clipped at the edge
of its box instead. gpui 0.2.2 measures text once per layout pass and keeps
the result, and its cache check compares only the wrap width. A line that
does not wrap has none, so the first measurement always hit the cache. Flex
layout takes that first measurement at max-content, where there is no width
to cut to, and the definite width offered next was answered with the full,
uncut line.

The patch stores the width a layout was truncated to beside its wrap width,
and the cache is used only when both match. Every changed line in
`vendor/gpui/src/elements/text.rs` is marked
`LUMEN PATCH (docs/ARCHITECTURE.md, 7.51)`, as 7.35's are. A test in the
same file lays out a truncated line in a narrow flex row and checks that it
ends in the ellipsis and fits the row.

Across a gpui upgrade (I-6) the test is the check: if upstream has fixed the
cache, drop the patch; if not, re-apply it. gpui is not a workspace member,
so its tests run against its own manifest.

### 7.52 A borderless window's client area is its whole rect, as a patch to gpui

The launcher's list was cut a few pixels short at the bottom, in the root
list and in every view: a row under the last one in view showed half above
the footer. The heights were right. gpui 0.2.2 answers `WM_NCCALCSIZE` for a
window without a title bar by insetting the client area by the resize
frame's thickness (`SM_CXSIZEFRAME` + `SM_CXPADDEDBORDER`) at the sides and
the bottom and by a pixel per scale step at the top. The launcher is a
`PopUp` with no frame at all, so the window rect `SetWindowPos` set was the
size Lumen computed and the client area gpui drew into was smaller: 941x595
against 959x605 at 1.25, 8 logical pixels short. The region between the
search field and the footer is the part that gives (`Launcher::render`), so
the list lost them.

The patch skips the insets for a window whose style has no `WS_THICKFRAME`:
its client area is the whole rect, which is what `rgrc[0]` already holds.
A window with a resize frame is unchanged. The changed lines in
`vendor/gpui/src/platform/windows/events.rs` are marked
`LUMEN PATCH (docs/ARCHITECTURE.md, 7.52)`; a test there checks the
decision for a pop-up and for a framed window. Across a gpui upgrade the test
and the window's client rect are the check.

## 8. Risk register

| # | Risk | Phase | Status |
| - | ---- | ----- | ------ |
| R-1 | Real transparency unworkable; the system backdrop does not show | F0 | **Hit and mitigated.** See 7.3. Three backdrop modes are implemented and configurable. |
| R-2 | Breaking changes in GPUI between versions | all | `gpui` pinned exactly (I-6). F0 assertions re-apply the window styles rather than assuming GPUI's. The vendored patch (7.35) is re-applied and re-measured on every upgrade. |
| R-3 | Insufficient GPUI documentation | all | Mitigated by reading the crate source directly; it ships its `examples/`. |
| R-4 | Windows assumptions leaking into the core | F1–F4 | I-1 and I-2, cross-checked in CI from the first commit. The cross-check is paused until F5 resumes (7.38); I-1 alone guards it meanwhile. |
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
