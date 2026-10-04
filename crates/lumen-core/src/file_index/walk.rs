//! Walking the scopes with the skip rules (F16).
//!
//! # The rules are `ignore`'s, with Lumen's on top
//!
//! Ignore files, hidden entries and glob patterns are what ripgrep's `ignore`
//! crate implements, including the Windows hidden attribute, which it reads
//! itself -- so this crate makes no OS call for it (I-1). What it does not
//! know is Lumen's: the folders never worth indexing ([`SKIPPED`]), the
//! user's exclusions, and the scopes that overlap. Those are one filter on
//! the walk, applied before a folder is descended into, so a skipped tree
//! costs one directory read and nothing beneath it.
//!
//! # In order, and never twice
//!
//! Scopes are walked in the order they are listed, each to its end: the
//! index has no ceiling (`docs/FILE-SEARCH.md`, "Decision: B"), since
//! memory holds only a hot layer of it. A scope inside one already walked adds nothing when
//! that walk reaches it; a scope *around* one already walked lists its
//! folder but does not walk it again. Either way an entry is found once.
//!
//! # A scope added on purpose beats every rule
//!
//! The rules apply below a scope, never to its own folder: the user named
//! it. So a scope the walk of an outer one would not reach -- a folder
//! between them is skipped, excluded, hidden or ignored, as `AppData` is
//! from `~` -- is walked on its own, in its turn, with exclusions around
//! it set aside. Whether the outer walk reaches it is asked of the same
//! rules, one directory read a level ([`reaches`]), never of a second copy.
//! One rule still beats a scope: an exclusion of that very folder. Both are
//! the user's, and the exclusion is the narrower intent.

use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;

/// What to walk and what to leave out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkRules {
    /// Where to look, in order, as absolute paths. `~` is resolved before
    /// it reaches here.
    pub scopes: Vec<PathBuf>,
    /// Folders inside the scopes never descended into, nor listed.
    pub exclusions: Vec<PathBuf>,
    /// Gitignore-style patterns, applied in every scope: `*.log`,
    /// `**/build/**`.
    pub ignore_patterns: Vec<String>,
    /// `.gitignore` (inside a Git repository), `.ignore`, Git's exclude file
    /// and `.lumenignore`.
    pub use_ignore_files: bool,
    /// A leading dot, or the Windows hidden attribute.
    pub include_hidden: bool,
    /// Names from [`default_skips`] the user turned back on, lower-cased.
    pub skips_off: Vec<String>,
}

/// One file or folder the walk found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub is_dir: bool,
    /// A folder's modification time ([`modified_time`]), read before the
    /// walk read what is inside, so a change made while it did is seen at
    /// the next reconcile. `None` for a file.
    pub modified: Option<i64>,
}

/// Everything the walk found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Walked {
    /// Scope by scope, in the order of the rules. Within a scope, a folder's
    /// entries by name.
    pub entries: Vec<Entry>,
    /// How many entries each scope gave, one count for each scope of the
    /// rules: 0 for one that was missing, inside another, or never reached.
    pub per_scope: Vec<usize>,
    /// Each scope's own modification time, taken before it was walked: the
    /// scope is not an entry, but the start reconcile compares its time too.
    pub roots: Vec<Option<i64>>,
}

/// Directory names never descended into, unless the user turned one back on
/// ([`WalkRules::skips_off`]).
///
/// Matched exactly and case-insensitively against the directory's own name, not
/// against the path, so a folder the user actually named `Build` is skipped
/// too. That is the right trade: the cost of skipping one folder is one file
/// not found, and the cost of descending into a `node_modules` is an index
/// made of nothing else. Spelled as Windows spells them, for the settings
/// page that lists them.
pub(crate) const SKIPPED: &[&str] = &[
    // Dependency and build trees.
    "node_modules",
    "target",
    "build",
    "dist",
    "out",
    "obj",
    "bin",
    "vendor",
    "__pycache__",
    ".venv",
    "venv",
    "env",
    // Version control and tooling state.
    ".git",
    ".svn",
    ".hg",
    ".cache",
    ".gradle",
    ".idea",
    ".vs",
    ".vscode",
    // Toolchains and package caches in the home folder: with hidden files
    // on, these were most of a home index. Whole, so `.cargo\bin` goes too.
    ".rustup",
    ".cargo",
    ".npm",
    ".nuget",
    ".pnpm-store",
    ".yarn",
    ".bun",
    ".deno",
    ".m2",
    ".conda",
    ".docker",
    // Windows' own. `AppData` alone is larger than everything the user made.
    "AppData",
    "Application Data",
    "$Recycle.Bin",
    "System Volume Information",
    // Where the Store and Xbox install to on a second drive: readable by
    // nobody but the system, so it is never anything to find, and a watch
    // on it fails at every start.
    "WindowsApps",
];

