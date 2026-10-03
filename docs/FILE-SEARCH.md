# File search, the rest of the way (F16)

**Status: in progress** -- slices 0 to 2 are built (see "What was done
differently"); slice 2's measurements of the running daemon -- idle CPU with
the watcher on, private memory after a build -- are still to be taken. This
is the plan. When a slice lands, its
decisions move into `ARCHITECTURE.md` (F16 decisions, and the revisions to
7.19, 7.20, 7.21 and 7.26 listed below), and this file records where the build
departed from it, as `LETTER-SPACING.md` does.

**Why.** F3's file search is honest about being small: a walk of six folders,
held in memory, capped at 5 000 files, never refreshed until the
configuration reloads. It finds the file you were working on and little else.
Folders are never results, a file downloaded a minute ago is not found, and
there is nowhere to search files *on purpose* -- only the eight rows root
search lends them. Raycast, Vicinae and Asyar all go further, and a user with
their work on a second drive finds nothing at all.

**Estimate.** Eight slices, one pull request each (see "Slices"). The first
is a measurement and changes nothing a user sees.

---

## Slice 0: what was measured

`cargo bench -p lumen-core --bench file_index` and
`cargo run --release -p lumen-core --example count_home`, on this machine.

**Real folders, after the skip rules** (`ignore` with hidden entries skipped,
ignore files on, today's `SKIPPED` names):

| Root | Entries | Walk, one thread | Where the bulk is |
| ---- | ------- | ---------------- | ----------------- |
| Home | 54 597 | 2.5 s | Downloads 39 255, `go` 9 323, Documents 2 383 |
| `D:\` | 350 373 | 23.7 s | ModrinthApp 216 646, Apps 78 945, SteamLibrary 33 188, XboxGames 12 387 -- and `Dev` 3 329 |

The estimate of 100 000 to 1 000 000 entries for a home folder was too high by
an order of magnitude. `D:\` is the case "offered, not taken" was decided
for: over 90 % of it is games and launchers, and the user's own work is 1 %.

**Storage, on a synthetic corpus** (p95 of 200 runs; "all" collects every
match, "500" stops at 500):

| | 100 000 entries | 1 000 000 entries |
| - | --------------- | ----------------- |
| SQLite + FTS5 trigram: build | 1.9 s | 30.7 s |
| SQLite: file on disk | 17.6 MB | 172.8 MB |
| SQLite, all, `invoice` | 1.89 ms | **19.06 ms** |
| SQLite, 500, any query | < 0.36 ms | < 0.54 ms |
| Names in memory: build | 3 ms | 28 ms |
| Names in memory: resident | 3.6 MB | 29.8 MB |
| Names in memory, all, worst query | 0.49 ms | 4.59 ms |
| Names in memory, 500, any query | < 0.10 ms | < 1.2 ms |

Two things the numbers say that the design did not expect. Trigram FTS gets
*slower* as the query gets longer, because it intersects more posting lists;
and ranking needs every match, not the first 500 in storage order, which is
where SQLite misses the budget.

**Decision: names are matched in memory; entries are kept on disk.** The
names live in one lower-cased blob with an offset table, scanned with
`memmem`, which answers every match within the 4 ms target up to a million
entries and builds a thousand times faster. At the sizes measured it is 2 MB
for this home folder and about 13 MB if all of `D:\` were added. SQLite stays,
without FTS, as the record of what was indexed -- paths, folders' modification
times for the start reconcile -- so a start never walks again from nothing.
The blob is rebuilt from it at start in milliseconds.

### Against the budgets

The names are only safe in memory because of a ceiling. Idle working set has
been measured at 71--75 MB across F8 to F15, against 100 MB:

| Index | Names in memory | Idle total, estimated |
| ----- | --------------- | --------------------- |
| This home folder, 55 000 | ~2 MB | ~76 MB |
| Home and all of `D:\`, 405 000 | ~13 MB | ~87 MB |
| 1 000 000 | ~30 MB | **~104 MB: over** |

So:

- **A hard ceiling of 500 000 entries** (~15 MB), a constant like F3's cap
  and for the same reason: a budget number. Scopes are walked in their
  listed order, so the scope that overflows is the one added last, never
  home. What did not fit is said, never hidden: *"The index is full at
  500 000 items; the rest of D:\ is not searched. Remove a scope or exclude
  folders."*
- **The settings page says what a scope costs.** Beside the scopes: *"Every
  folder and drive you add is held in Lumen's memory to search it instantly.
  Add the folders you search, not whole drives you don't."* -- with each
  scope's item count and the index's total against the ceiling.
- **SQLite is not left warm.** Once the names are loaded, the connection's
  page cache is released; it holds no memory while the launcher is idle.
- **Per-frame is untouched by the scan,** which runs off the UI thread; the
  frame receives at most eight rows.

The table is arithmetic, not a measurement of the process. Slice 1 is not
done until it is measured (see "Slices").

Also confirmed: `ignore` with hidden entries skipped does skip a file that has
only the Windows hidden attribute, so I-1 holds without a `Platform` call. And
`ignore` applies `.gitignore` only inside a Git repository by default
(`require_git`), which is what Git itself does and is kept.

## What was done differently

- **Fallbacks close the list, after the files -- but not inside them.** The
  *Files* section comes after every match and before the fallbacks, and a
  fallback still takes the last row of the first screen at the latest. Where
  that row would split the files, the fallbacks go above the section
  instead, so it stays in one piece and they stay in view. Fallbacks after
  the files have no header of their own.
- **A start walks nothing it does not have to.** The index the store
  recorded is installed at once when its rules are today's, then every
  indexed folder's modification time (schema version 2 records them, and the
  scopes' own) is compared with the disk and only the folders whose time
  moved are read again; the log says how many were checked and read, and in
  how long. A full walk is for a store that cannot be used, rules whose
  digest changed, the reload command and *Rebuild File Index* -- a command
  row in root for now; the view's action and the settings button come with
  slices 3 and 4. A configuration change walks again only when the walk
  itself changed, so `in-root` never walks, and turning `enabled` off and on
  loads the stored index back and reconciles it.
- **A change is applied to its folder, not its scope.** Whatever the
  watcher reports, the folder it happened in is read again -- one directory
  read, by the walk's own rules -- and compared with the store: what left is
  removed with its subtree, what arrived is added, and a folder that arrived
  is walked. A folder is read only if it is a scope or already indexed, so a
  change under `AppData`, a hidden, excluded or ignored folder is passed
  over without a read; a changed ignore file reads its folder's subfolders
  again too. The reconcile at start is the same work over the folders whose
  time moved. Updates stop adding at the ceiling and report the scope cut
  short, as a walk does. Folder times are read from the folder itself, not
  from its parent's listing, which NTFS updates lazily: compared with each
  other, the two made the reconcile read folders that had not changed.
- **The watch, as designed, settles in the daemon.** Each scope's folder is
  watched without its subfolders and each folder in it the rules allow with
  them; a folder that arrives at the top gets its watch when that change is
  applied. The callback hands on the paths `notify` allocated and returns.
  The daemon gathers them until nothing has arrived for 1.5 s (as 7.34), and
  the launcher applies the batch off the UI thread, under the build lock; a
  rebuild asked for since drops it. A watcher error, or a report that
  changes were lost, makes the batch a reconcile rather than a guess.
- **Names carry their row ids.** The names keep each entry's row id beside
  it, four bytes an entry, rather than making entry `i` row `i + 1`: a batch
  then removes and appends names in memory -- a copy of the blob, not a load
  of the store -- without renumbering rows on disk. Rows are `AUTOINCREMENT`,
  so between rebuilds a deleted row's id is never another entry's, and every
  row a query reads is still checked against the name and kind it matched.
  The store counts a version up on every write that changes rows; an index
  of an older version is not patched but loaded from the store.
- **The walk's memory is given back before the load.** The walked entries
  are dropped once written, before the names are read back, so the two are
  never held at once; the names and the short list are shrunk to fit, and
  SQLite's page cache is released after every load and batch. Whether
  private memory falls is still to be measured.
- **One build at a time, and never a wrong file.** Every build of the index
  file holds one lock from the walk to the load; a build superseded while it
  waited or walked writes nothing. A rebuild still rewrites the database
  under the index that is installed, so every row a query reads is checked
  against the name and kind it matched, and a row that moved is left out:
  until the swap the old index can answer less, never another file.
- **`limits` is read and ignored with one line in the log per run**, not one
  per reload.

## Slice 1: what was measured

The release build, each configuration in its own `LUMEN_HOME`, idle with
`scripts/measure-idle.ps1` (60 s sampled after the start's walk settled):

| Files | Entries | Names | Database | Load at start | Walk, warm | Working set | Private | Idle CPU |
| ----- | ------- | ----- | -------- | ------------- | ---------- | ----------- | ------- | -------- |
| Off | -- | -- | -- | -- | -- | 68.3 MB | 82.5 MB | 0.009 % |
| Home | 54 597 | 1.7 MB | 16 MB | 37 ms | 1.2 s | 73.9 MB | 86.5 MB | 0.004 % |
| Home and `D:\` | 404 974 | 12.0 MB | 110 MB | 200 ms | 15.3 s | 76.7 MB | 100.9 MB | 0.009 % |

The working-set budget (100 MB) holds with room in both. Two figures are
worse than the design assumed, and are recorded rather than rounded (both
taken before slice 2):

- **The database is about 270 bytes an entry**, 110 MB for home and `D:\`,
  not "tens of megabytes". It is a cache on the local disk; the full path is
  stored per row, and storing a parent id instead is the change to make if
  its size matters.
- **Private memory reaches 100.9 MB with `D:\` added**, 18 MB over files off:
  12 MB of names and what the allocator keeps after the walk and the load.
  Private is not the budgeted figure, but it is the one that grows; slice 2
  looks at returning the walk's memory.

The load runs off the UI thread and the applications are searchable before
it ends, so cold start to usable is unchanged. A first idle sample with
`D:\` read 0.022 % because the start's walk had not ended inside 45 s of
settling; with 60 s it read 0.009 %. Every start then walked the scopes
again, which was that cost; since slice 2 a start reconciles instead.

## Slice 2: what was measured

The same build and `LUMEN_HOME`s, with the watcher running:

| Files | Watched folders | Reconcile at start | Working set | Private | Idle CPU |
| ----- | --------------- | ------------------ | ----------- | ------- | -------- |
| Home | 22 | 166 ms, 4 371 folders, 0 re-read | 71.9--72.5 MB | 84.7--86.5 MB | 0.002 % (twice); one run 0.017 % |
| Home and `D:\` | 43 | 2 990 ms, 36 161 folders, 0 re-read | 74.8 MB | 97.2 MB | 0.002 % |

- **A start no longer walks.** Home's 2.5 s walk became a 166 ms check of
  folder times, behind an index installed in 33 ms.
- **A file on the Desktop is found within about 2.5 s** -- the 1.5 s quiet
  period and the batch -- applied in 7 ms; its rename and its deletion the
  same way.
- **The watcher is silent at idle.** One home sample read 0.017 %; a debug
  log over 100 s idle showed no batch applied and two more samples read
  0.002 %, so that sample was the machine, not the index. Excluded subtrees
  are never watched, so `AppData`'s churn is never delivered.
- **Private memory with `D:\`** fell from 100.9 to 97.2 MB once the walk's
  entries were freed before the load.
- `D:\WindowsApps` (the Store's install folder, readable only by the system)
  could not be watched and warned at every start; it joins the skip list.

---

## The decisions

| Question | Decision |
| -------- | -------- |
| Where does it look by default? | The user's home folder. Other internal drives are **offered**, never added silently |
| Can home be removed? | Yes. Home is an ordinary scope; scopes and exclusions are two lists |
| Where does the index live? | Entries on disk, in Lumen's cache folder; names matched from a compact blob in memory (slice 0) |
| How is it kept current? | Watching for changes, plus a reconcile at start and a *Rebuild Index* action |
| What can root search show? | A setting with three levels: *Off*, *Folders*, *Folders and files* |
| Text inside files? | Only in the *Search Files* view, never in root, through the Windows Search index |
| Matching | Applications stay fuzzy. Files and folders match by word start or substring -- no fuzzy, no sensitivity setting |
| Portable applications | Extra application folders, listed as applications, in this phase |
| Network and removable drives, low-disk block | Not in this phase (see "Not in this phase") |

---

## What changes, at a glance

| | Today (F3) | F16 |
| - | ---------- | --- |
| Scope | Six known folders, or `roots` replacing them | Home by default; scopes added and removed; exclusions inside them |
| Folders as results | No | Yes |
| Size | 5 000 entries in memory, depth 6 | Every entry the rules allow, on disk |
| Skip rules | 25 fixed folder names, dot-prefix hidden | Those as defaults, plus `.gitignore`/`.ignore`/`.lumenignore`, user patterns, the Windows hidden attribute |
| Freshness | Re-walked on start and config change | Watched; reconciled at start; rebuilt on demand |
| In root | An aside of up to 8 rows, merged by discounted score | A *Files* section at the end, filled when the index answers |
| Dedicated command | None | *Search Files*, a view with recents, details and actions |
| Content | None | In the view, through Windows Search |

---

## Scopes, drives and exclusions

### Two lists

- **Scopes** -- where to look. `~` (the home folder) by default. Any folder or
  drive root can be added; `~` can be removed like any other.
- **Exclusions** -- folders *inside* the scopes never descended into.

`~` is written as `~` in the file, not resolved, for the reason `roots` is
empty today: a configuration copied to another machine, or a profile
redirected to OneDrive, keeps working.

An empty scope list means **nothing is indexed**, not "home". Home is
removable, so empty has to mean empty; the settings page says so in words
rather than falling back silently.

### Other drives are offered, not taken

Indexing every internal drive by default was considered and rejected. On `C:`
the system's folders are known by the system (Windows, Program Files,
ProgramData, AppData) and could be skipped reliably. On any other drive the
noise is whatever the user installed there -- a Steam library, `D:\Games`,
`E:\XboxGames`, an old Windows -- and skipping it would mean guessing from
folder names. A wrong guess is a first index of several hundred thousand game
assets and a root list full of `.pak` files, with nothing to tell the user
why.

The opposite failure is real too: a user whose files are on `D:` installs
Lumen, searches, finds nothing and concludes file search is broken. So the
other drives are made *visible* instead of automatic:

1. **The first time *Search Files* opens**, a row at the top names the internal
   drives that are not scopes: *"D:\ (Data, 1.8 TB) is not searched -- Add"*,
   with *Not now*. It appears where the user is looking for files, not in a
   dialog at install. *Not now* is remembered per drive.
2. **An empty result explains itself.** When *Search Files* finds nothing and
   internal drives are off, the empty state says which, with *Add* beside each.
3. **The settings page lists every internal drive as a switch**, not as a path
   to type.
4. **What is skipped is shown.** When a drive is added, the default skip rules
   apply to it, and the settings page lists the folders they actually skipped
   on it, each one switchable back on. Nothing is excluded invisibly -- the
   same idea as Raycast's *Excluded Automatically* list.

Only fixed, internal drives are listed. Which drives those are is an OS
question, so it is a `Platform` method (see "Platform changes").

### The skip rules

Applied in this order; any one skips the entry:

1. **Exclusions** from the configuration.
2. **The default skip list** -- today's `SKIPPED` names (`node_modules`,
   `target`, `.git`, `AppData`, ...) plus the system folders of the system
   drive, located through the known-folder paths rather than by name.
3. **Ignore files**, when *Use ignore files* is on (default on): `.gitignore`,
   `.ignore`, Git's exclude file, and `.lumenignore` -- Lumen's own, for
   excluding a folder without touching a repository's Git configuration.
4. **Ignore patterns** from the configuration: globs, global, for example
   `*.log, **/build/**`.
5. **Hidden entries**, unless *Include hidden files* is on: a leading dot *and*
   the Windows hidden attribute.

Rules 3 to 5 are what ripgrep's `ignore` crate implements, including the
hidden attribute on Windows, so the walker uses it rather than re-implementing
`.gitignore` semantics. That keeps I-1 intact in `lumen-core` -- the attribute
is read inside the dependency, as `std::fs` and `notify` already do -- and
replaces F3's dot-only rule, whose own comment names the attribute as the gap.
*To verify in slice 1:* the crate's Windows hidden-attribute behaviour.

*Exclude from Index*, an action on any file or folder, appends the rule to the
nearest `.lumenignore` (creating one in the parent folder if there is none)
and drops the entries at once. It is how most exclusions should be made:
from the result that annoyed you, not from a settings page.

---

## The index

### Entries on disk, names in memory

An index is the list of every name and path the rules allow, built once so a
keystroke never touches the disk. F3 keeps it in memory with the matcher's
prepared haystacks for every entry, which is affordable at 5 000 and not at
the hundreds of thousands a second drive can hold -- the idle budget is
100 MB for everything.

So it is split (slice 0 measured why):

- **The entries** are a SQLite database in Lumen's cache folder
  (`Paths::cache_dir`, local, not roaming: the index is rebuilt from the disk
  and has no business following the user between machines). SQLite is
  already in the build for the clipboard history (F3, F12). A row holds the
  path, the name, the parent, folder or file, size, modification time, and
  which scope it came from.
- **The names** are one lower-cased, normalised blob with an offset table,
  about 30 bytes an entry, rebuilt from the database at start in
  milliseconds. Only the matches of a query get the matcher's prepared form.

### How a query is answered

Fuzzy matching every name on every keystroke is linear, and 7.20 measured
what linear costs: 7.42 ms at 20 000 entries. A million is out of the
question. So files and folders are matched by **word start or substring** --
`inv` finds `Invoice-2026.pdf` and `2026-invoices`, but `ive` scattered across
a path does not -- and the database narrows candidates through an index
before anything is scored:

- Three characters or more: an FTS5 *trigram* index answers substring
  matches.
- One or two characters: a prefix range over the stored names.

The narrowed candidates, a few hundred at most, are then ranked with the
existing policy -- match quality and frecency -- off the UI thread.

This is also why there is no *search sensitivity* setting. Raycast's controls
how loose the fuzzy matcher is for all of root; its purpose is to suppress
scattered-letter matches. Matching files by word start and substring removes
those for the one source that has hundreds of thousands of them, and leaves
applications as fuzzy as they are today.

**Decided by measurement** (slice 0, above): not FTS5 trigram but the names
in memory -- one blob, about 30 bytes an entry, scanned with `memmem` -- with
the entries themselves in SQLite. The prefix and trigram paths described
above were the SQLite candidate and are not built.

### Kept current

- **Watching.** `notify` (already the live-reload watcher, AD-05) reports
  changes; the callback pushes and returns (I-15), and changes are applied in
  batches once a burst settles, as 7.34 does for applications.
- **Never a watch over an excluded subtree.** A recursive watch on a folder
  reports every change beneath it, skipped or not, and a home folder contains
  `AppData`, where browsers write constantly. So each scope gets a
  non-recursive watch on its root (to see new top-level folders) and a
  recursive watch on each child that is not skipped. `AppData`'s churn is
  never delivered, so it costs nothing to ignore. Whether this holds idle CPU
  under 0.02 % is measured in slice 2, not assumed.
- **Reconcile at start.** While the daemon was not running nothing was
  watched. At start, every indexed folder's modification time is compared
  with the one stored, and only folders whose time moved are read again --
  on NTFS a folder's time changes when an entry is added, removed or renamed
  in it. The index is usable from the first moment; the reconcile runs
  behind it.
- ***Rebuild Index*** -- an action in *Search Files* and a button on the
  settings page -- throws the index away and walks again. The answer to
  "something is wrong", not the way freshness works.

Changing scopes, exclusions or rules re-walks only what they affect.

### Indexing is always in the background

Asyar's *Run indexing in background* has no Lumen equivalent because there is
no foreground mode to switch to: nothing in the launcher waits on the index
(I-3). What the user needs is to know it is happening -- *"Indexing... 42 310
items"* in the *Search Files* footer and on the settings page, and the count
and the time of the last complete pass when it is not.

---

## Root search

### Three levels

| Level | Root shows |
| ----- | ---------- |
| *Off* | No files or folders. *Search Files* still works |
| *Folders* | Folders only -- the cheapest and often the most useful: `proj` reaches `D:\Dev\projects` |
| *Folders and files* | Both. **The default**, because F3 shows files in root today and removing them would be a regression |

Text inside files is deliberately **not** a level. A content match is slow,
so it would always arrive late, and it is noisy: `invoice` would list every
document containing the word and bury the application being looked for.
Raycast does not put content in root either.

### A section at the end, filled when the index answers

Root search never waits (I-3), and the index is now a query to another
thread, not a pass over memory. So:

- The root list is drawn at once from everything else.
- The file query starts in parallel. When it answers, its rows fill a
  **Files** section **after** every other row, capped at eight as asides are
  today (7.21).
- The selection never moves to a row because it arrived. Rows only ever
  append below; nothing above shifts (I-10).
- An answer for a query that is no longer the current one is dropped, and
  the scan behind it stops: every keystroke cancels the one before. Until
  the new answer arrives, the section keeps the previous rows that still
  match what is typed.

The consequence, accepted: a file can no longer outrank an application, as
7.26's discount allowed for a clear match. A late row that jumped above the
selection would be worse than a perfect file match sitting lower.

### One or two letters search what you use

Slice 1a measured the one case the names cannot answer quickly: a query of
one or two letters, whose first byte is in nearly every name. At 350 000
entries that is 4.4 ms at p95, about 6 ms at the ceiling -- off the UI
thread, but work spent on rows nobody wants: with `a` typed, a never-opened
file that happens to start with *a* is noise. So in root:

| Query | Files and folders searched |
| ----- | -------------------------- |
| One or two letters | Those with a frecency score -- opened from Lumen -- and those whose whole name is one or two letters (`a`, `go`), used or not. A few hundred entries: under 0.05 ms |
| Three letters or more | The whole index: at most 1.3 ms at 350 000, about 1.5 ms at the ceiling |

Every name stays reachable: a short name by the exact-name rule, any other
by its first three letters. *Search Files* searches the whole index at any
length, where waiting is allowed. Threads were considered and not built:
they spend the same CPU to shorten a wait this rule removes.

The target is therefore that file rows arrive within the frame after the
keystroke at the ceiling -- not a figure the per-frame budget depends on,
since the scan never runs in a frame. Files opened outside Lumen do not
count as used; reading the system's recent items to seed frecency is a
possible later addition.

Empty query: no files, as today (7.21). Recents belong to the view.

---

## The *Search Files* view

A built-in view, reached as a command in root (*Search Files*), with an alias
and a hotkey like any command.

- **Empty query:** recently opened files and folders, from frecency.
- **Typing:** the same name index as root, without root's eight-row cap.
- **Details:** a pane with path, kind, size, dates and, for an image, a
  preview -- the vocabulary F10 to F13 already use.
- **Contents:** a toggle in the view's filter (the *Type* filter's
  pattern from F12) that adds content matches **after** name matches, from
  Windows Search.
- **Status:** indexing progress or the last pass, in the footer.
- **Empty states:** "nothing indexed" when there are no scopes, and the drive
  suggestions described above.

**Actions** (on files and folders, in the view and in root):

| Action | Notes |
| ------ | ----- |
| Open | Default application; a folder opens in Explorer |
| Show in Explorer | Selects the item in its folder |
| Copy Path | Today's only secondary action |
| Open With... | The system's chooser |
| Exclude from Index | Writes `.lumenignore` (above) |
| Rebuild Index | View only |

Later, not in this phase: *Save as Quicklink*, typing a path to browse a
folder, sorting a folder.

---

## Content search through Windows Search

Raycast does not index file contents itself either: it asks Spotlight or
Windows Search, and greys its toggle out when the system's index does not
cover the scopes. Lumen does the same, for the same reason -- a content index
of its own would be large on disk, slow to build, and a threat to the size
and memory budgets.

- A `Platform` method runs the query. On Windows it asks the system index
  (`SystemIndex`, through the Windows Search OLE DB provider), restricted to
  the scopes, and returns paths.
- It is called only from the view, off the UI thread, where waiting is
  allowed. Root never calls it (I-3).
- **Coverage is shown per scope.** By default Windows indexes the user's
  folders and not `D:\`. The settings page marks each scope *covered* or *not
  covered by Windows Search*, with a button that opens Windows' *Indexing
  Options*. A scope that is not covered simply contributes no content
  matches.
- Linux: the method is `todo!()` until F5 resumes (I-2); the choice there
  (Tracker, Baloo, or a streaming scan) is F5's.

**Revises 7.19.** "Not the system index" stays true for names, and becomes
an exception for content. The reasons in 7.19 (a COM path with no safe
binding, an answer root cannot wait for) are why it stays out of root search
and out of the name index.

---

## Extra application folders

Vicinae lets the user name folders whose files count as applications. On
Windows that is the portable-application problem: `D:\Tools\Obsidian\
Obsidian.exe` is not in `AppsFolder` and never appears in Lumen today.

- `apps.extra-folders` -- a list of folders, empty by default.
- In each, `.exe`, `.lnk` and `.url` files up to two levels down are listed as
  **applications** (`Kind::Application`), named by file name, not as files.
  Two levels covers `Tools\Obsidian\Obsidian.exe`; deeper is where installers
  hide their helpers.
- Launching and icons need nothing new: `LaunchTarget::ShellExecute` and
  `IconSource::ShellItem` already accept a path to an `.exe`.
- An entry whose target `AppsFolder` already lists is not listed twice.
- Watched like the Start Menu folders (7.34): small folders, a cheap watch.
- On Linux this is the list of extra `.desktop` directories, at F5.

It lives with the application index, not with file search: its rows are
applications, ranked as applications, and it works with file search off.

---

## Configuration

```json
"files": {
  "enabled": true,
  "in-root": "folders-and-files",
  "scopes": ["~"],
  "exclusions": ["~/Videos"],
  "ignore-patterns": ["*.log", "**/build/**"],
  "use-ignore-files": true,
  "include-hidden": false,
  "declined-drives": ["E:/"]
},
"apps": {
  "extra-folders": ["D:/Tools"]
}
```

- `in-root`: `"off"`, `"folders"`, `"folders-and-files"`.
- `declined-drives`: the drives answered *Not now*, so they are not offered
  again. Any drive can still be added from settings.
- **Old configurations still load.** `roots`, if present, is read as `scopes`
  -- it replaced the defaults, and a scope list does the same. `limits` is no
  longer used; it is read, ignored and logged once, never an error.

## The settings page

A *File Search* page in the settings window (F15's navigation), in four
groups:

1. **In root search** -- the three levels, as a segmented choice.
2. **Where** -- scopes with *Add Folder*; internal drives as switches; the
   exclusions; per drive, *Skipped automatically* with a switch each. Above
   the list, the memory note ("Against the budgets"); beside each scope its
   item count, and the total against the 500 000 ceiling.
3. **What** -- *Use ignore files*, *Include hidden files*, ignore patterns.
4. **Index** -- status (items, last pass, or progress), *Rebuild Index*; per
   scope, Windows Search coverage with *Indexing Options*.

*Extra application folders* goes on the page that owns applications; if
none exists when slice 7 lands, at the end of this one, under its own heading.

---

## Platform changes

| Method | Why it is OS-specific |
| ------ | --------------------- |
| `internal_drives()` -- root, label, size | Drive types are an OS question (I-1) |
| `search_content(query, scopes, limit)` | Windows Search |
| `content_coverage(scopes)` | Whether Windows Search indexes each scope |
| `open_indexing_options()` | Opens the system's settings |
| *Show in Explorer*, *Open With...* | Shell calls, if `launch` cannot express them |

Each lands with its Linux stub in the same commit (I-2, I-9). Walking and
watching need nothing new: `std::fs`, `ignore` and `notify` already sit
behind I-1.

## Invariants and budgets

| | How F16 keeps it |
| - | ---------------- |
| I-1 | Walk and watch through crates already allowed; everything else through `Platform` |
| I-3 | Root's file rows arrive when the index answers; content search is never called from root |
| I-5 | The section header, pane and banner use existing tokens; any new geometry goes into `layout` |
| I-10 | File rows only append; the selection never follows an arrival |
| I-15 | The watcher's callback pushes and returns |
| Per-frame p95 < 8 ms | The query runs off the UI thread; slice 0 sets its own target, p95 < 4 ms |
| Idle memory < 100 MB | The index is on disk; measured with a large index in slice 0 and again in slice 2 |
| Idle CPU < 0.02 % | Measured with the watcher on a real home folder in slice 2 |
| Cold start < 400 ms | The index is opened, never built, at start; the reconcile runs behind it |
| Installed size < 40 MB | SQLite is already in; `ignore` is small. Measured at slice 1 |

**Revisions to `ARCHITECTURE.md`**, written when the slice that causes them
lands:

- **7.19** -- names stay Lumen's own index; content is the exception (slice 6).
- **7.20** -- the 5 000 cap is replaced by a query-time budget (slice 1).
- **7.21** -- files leave the asides for an appended section; still never
  browsed in root (slice 1).
- **7.26** -- the aside discount no longer applies to files (slice 1).

---

## Slices

Measurement first, then each user-visible step on its own.

| # | Slice | Done when |
| - | ----- | --------- |
| 0 | **Measure.** A synthetic index of 100 000 and 1 000 000 names; FTS5 trigram against an in-memory name table; query p95 per query length, resident memory, size on disk. Also the real entry count of this machine's home folder under the skip rules | The storage is chosen by the numbers, recorded in this file |
| 1 | **The index.** Schema, the walker with the skip rules, scopes and exclusions with the old `roots` migration, the 500 000 ceiling, folders as results, root's appended *Files* section, `Kind::Folder` | Root finds files and folders from the index; F3's walk is gone. Measured: idle working set with home, and with home and `D:\` (`measure-idle.ps1`); find-and-rank latency at 55 000, 350 000 and 1 000 000; time to load the names at start |
| 2 | **Fresh.** Watching without excluded subtrees, the start reconcile, *Rebuild Index* | A file created on the Desktop is found within seconds; idle CPU measured with the watcher on |
| 3 | ***Search Files*.** The view, recents, details, actions, *Exclude from Index*, status | The view finds what root finds and more, with no cap |
| 4 | **Settings.** The page, the three levels, ignore files, patterns, hidden | Every key in "Configuration" is settable without editing JSON |
| 5 | **Drives.** `internal_drives`, the first-view offer, the empty-result suggestion, drive switches, the list of folders skipped on each drive | A drive can be found, added and pruned without typing a path |
| 6 | **Contents.** Windows Search in the view, coverage per scope | Text inside a document in a covered scope is found from the view |
| 7 | **Extra application folders** | A portable `.exe` is listed and launched as an application |

Slices 0 to 2 are the risky ones, and each can stop the phase: if no storage
meets slice 0's target, or the watcher cannot meet the idle budget, the
design changes before anything is built on it.

## Not in this phase

- **Network and removable drives**, and *Keep files from removed volumes*.
  Rows record their scope, so adding a volume identity later is a column, not
  a redesign.
- **The low-disk-space block.** It matters for Raycast because content
  indexes grow large. A name index of a home folder is about 16 MB, and
  content is the system's index, not Lumen's.
- **A search sensitivity setting** -- see "How a query is answered".
- **Content in root search** -- see "Three levels".
- Browsing a typed path, sorting a folder, *Save as Quicklink* on a file.

## Open questions, answered by measurement

- Which storage meets the 4 ms query target and the memory budget? (slice 0)
- How many entries does a real home folder have after the skip rules?
  (slice 0)
- Does the watcher, built without excluded subtrees, keep idle CPU under
  0.02 %? (slice 2)
- How long is a first full index of a home folder, and of a large second
  drive? It decides whether the offer row warns about time. (slices 1 and 5)
