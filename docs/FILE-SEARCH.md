# File search, the rest of the way (F16)

**Status: designed, not started.** This is the plan. When a slice lands, its
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

## The decisions

| Question | Decision |
| -------- | -------- |
| Where does it look by default? | The user's home folder. Other internal drives are **offered**, never added silently |
| Can home be removed? | Yes. Home is an ordinary scope; scopes and exclusions are two lists |
| Where does the index live? | On disk, in Lumen's cache folder. Not in memory |
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

### On disk, because of the memory budget

An index is the list of every name and path the rules allow, built once so a
keystroke never touches the disk. F3 keeps it in memory, and at 5 000 entries
that is one or two megabytes. A home folder on a developer's machine is
plausibly 100 000 to 1 000 000 entries *after* the skip rules (an estimate,
measured in slice 0). In memory, with the matcher's prepared haystacks, that
is tens to hundreds of megabytes -- the idle budget is 100 MB for everything.

So the index is a SQLite database in Lumen's cache folder
(`Paths::cache_dir`, which is local, not roaming: the index is rebuilt from
the disk and has no business following the user between machines). SQLite is already in the build for the
clipboard history (F3, F12); its page cache is bounded, so resident memory
does not grow with the index.

What a row holds: the path, the name, the parent, folder or file, size,
modification time, and which scope it came from. The name is also stored in
a form for matching (lower-cased, normalised).

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

**Decided by measurement, not here:** FTS5 trigram against an in-memory
table of names alone (roughly 30--40 bytes an entry, no haystacks). Slice 0
measures both on a synthetic million-entry index. The design depends only on
"a query answers in under 4 ms at p95 and idle memory stays in budget"; the
storage that meets it wins.

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
- An answer for a query that is no longer the current one is dropped. Until
  the new answer arrives, the section keeps the previous rows that still
  match what is typed.

The consequence, accepted: a file can no longer outrank an application, as
7.26's discount allowed for a clear match. A late row that jumped above the
selection would be worse than a perfect file match sitting lower. If the
query answers within the frame, which is slice 0's target, the user never
sees the section arrive.

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
   exclusions; per drive, *Skipped automatically* with a switch each.
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
| 1 | **The index.** Schema, the walker with the skip rules, scopes and exclusions with the old `roots` migration, folders as results, root's appended *Files* section, `Kind::Folder` | Root finds files and folders from the index; F3's walk is gone; budgets hold |
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
  indexes grow large. A name index of a home folder is tens of megabytes, and
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
