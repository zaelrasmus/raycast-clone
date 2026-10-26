# File search, the rest of the way (F16)

## STATUS 2026-10-04 evening: every planned F16 slice is built and installed

F16 CLOSED 2026-10-04: all tasks done and installed; fast-forwarded into
local `main` at `f1b9ea0` (user's request). Not pushed. LAST TWO TASKS before closing F16 (user approved 2026-10-04, then reinstall):
- [x] 9. (`dfec304`, `1b0e313`; core 1180, ui 136) `files.hot-memory` (MB, default 8, cap 32) replaces HOT_BUDGET;
  settings page field; docs.
- [x] 10. (10a `7847264`, 10b next commit; core 1187, ui 140, win 75;
  release built; installed; live use left to the user; apps_for/open_path_with, APP_PREFIX, live
  .txt -> Notepad first of 4, folder -> File Explorer) Save a file/folder as a quicklink (Ctrl+K panel): 10a Platform
  `open_with_apps(path)` (Windows SHAssocEnumHandlers, default first;
  folders -> Explorer) + launching a path with a chosen app (I-9);
  10b quicklink model/form: path links with an app chooser; action on file
  and folder rows in root and Search Files; docs.

DONE `f159dfe` (installed; root seen with space under the last row;
window 959x611, client equal; config.rs window_height fixed too) -- space
under the last row = max(list.padding, row.inset) (user's choice; Carbon
list.padding 2 left the last row touching the footer rule). One helper in
styling.rs used by render (view.rs ~262, launcher.rs ~5127) and height
math (launcher.rs ~5066, ~2402-2470). On resume: `git status`; if the
change is there and checks pass, commit, `scripts/install.ps1 -SkipBuild`
(after a release build), screenshot root with drive.ps1 and no keys.

Later: user-reported visual bugs fixed and installed -- preview wraps long
lines, CR/CRLF -> LF (`94291ca`); window client area was inset 18x10 px by
gpui for a frameless popup, cutting the list above the footer everywhere
(`882ff69`, vendor/gpui patch, ARCH 7.52; measured client == window
959x605 after; root's last row seen complete).

HEAD `e5fd73e`, 39 commits over `main`, not pushed (push/PR/merge are the
user's). Left to confirm live, with the user's OK before any keystrokes
(the user was typing during earlier drives): settings coverage page (6c),
preview wheel scrolling + tint (6d-ux). Known gaps: extra-folder watch not
restarted on config change; a cached pane from an older query shows until
the new read lands; cold start 370-415 ms vs 400 ms budget (one run read
415 ms while another daemon was starting).

## RESUME HERE (written 2026-10-04, end of a long session)

Branch `feat/file-index`, HEAD `7897b19`, 24 commits over `main` (`2bf727b`),
working tree clean. Not pushed; push/PR/merge are the user's call. The
animation work is parked as `3ba139f wip: animations` on `feat/animations`.

Done: slices 0-5, and task 6c (the 500k ceiling replaced by an on-disk
compact index + an 8 MB hot layer in memory; WAL; `files.index-detail`
complete | balanced | compact). Design and every measurement:
`docs/FILE-SEARCH.md`.

2026-10-04 (user away): user waived RDD/review for the remaining work and
asked to finish it, then build and reinstall (`scripts/install.ps1`).

Next, in order:

1. **c4 -- measure and install.** Run `scripts/measure-idle.ps1` on the
   release build with `LUMEN_HOME=target/measure-f16/{home,homed}` (configs
   already there) -- record working set, private, idle CPU in FILE-SEARCH.md.
   It STOPS the user's installed Lumen: restart
   `C:\Users\kiza2\AppData\Local\Programs\Lumen\lumen.exe` afterwards. Also
   unmeasured: the WAL double write on a real 500k rebuild. Then
   `scripts/install.ps1`. Expect the user's index to rebuild once (digest
   changed when the ceiling left). UPDATE: `7897b19` was already installed at
   the end of the session; only the idle measurement is left.
2. **6b -- two fixes from the user's use.** (a) With *Include hidden files*
   on, still skip tool caches (`.rustup`, `.cargo/registry`, `.npm`, ...,
   Raycast-like): 81 % of the user's home index was dot-folders. Show them in
   *Excluded by default*. (b) Typed paths: `C:\`, `D:\Dev\`, `~\` in root and
   Search Files list that folder live from disk (not the index), filtered by
   the text after the last `\`; Tab completes, Enter opens or enters.
3. **Slice 6 -- content via Windows Search**, view only (see FILE-SEARCH.md).
4. **Slice 7 -- extra application folders** (`apps.extra-folders`).

Known small gaps: the root trade (no mid-word outside the hot layer; the
view finds all); picture rows show thumbnails, not type icons
(lumen-platform-windows icon flag); whole-drive warning has no entry count;
Rebuild has no IPC/CLI route.

How to work here (learned the hard way):

- Delegate to `gentle-ai-worker` with a `## Allowed edit surfaces` block of
  bare paths; split work so each piece ends in one commit -- workers die at
  the user's usage limits; after one dies, `git status` shows what survived.
- Benches: `timeout 900`, progress lines, output to `target/disk-bench`.
  C: is nearly full: temp files under `target/`. Build with `-j 2` on OOM.
- Live UI tests: `target/measure-f16/drive.ps1` (-Keys "keys~ms~...",
  -NoShow), `wheel.ps1`. Open settings with `lumen settings`, pages with
  Ctrl+Tab. NEVER send Enter in root without a screenshot of the selected
  row first (it once ran Cheat Engine's "Reset settings").
- The user wants honest numbers, decisions as questions, and Raycast parity.

Branch `feat/file-index` from `main` (`2bf727b`). Design: `docs/FILE-SEARCH.md`
(`3d55e1e`). Animation work parked as `3ba139f wip: animations` on
`feat/animations`, resumed later (`odd/tasks/animations.md`).

One slice, one work-unit commit at least; push, PR and merge are the user's.

## Tasks

- [x] 0. docs: the design (`3d55e1e`)
- [x] 1. Slice 0 -- measure: synthetic 100k/1M name corpus; FTS5 trigram vs
  in-memory name table; query p95 per query length, memory, disk size, build
  time; the real entry count of this machine's home under the skip rules;
  verify FTS5 in the bundled SQLite and `ignore`'s Windows hidden attribute.
  Record the numbers and the storage choice in `docs/FILE-SEARCH.md`
- [x] 2. Slice 1 -- the index: schema, walker with skip rules, scopes and
  exclusions with `roots` migration, folders as results (`Kind::Folder`),
  root's appended Files section; F3's walk removed; ARCH 7.20/7.21/7.26
  - [x] 2a. core `file_index` module: walk (ignore rules, 500k ceiling in
    scope order), SQLite store (no FTS), in-memory names + matching and
    ranking, frecency boosts, `Kind::Folder` + glyph, find-and-rank bench
  - [x] 2b. wiring: config keys + `roots` migration (empty `roots` = default),
    Files section before fallbacks, async query with generation counter,
    selection kept by item identity, in-root levels, remove F3 scan, ARCH
    7.20/7.21/7.26 + README, measurements (idle memory, start load)
    - carry-over from 2a: scope/exclusion/`id_of` path comparisons are
      case-sensitive; resolve config paths to the disk's real form so a
      differently cased exclusion still matches. Items have no keywords:
      root must place them, never fuzzy-rank them
- [x] 3. Slice 2 -- fresh: watcher without excluded subtrees, start
  reconcile, Rebuild Index; idle CPU measured
- [x] 4. Slice 3 -- Search Files view: recents, details, actions, Exclude
  from Index, status (reference: Raycast screenshot; layout as Clipboard History)
  - [x] 4a. core + platform: view source, Recently Used (frecency + Windows
    recents via `Platform::recent_files`), Type filter categories, preview
    choice and reading (64 KB, image/code/prose/none), metadata, actions
    (Show in Explorer, Open With, Exclude from Index, Rebuild), status
  - [x] 4b0. fix (`7406064`; RED 7 failed, GREEN 79; lumen-core 1030): a scope nested under a skipped/excluded/hidden/ignored
    folder of an earlier scope is dropped (walk.rs `effective_scopes`); keep
    it when the outer walk cannot reach it. Test-first. Config key
    `default-skips-off` (settings UI in slice 4)
  - [x] 4b. (`75eddff`; ellipsis fixed by gpui patch `936e2fc`, ARCH 7.51;
    gpui RED/GREEN, seen live in view and root) UI: mono palette key (Cascadia Mono default) and the code card
    for every text preview, `files.preview-font` = monospace | interface,
    shell icons per file, dimmed parent prefix on duplicate names; plus from
    the live test: text typed in the instant the view opens is lost (also
    true of Clipboard History), long titles cut without an ellipsis
- [x] 5. Slice 4 (`7427b73`; seen live via `lumen settings` + Ctrl+Tab; picker
  not clicked) -- settings page: three levels, ignore files, patterns, hidden
- [x] 6. Slice 5 (`a4d4d5b`; RED 12, GREEN lumen-core 1062, ui 129, win 59; seen
  live: offer, empty hint, picker at D:, cancel keeps view; installed) (revised `f37f4fd`: folders on drives, not drives) -- drives:
  `internal_drives`, offer row + empty-result hint opening the picker at the
  drive, `declined-drives`, whole-drive warning; NO drive switches
- [x] 6a. (`13ff2e7`: compact V1 36 MB vs 115 MB today on real 500k; V2 words
  0.1-0.5 ms; trigram/scan for mid-word; ranked disk query not yet measured;
  user to choose A/B/C) Measure a hot-layer + disk design to remove the 500k ceiling (user:
  the hard limit is a trap -- data loss, not slowness). Bench
  file_index_disk.rs on the real files.db (500k) + synthetic 1M/2M: V0 full
  paths, V1 compact folders table, V2 +FTS5 word prefix, V3 +trigram, V4
  memory blob; sizes, build, warm/cold-ish latency, memory. Raycast: a whole
  drive + hidden in ~89 MB. Task musqzy75-a-b6cq. No product code.
- [x] 6c. Replace the ceiling with design B (user's choice): compact store
  (folders once), FTS5 word index, hot layer in memory within a budget,
  root = hot then disk appended, view mid-word by plain scan; remove CEILING
  and the truncated meta (fixes 6b(a)/(b) by removal)
  - [x] c1. (done; tiered SQL b <=3.4 ms real, 2M needs per-tier LIMIT;
    index whole words AND parts; ~92 MB real) measure ranked best-N from the word index (exact, name start,
    shortest) at real 500k / 2M -- the one open point
  - [x] c2a. (`2c32c83`; RED 8, GREEN file_index 92, lumen-core 1075; real
    102 MB vs 121.5, build 6.3 s, load 195 ms; contentless_delete OK)
    schema v3, tokens, write paths, Names from v3; app unchanged
  - [x] c2b. (`84848cf`; stall was substring row reads + per-statement read
    transactions, not compact; lumen-core 1090; 2M p95: complete ok,
    balanced 8.09 inv 2024 N=64, compact ~30 ms html/inv 2024; real <=3.7 ms;
    setting saved, not applied by daemon; open: 4-char prefix index for
    compact) ranked disk query (keyed contentless FTS, per-tier LIMIT),
    substring scan, index-detail setting, DISK_BENCH=store acceptance <8 ms 2M
  - (original c2 text) store schema v3 compact + word index (split at separators,
    capitals, digits) + migration (rebuild); `files.index-detail` =
    complete (trigram) | balanced (default) | compact (detail=none)
  - [x] c2c. (`f3970a7`; 2M worst p95 5.7 ms all levels, +3-6% disk) quick fixes: prefix '1 2 3 4' (word
    tables rebuilt, no walk) + apply index-detail in the launcher; bounded bench
  - [x] c3a. (`f8a5f34`; hot 282k/500k in 8.1 MB, load 182 ms, root p95 <=2.1 ms) hot layer within HOT_BUDGET, root = hot + Store::ranked
    merged, ceiling removed, startup loads hot only, relayout under build lock
  - [x] c3b. (`5bc3fb0`; first phase 4-11 ms for 200 rows; mid-word 165-190 ms
    balanced) Search Files on disk
  - [x] c3c. (WAL + read-only reader; lumen-core 1110) mid-word scan on its own read connection (root never
    waits); fix 'held in memory' wording: ranked first, mid-word appended
  - [x] c3. (covered by c3a-c3c) (also: Search Files shows ranked word starts first, mid-word
    substring rows appended when they arrive -- balanced substring ~90-105 ms
    on the real index) hot layer + root/view queries on the new store; root under 3
    letters = hot layer + names of 1-2 chars only
  - [x] c4. (`a981067`; home 72.8 MB WS / 0.000 %, home+D: 405k entries
    80.8 MB WS / 97.0 MB private / 0.004 %; WAL peak >=94.6 MB during
    rebuild; cold start 374-384 ms) remove ceiling, measure idle memory/CPU, docs (ARCH, README)
- [ ] 6b. Fixes from the user's live use (one index kept; two-level model
  rejected -- Raycast is one index too): (a) `truncated` meta names home when
  D:\ was cut (meta: per_scope 303447,196553; truncated C:\Users\kiza2);
  (b) full-index warning in Search Files and settings; (c) under include-hidden,
  still skip tool caches -- DONE `65c8c39` (RED 3, GREEN file_index 123,
  lumen-core 1113, ui 130; fmt, clippy clean) (.rustup, .cargo/registry, .npm, ...) -- DECIDED
  2026-10-04: whole folders by bare name in SKIPPED (.rustup .cargo .npm
  .nuget .pnpm-store .m2 .conda .docker), digest bumped so indexes rebuild
  once; settings card picks them up unchanged -- hidden on
  took home 54k -> 303k; (d) typed paths (C:\, D:\Dev\, ~\) list a folder
  live from disk in root and Search Files, Tab completes, Enter opens/enters
- [ ] 6d. Typed paths (6b(d)). DECIDED 2026-10-04: Enter opens a folder (Explorer),
  Tab completes it into the query; Tab is contextual -- a deeper key context
  (e.g. `TypedPath`) on the search row only in path mode wins over NextScope
  (Search Files Type filter) and AskArgument (root); Shift+Tab unchanged.
  Root in path mode = listing only (replaces Files, first; fallbacks last).
  Filter = case-insensitive contains, starts-with ranked first, folders
  before files. Live listing ignores skip list/exclusions; hidden items
  follow include-hidden. Path = whole query only; `X:\` `X:/` `~\` `~/`;
  no UNC, no relative. Core reads with std::fs (precedent preview.rs:177;
  note in ARCH), on the background executor, cancellable, capped.
  Slices (one commit each): 1 core browse.rs parse/list/complete;
  2 core root path rows (shared item builder, file_rows filter bypass);
  3 core view Session::ask path branch + FILE-SEARCH.md section (fix :1120,
  :1355, :1389); 4 ui wiring query_files + Effect::Search; 5 ui CompletePath
  action + key context + BINDINGS row + Enter.
  - [x] d1. core browse.rs (`b9dad3e`; RED 12/14, GREEN 14, lumen-core 1127;
    API parse/list/complete, Typed{dir,filter,shown}; hidden via ignore,
    no cfg; cancel returns empty Vec -- treat as stale; review declined)
  - [x] d2. core root (`2dfb9cc`; lumen-core 1135; RootSearch::typed/set_typed,
    file_index::item_for; open: blank flash per keystroke -> keep same-dir
    listing re-filtered until the new one; cap in launcher; cancel index query)
  - [x] d3. core view + docs (`14f8cb1`; RED 7, GREEN lumen-core 1145, ui 130;
    Effect::Browse, Session::completion(id), RootSearch::completion(query,id),
    browse::narrow; view half of d4 wired via FileSearchRun::browse)
  - [x] d4+d5. ui wiring + Tab (`b2a373c`; ui 131, core 1146; release built)
  - [x] fix (`c7fa851`): landing listing kept selection off the file rows
    (forget_file_hits before place_files read the id; index set_files had
    it too) -> RootSearch::take_typed; files/folders say "Open". Seen live
    after install: C:\Us selects Users; Tab into AMD selects first row;
    Search Files D:\Dev\ + Tab -> D:\Dev\.atl\ lists.
  - Known: view footer still hints "Type Tab" in path mode; view window
    613 vs 605 px with a short list (pre-existing, not typed paths);
    `.bun` cache not skipped; failure toast says "Couldn't open application".
- [x] 7. Slice 6 DECIDED 2026-10-04: contents asked from 3 letters, always,
  setting `files.search-contents` (default on); a content match is NOT
  marked on its row -- its text preview highlights the matched words and
  scrolls to the first; coverage shown only in settings (per scope, button
  to Indexing Options). Approach: OLE DB Search.CollatorDSO via windows
  0.61 (`Win32_System_Search`), Apartment::enter. Commits: 6a platform
  (content_search, scope_covered, open_indexing_options; I-9) + SQL
  builder tests; 6b view third phase + preview highlight; 6c settings.
  - [x] 6a platform (`a5a3bda`; RED 4, GREEN win 71; live: home covered,
    5 matches; CSearchManager needs CLSCTX_LOCAL_SERVER; DBGUID_DEFAULT
    defined locally; Linux todo!() -- callers must not run it on Linux)
  - [x] 6b view + highlight (`94bcedf`; core 1164, ui 132; installed; live:
    'lorem' appends main.js/zip/index.html from Windows Search)
  - [x] 6b-fix (`4f5bc94`; core 1171, win 71, live index ok; installed;
    NOT yet seen on screen -- live driving stopped, user at keyboard):
    preview opens at a deep match (index.html line 2209 was
    past the 200-line preview, so no highlight); CONTAINS(*) matched path
    properties -> System.Search.Contents. Highlight not yet seen on screen.
  - [x] 6c settings coverage (`b0dd6c5`; ui 133; release built; not seen
    live yet -- needs Ctrl+Tab, ask user first)
  - [x] 6d-ux (`e5fd73e`; ui 135; installed; wheel + tint not yet seen live)
    (user, after 6c): (1) a stronger visual highlight of content
    matches in the preview; (2) the preview card scrolls on its own --
    today the whole right pane scrolls; split: preview card scrollable,
    metadata separate. User confirmed the deep-match preview works live.
- [x] 7. (orig) Slice 6 -- contents through Windows Search; coverage per scope; ARCH 7.19
- [x] 8. Slice 7 -- extra application folders (first worker died at the
  usage limit with nothing saved; split into 7a config + trait
  `enumerate_applications(&self, extra_folders: &[PathBuf])` + Windows
  listing, and 7b watching + docs. Slice 6 mapping also died; redo it.)
  - [x] 7a (`f8c7cfd`; RED compile, GREEN win 62, core 1153, ui 132)
  - [x] 7b (`2f094cb`; win 65; release built; seen live in a LUMEN_HOME:
    Demo listed, 3rd-level Helper not, new exe picked up ~1.5 s; installed).
    Watch not restarted on config change (no handle): new folders watched
    from next start; re-index is immediate. Documented.

## Evidence

- Task 0: `3d55e1e` docs commit.
- Task 1: `36c52af`. Home 54 597 entries (2.5 s walk), D:\ 350 373 (23.7 s,
  >90 % games/launchers). Names-in-memory blob: all matches at 1M p95 4.59 ms,
  build 28 ms, 29.8 MB; SQLite FTS5 trigram uncapped 19.06 ms, build 30.7 s.
  Decision: names in memory, entries in SQLite (no FTS). `ignore` honours the
  Windows hidden attribute. fmt clean; clippy --all-targets clean (worker run).
- Task 2a: `2c2f9ed` (+2206/-152, ~1180 production + ~1020 tests; flagged
  over 400). RED 1 passed/43 failed against stubs, GREEN 44/44; lumen-core
  949 passed; lumen-ui 127 passed after cherry-picking the CRLF test fix
  `1c8dacd` as `5752239`. Find+rank p95: 55k 0.72 ms; 350k 4.43 ms for 1-2
  chars, <=1.25 ms for 3+; 1M 12.5 ms for 1-2 chars. Names 1.3/8.3/23.7 MB.
- Task 2b: `aab4732` (+1735/-732, flagged over 400). Root RED 7 failed / GREEN
  37 passed; lumen-core 962, lumen-ui 127; fmt, clippy, release build clean
  (worker). Fixes after review: row check vs names, build lock, digest-only
  rewalk. Measured idle (release, LUMEN_HOME): off 68.3 MB WS; home 73.9 MB,
  load 37 ms; home+D: 76.7 MB WS / 100.9 MB private, load 200 ms, DB 110 MB;
  CPU 0.004-0.009 %. Follow-ups for slice 2: private memory after the walk,
  DB stores full paths (~270 B/entry). Installed daemon restored.
- Task 3: `cdbafb4` (+1432/-135 + 2 new files, flagged). RED 14 failed, GREEN
  lumen-core 978, lumen-ui 127; fmt, clippy, release build (worker); file_index
  70/70 rerun after the WindowsApps skip. Measured: home reconcile 166 ms (no
  walk), home+D: 2.99 s; Desktop file applied ~2.5 s after creation, rename and
  delete too; idle CPU 0.002 % (one noisy 0.017 % sample, 0 batches in 100 s
  debug log); private home+D: 97.2 MB. Open: no IPC/CLI route for Rebuild
  (ipc.rs outside surfaces); quicklink-icon change no longer rebuilds files.
- Task 4a: `5c7a8a9` (+2581/-31, flagged). GREEN lumen-core 1019, lumen-ui
  127, platform-windows 58; fmt, clippy, release (worker). Live (release,
  LUMEN_HOME home, target/measure-f16/drive.ps1): view opens, Recently Used
  with Windows recents, results, image and JSON previews, metadata, watcher
  picked up new Desktop files. Found: lost first keystrokes when typed with
  Enter; no ellipsis on long titles; carbon.json not found because AppData is
  skipped (by design; ask user).