/// The default skip list, for the settings page to show and switch: a new
/// default reaches everyone, and only what was turned off is stored.
pub fn default_skips() -> &'static [&'static str] {
    SKIPPED
}

/// Whether a folder named `name` is skipped: on the default list and not
/// turned back on in `skips_off`.
pub(crate) fn is_skipped(name: &str, skips_off: &[String]) -> bool {
    SKIPPED
        .iter()
        .any(|skipped| name.eq_ignore_ascii_case(skipped))
        && !skips_off.iter().any(|off| name.eq_ignore_ascii_case(off))
}

/// When `path` was last modified, in nanoseconds since 1970; `None` when it
/// cannot be read.
///
/// What the start reconcile compares: on NTFS a folder's time moves when an
/// entry is added to it, removed from it or renamed in it, and not when a
/// file inside is only written to -- which the index does not record anyway.
pub fn modified_time(path: &Path) -> Option<i64> {
    std::fs::metadata(path).ok().as_ref().and_then(modified)
}

fn modified(metadata: &std::fs::Metadata) -> Option<i64> {
    let since = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    i64::try_from(since.as_nanos()).ok()
}

/// The positions of the scopes that are walked, in order: every scope but
/// one excluded itself, or one whose folder the walk of a scope listed
/// before it reaches, which adds nothing (see "In order, and never twice"
/// and "A scope added on purpose beats every rule" above).
///
/// Reads a directory for each level between a scope and one inside it, so
/// it runs where the walk does, never where anything waits (I-3). Without
/// nested scopes it reads nothing.
pub(crate) fn effective_scopes(rules: &WalkRules) -> Vec<usize> {
    let mut effective: Vec<usize> = Vec::new();
    for (index, scope) in rules.scopes.iter().enumerate() {
        if rules.exclusions.contains(scope) {
            continue;
        }
        let walked = effective
            .iter()
            .map(|&earlier| rules.scopes[earlier].clone())
            .collect::<Vec<_>>();
        if !walked
            .iter()
            .any(|outer| reaches(outer, scope, rules, &walked))
        {
            effective.push(index);
        }
    }
    effective
}

/// Whether the walk of `outer`, beside the scopes `walked`, lists `inner`:
/// every folder from the first below `outer` down to `inner` itself is one
/// the rules allow. Asked of the rules a level at a time, as the walk
/// meets them, and given up at the first refused.
fn reaches(outer: &Path, inner: &Path, rules: &WalkRules, walked: &[PathBuf]) -> bool {
    let Ok(rest) = inner.strip_prefix(outer) else {
        return false;
    };
    let mut folder = outer.to_path_buf();
    for part in rest.components() {
        let next = folder.join(part);
        let mut builder = walker(&folder, outer, rules, walked);
        builder.max_depth(Some(1));
        let listed = builder.build().flatten().any(|found| {
            found.depth() == 1
                && !found.path_is_symlink()
                && found.file_type().is_some_and(|kind| kind.is_dir())
                && found.path() == next
        });
        if !listed {
            return false;
        }
        folder = next;
    }
    true
}

/// The entries directly in `folder`, by the rules, as a walk of `scope`
/// would find them -- the scope matters for its ignore patterns, which are
/// written relative to it -- leaving out what is inside `nested`, the scopes
/// walked on their own. `folder` itself is taken as allowed.
///
/// One directory read: how a change is applied, and a folder whose time
/// moved is read again, without walking anything else.
pub(crate) fn list(
    folder: &Path,
    scope: &Path,
    rules: &WalkRules,
    nested: &[PathBuf],
) -> Vec<Entry> {
    let mut builder = walker(folder, scope, rules, nested);
    builder.max_depth(Some(1));
    entries_below(builder)
}

/// Everything below `folder` by the rules, as [`list`] reads one level.
pub(crate) fn walk_below(
    folder: &Path,
    scope: &Path,
    rules: &WalkRules,
    nested: &[PathBuf],
) -> Vec<Entry> {
    entries_below(walker(folder, scope, rules, nested))
}

/// The entries of a walk below its root.
fn entries_below(builder: WalkBuilder) -> Vec<Entry> {
    let mut entries = Vec::new();
    for result in builder.build() {
        let Ok(found) = result else {
            continue;
        };
        if found.depth() == 0 || found.path_is_symlink() {
            continue;
        }
        entries.push(entry(found));
    }
    entries
}

/// An entry of the index from one of the walk's, with a folder's time.
fn entry(found: ignore::DirEntry) -> Entry {
    let is_dir = found.file_type().is_some_and(|kind| kind.is_dir());
    // From the folder itself, not from the listing that found it: NTFS
    // updates the copy of a folder's times in its parent's listing lazily,
    // and the reconcile must compare the time it reads with one read the
    // same way, or it reads folders again for nothing.
    let modified = if is_dir {
        modified_time(found.path())
    } else {
        None
    };
    Entry {
        is_dir,
        modified,
        path: found.into_path(),
    }
}

/// Walk every scope of `rules`, in order, each to its end.
///
/// Touches the disk for as long as it takes -- seconds for a home folder,
/// tens of them for a large drive -- so it is never called where anything
/// waits on it (I-3). Entries that cannot be read are skipped rather than
/// reported: a permissions error somewhere under Documents is not something
/// the user can act on, nor a reason to index nothing.
pub fn walk(rules: &WalkRules) -> Walked {
    let mut walked = Walked {
        per_scope: vec![0; rules.scopes.len()],
        roots: vec![None; rules.scopes.len()],
        ..Walked::default()
    };
    let mut done: Vec<PathBuf> = Vec::new();

    for index in effective_scopes(rules) {
        let scope = &rules.scopes[index];
        walked.roots[index] = modified_time(scope);
        for result in walker(scope, scope, rules, &done).build() {
            let Ok(found) = result else {
                continue;
            };
            // The scope's own folder is where the user looks *from*, not a
            // result. A link is not followed and not listed: one back up its
            // own tree is an endless walk, and its target is almost always
            // found by its real path anyway.
            if found.depth() == 0 || found.path_is_symlink() {
                continue;
            }
            walked.entries.push(entry(found));
            walked.per_scope[index] += 1;
        }
        done.push(scope.clone());
    }

    walked
}

/// The walker for `root`, in `scope`, with every rule but Lumen's applied by
/// `ignore` and Lumen's in its entry filter. `root` is the scope itself
/// except when a change is applied, which reads one folder of it. Of `done`,
/// the scopes walked on their own, only those inside `scope` matter; of the
/// exclusions, only those that are not `scope` or around it.
fn walker(root: &Path, scope: &Path, rules: &WalkRules, done: &[PathBuf]) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    let ignore_files = rules.use_ignore_files;
    builder
        .hidden(!rules.include_hidden)
        .parents(ignore_files)
        .ignore(ignore_files)
        .git_ignore(ignore_files)
        .git_exclude(ignore_files)
        // The user's global Git ignore file is about their repositories'
        // build output, not about which of their files they search.
        .git_global(false)
        .follow_links(false)
        .max_depth(None)
        // By name, so the order of the index, and with it the ids, is the
        // same from one walk of an unchanged disk to the next.
        .sort_by_file_name(|a, b| a.cmp(b));
    if ignore_files {
        builder.add_custom_ignore_filename(".lumenignore");
    }

    // Overrides are `ignore`'s globs given in code. A plain one would mean
    // *only* what it matches, so each is negated: an ignore, as in a
    // `.gitignore` line.
    let mut overrides = OverrideBuilder::new(scope);
    for pattern in &rules.ignore_patterns {
        if let Err(error) = overrides.add(&format!("!{pattern}")) {
            log::warn!("ignoring the file pattern {pattern:?}: {error}");
        }
    }
    match overrides.build() {
        Ok(overrides) => {
            builder.overrides(overrides);
        }
        Err(error) => log::warn!("ignoring the file patterns: {error}"),
    }

    let exclusions = rules
        .exclusions
        .iter()
        .filter(|excluded| !scope.starts_with(excluded))
        .cloned()
        .collect::<Vec<_>>();
    let done = done
        .iter()
        .filter(|other| other.starts_with(scope) && *other != scope)
        .cloned()
        .collect::<Vec<_>>();
    let skips_off = rules.skips_off.clone();
    builder.filter_entry(move |entry| {
        if entry.depth() == 0 {
            return true;
        }
        let path = entry.path();
        if exclusions.iter().any(|excluded| path.starts_with(excluded)) {
            return false;
        }
        // Inside a scope already walked: its folder is listed, its contents
        // already were.
        if done
            .iter()
            .any(|earlier| path.starts_with(earlier) && path != earlier)
        {
            return false;
        }
        let is_dir = entry.file_type().is_some_and(|kind| kind.is_dir());
        !(is_dir && is_skipped(&entry.file_name().to_string_lossy(), &skips_off))
    });
    builder
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A small tree under a unique temporary directory, removed when dropped.
    /// No fixture crate: this is twenty lines and one dependency fewer.
    pub(crate) struct Tree(pub PathBuf);

    impl Tree {
        pub fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("lumen-file-index-{name}"));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        pub fn file(&self, relative: &str) -> &Self {
            self.write(relative, "x")
        }

        pub fn write(&self, relative: &str, contents: &str) -> &Self {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, contents).unwrap();
            self
        }

        pub fn dir(&self, relative: &str) -> &Self {
            std::fs::create_dir_all(self.0.join(relative)).unwrap();
            self
        }

        pub fn path(&self, relative: &str) -> PathBuf {
            self.0.join(relative)
        }

        /// The default rules, with this tree as the one scope.
        pub fn rules(&self) -> WalkRules {
            rules(std::slice::from_ref(&self.0))
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub(crate) fn rules(scopes: &[PathBuf]) -> WalkRules {
        WalkRules {
            scopes: scopes.to_vec(),
            exclusions: Vec::new(),
            ignore_patterns: Vec::new(),
            use_ignore_files: true,
            include_hidden: false,
            skips_off: Vec::new(),
        }
    }

    /// The found paths relative to `root`, with `/` separators and a folder
    /// marked by a trailing one, sorted.
    fn relative(walked: &Walked, root: &Path) -> Vec<String> {
        let mut found = walked
            .entries
            .iter()
            .map(|entry| {
                let rest = entry.path.strip_prefix(root).unwrap();
                let rest = rest
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                if entry.is_dir {
                    format!("{rest}/")
                } else {
                    rest
                }
            })
            .collect::<Vec<_>>();
        found.sort();
        found
    }

    #[test]
    fn files_and_folders_are_entries_but_the_scope_is_not() {
        let tree = Tree::new("walk-basic");
        tree.file("notes.md").file("projects/invoice.pdf");

        let walked = walk(&tree.rules());

        assert_eq!(
            relative(&walked, &tree.0),
            ["notes.md", "projects/", "projects/invoice.pdf"]
        );
        assert_eq!(walked.per_scope, [3]);
    }

    /// No depth limit: F3 stopped at six, the index does not.
    #[test]
    fn the_walk_has_no_depth_limit() {
        let tree = Tree::new("walk-deep");
        tree.file("a/b/c/d/e/f/g/h/deep.txt");

        let walked = walk(&tree.rules());
        assert!(relative(&walked, &tree.0).contains(&"a/b/c/d/e/f/g/h/deep.txt".to_string()));
    }

    #[test]
    fn dependency_and_build_directories_are_never_descended_into() {
        let tree = Tree::new("walk-skips");
        tree.file("app/main.rs")
            .file("app/node_modules/left-pad/index.js")
            .file("app/Target/debug/build.log")
            .file("AppData/Local/thing.db");

        assert_eq!(
            relative(&walk(&tree.rules()), &tree.0),
            ["app/", "app/main.rs"]
        );
    }

    /// A scope is walked even when its own name is on the skip list.
    #[test]
    fn a_scope_named_like_a_skipped_folder_is_still_walked() {
        let tree = Tree::new("walk-build");
        tree.file("build/report.txt");

        let scope = tree.path("build");
        assert_eq!(
            relative(&walk(&rules(std::slice::from_ref(&scope))), &scope),
            ["report.txt"]
        );
    }

    #[test]
    fn dotted_entries_are_hidden_unless_asked_for() {
        let tree = Tree::new("walk-hidden");
        tree.file(".env")
            .file(".config/app.toml")
            .file("readme.txt");

        assert_eq!(relative(&walk(&tree.rules()), &tree.0), ["readme.txt"]);

        let mut all = tree.rules();
        all.include_hidden = true;
        assert_eq!(
            relative(&walk(&all), &tree.0),
            [".config/", ".config/app.toml", ".env", "readme.txt"]
        );
    }

    #[test]
    fn an_exclusion_and_everything_under_it_is_left_out() {
        let tree = Tree::new("walk-exclusions");
        tree.file("keep.txt")
            .file("Videos/clip.mp4")
            .file("Videos/raw/take1.mov");

        let mut rules = tree.rules();
        rules.exclusions = vec![tree.path("Videos")];
        assert_eq!(relative(&walk(&rules), &tree.0), ["keep.txt"]);
    }

    #[test]
    fn ignore_patterns_are_applied_like_gitignore_lines() {
        let tree = Tree::new("walk-patterns");
        tree.file("keep.txt")
            .file("debug.log")
            .file("deep/trace.log")
            .file("project/build/out.bin")
            .file("project/src/lib.rs");

        let mut rules = tree.rules();
        rules.ignore_patterns = vec!["*.log".into(), "**/project/src/**".into()];
        assert_eq!(
            relative(&walk(&rules), &tree.0),
            ["deep/", "keep.txt", "project/", "project/src/"]
        );
    }

    #[test]
    fn a_lumenignore_excludes_what_it_names_unless_ignore_files_are_off() {
        let tree = Tree::new("walk-lumenignore");
        tree.write("docs/.lumenignore", "drafts/\n*.tmp\n")
            .file("docs/report.md")
            .file("docs/scratch.tmp")
            .file("docs/drafts/one.md");

        assert_eq!(
            relative(&walk(&tree.rules()), &tree.0),
            ["docs/", "docs/report.md"]
        );

        let mut off = tree.rules();
        off.use_ignore_files = false;
        assert_eq!(
            relative(&walk(&off), &tree.0),
            [
                "docs/",
                "docs/drafts/",
                "docs/drafts/one.md",
                "docs/report.md",
                "docs/scratch.tmp"
            ]
        );
    }

    /// `.gitignore` applies inside a Git repository only, as Git itself does.
    /// A `.git` folder is enough for `ignore` to call it one.
    #[test]
    fn a_gitignore_applies_inside_a_repository_only() {
        let tree = Tree::new("walk-gitignore");
        tree.dir("repo/.git")
            .write("repo/.gitignore", "secret.txt\n")
            .file("repo/secret.txt")
            .file("repo/main.rs")
            .write("loose/.gitignore", "secret.txt\n")
            .file("loose/secret.txt");

        assert_eq!(
            relative(&walk(&tree.rules()), &tree.0),
            ["loose/", "loose/secret.txt", "repo/", "repo/main.rs"]
        );
    }

    /// Scopes overlap -- `~` and `~/Documents` -- and an entry found twice is
    /// a duplicate row.
    #[test]
    fn a_scope_inside_an_earlier_one_or_repeated_adds_nothing() {
        let tree = Tree::new("walk-nested");
        tree.file("Documents/notes.md").file("top.txt");

        let walked = walk(&rules(&[
            tree.0.clone(),
            tree.path("Documents"),
            tree.0.clone(),
        ]));

        assert_eq!(
            relative(&walked, &tree.0),
            ["Documents/", "Documents/notes.md", "top.txt"]
        );
        assert_eq!(walked.per_scope, [3, 0, 0]);
    }

    /// The other way round: a later scope that contains an earlier one keeps
    /// the earlier one's folder as an entry but does not walk it again.
    #[test]
    fn a_scope_around_an_earlier_one_does_not_walk_it_twice() {
        let tree = Tree::new("walk-around");
        tree.file("Documents/notes.md").file("top.txt");

        let walked = walk(&rules(&[tree.path("Documents"), tree.0.clone()]));

        assert_eq!(
            relative(&walked, &tree.0),
            ["Documents/", "Documents/notes.md", "top.txt"]
        );
        assert_eq!(walked.per_scope, [1, 2]);
    }

    /// `~` skips `AppData`, so a scope inside it is walked on its own: the
    /// outer walk never reaches it, and dropping it indexed nothing.
    #[test]
    fn a_scope_under_a_skipped_folder_is_walked_on_its_own() {
        let tree = Tree::new("walk-nested-skipped");
        tree.file("top.txt")
            .file("AppData/Local/cache.db")
            .file("AppData/Roaming/Lumen/config/settings.json");

        let walked = walk(&rules(&[
            tree.0.clone(),
            tree.path("AppData/Roaming/Lumen/config"),
        ]));

        assert_eq!(
            relative(&walked, &tree.0),
            ["AppData/Roaming/Lumen/config/settings.json", "top.txt"]
        );
        assert_eq!(walked.per_scope, [1, 1]);
    }

    /// The same for every rule between them: an exclusion, a hidden folder,
    /// an ignore file and an ignore pattern. The scope's own folder is where
    /// the user looks from, so the rules apply below it, never to it.
    #[test]
    fn a_scope_beats_every_rule_that_hides_it_from_an_outer_one() {
        let tree = Tree::new("walk-nested-rules");
        tree.file("top.txt")
            .file("Videos/keep/clip.mp4")
            .file(".config/app/app.toml")
            .write("docs/.lumenignore", "drafts/\n")
            .file("docs/drafts/kept/one.md")
            .file("logs/today/run.txt");

        let mut rules = rules(&[
            tree.0.clone(),
            tree.path("Videos/keep"),
            tree.path(".config/app"),
            tree.path("docs/drafts/kept"),
            tree.path("logs/today"),
        ]);
        rules.exclusions = vec![tree.path("Videos")];
        rules.ignore_patterns = vec!["**/logs/**".into()];
        let walked = walk(&rules);

        assert_eq!(
            relative(&walked, &tree.0),
            [
                ".config/app/app.toml",
                "Videos/keep/clip.mp4",
                "docs/",
                "docs/drafts/kept/one.md",
                "logs/",
                "logs/today/run.txt",
                "top.txt",
            ]
        );
        assert_eq!(walked.per_scope, [3, 1, 1, 1, 1]);
    }

    /// A scope that is also an exclusion is both asked for and refused, by
    /// the same user; the exclusion is the narrower intent and wins.
    #[test]
    fn an_exclusion_of_the_scope_itself_wins() {
        let tree = Tree::new("walk-nested-excluded");
        tree.file("top.txt").file("Videos/clip.mp4");

        let mut rules = rules(&[tree.0.clone(), tree.path("Videos")]);
        rules.exclusions = vec![tree.path("Videos")];
        let walked = walk(&rules);

        assert_eq!(relative(&walked, &tree.0), ["top.txt"]);
        assert_eq!(walked.per_scope, [1, 0]);
    }

    /// Listed first or last, a scope under a skipped folder gives its
    /// entries once, and a scope inside it that its walk reaches adds
    /// nothing.
    #[test]
    fn a_scope_under_a_skipped_folder_is_never_walked_twice() {
        let tree = Tree::new("walk-nested-skipped-order");
        tree.file("top.txt")
            .file("AppData/Lumen/config/settings.json")
            .file("AppData/Lumen/config/themes/dark.json");
        let config = tree.path("AppData/Lumen/config");

        let walked = walk(&rules(&[
            config.clone(),
            tree.0.clone(),
            config.join("themes"),
        ]));

        assert_eq!(
            relative(&walked, &tree.0),
            [
                "AppData/Lumen/config/settings.json",
                "AppData/Lumen/config/themes/",
                "AppData/Lumen/config/themes/dark.json",
                "top.txt",
            ]
        );
        assert_eq!(walked.per_scope, [3, 1, 0]);
    }

    /// A nested scope walked on its own takes its turn where it is listed,
    /// not after the rest, and counts what it gave.
    #[test]
    fn a_nested_scope_takes_its_turn_in_the_listed_order() {
        let tree = Tree::new("walk-nested-order");
        let other = Tree::new("walk-nested-order-other");
        tree.file("a.txt").file("AppData/config/b.txt");
        other.file("c.txt").file("d.txt");

        let rules = rules(&[tree.0.clone(), tree.path("AppData/config"), other.0.clone()]);
        let walked = walk(&rules);

        assert_eq!(walked.per_scope, [1, 1, 2]);
        assert_eq!(walked.entries[1].path, tree.path("AppData/config/b.txt"));
    }

    /// With hidden files on, a dotted folder of the user's is found and a
    /// tool's cache is not: the skip list holds the caches by name, apart
    /// from the hidden rule, and each can be turned back on.
    #[test]
    fn tool_caches_stay_out_with_hidden_files_on() {
        let tree = Tree::new("walk-tool-caches");
        tree.file(".rustup/x")
            .file(".cargo/registry/y")
            .file(".npm/z")
            .file(".bun/install/cache/w")
            .file(".yarn/berry/v")
            .file(".deno/bin/u")
            .file(".myconfig/keep.txt");

        let mut rules = tree.rules();
        rules.include_hidden = true;
        assert_eq!(
            relative(&walk(&rules), &tree.0),
            [".myconfig/", ".myconfig/keep.txt"]
        );

        rules.skips_off = vec![".CARGO".into()];
        assert_eq!(
            relative(&walk(&rules), &tree.0),
            [
                ".cargo/",
                ".cargo/registry/",
                ".cargo/registry/y",
                ".myconfig/",
                ".myconfig/keep.txt",
            ]
        );
    }

    /// A default skip the user turned back on is descended into like any
    /// folder, matched without regard to case.
    #[test]
    fn a_default_skip_turned_off_is_indexed() {
        let tree = Tree::new("walk-skips-off");
        tree.file("app/node_modules/left-pad/index.js")
            .file("app/target/out.bin");

        let mut rules = tree.rules();
        rules.skips_off = vec!["node_modules".into()];
        assert_eq!(
            relative(&walk(&rules), &tree.0),
            [
                "app/",
                "app/node_modules/",
                "app/node_modules/left-pad/",
                "app/node_modules/left-pad/index.js",
            ]
        );
    }

    /// The index has no ceiling (`docs/FILE-SEARCH.md`, "Decision: B"):
    /// every scope is walked to its end, however many came before it, and
    /// each counts all it gave. Nothing in the rules can cut one short.
    #[test]
    fn every_scope_is_walked_to_its_end() {
        let first = Tree::new("walk-all-first");
        let second = Tree::new("walk-all-second");
        for index in 0..5 {
            first.file(&format!("a{index}.txt"));
            second.file(&format!("b{index}.txt"));
        }

        let rules = WalkRules {
            scopes: vec![first.0.clone(), second.0.clone()],
            exclusions: Vec::new(),
            ignore_patterns: Vec::new(),
            use_ignore_files: true,
            include_hidden: false,
            skips_off: Vec::new(),
        };
        let walked = walk(&rules);

        assert_eq!(walked.entries.len(), 10);
        assert_eq!(walked.per_scope, [5, 5]);
    }

    #[test]
    fn a_scope_that_does_not_exist_is_empty_not_an_error() {
        let walked = walk(&rules(&[PathBuf::from("no/such/folder/anywhere")]));
        assert!(walked.entries.is_empty());
        assert_eq!(walked.per_scope, [0]);
    }
}
