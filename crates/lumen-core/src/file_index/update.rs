//! Applying changes to the index in place (F16, slice 2).
//!
//! # A folder at a time
//!
//! Whatever the watcher reports -- a file created, a folder deleted with
//! everything in it, a rename -- is applied the same way: the folder it
//! happened in is read again, one directory read by the same rules as the
//! walk ([`super::walk::list`]), and compared with what the store has in it.
//! What left is removed with its subtree; what arrived is added, and a
//! folder that arrived is walked, since nothing reported what came with it.
//! The reconcile at start is the same work over the folders whose time
//! moved.
//!
//! # Only where the rules already looked
//!
//! A folder is read again only when it is a scope or an indexed folder:
//! one the rules allowed when it was last read. A change inside `AppData`,
//! a hidden folder, an exclusion or an ignored one is in no indexed folder,
//! so it is passed over without a read -- the same rules decide as for the
//! walk, without a second copy of them here. A scope the walk reached on
//! its own, inside `AppData` say, is a scope like any other: a change in it
//! belongs to it, the deepest walked scope around the folder. A changed ignore file reads
//! its folder's subfolders again too, since it may say something new about
//! any of them.
//!
//! # Shallowest first
//!
//! Folders are read parents first, in one transaction whose reads see its
//! own writes. A folder that arrived with files in it is walked when its
//! parent is read; reading it again afterwards finds nothing to change.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;

use super::store::{Batch, Child, Meta};
use super::walk::{self, Entry, WalkRules};

/// Ignore files: a change to one may change what is allowed anywhere below.
const IGNORE_FILES: &[&str] = &[".gitignore", ".ignore", ".lumenignore"];

/// What a batch did to the rows, for the names to follow.
#[derive(Debug, Default)]
pub(crate) struct Delta {
    /// The rows removed, ascending once [`Self::finish`] has run.
    pub removed: Vec<u32>,
    /// The rows added, in the order of their ids, with name and kind.
    pub added: Vec<(u32, String, bool)>,
    /// How many folders were read again.
    pub reread: usize,
    /// A scope's own folder was among them.
    pub retarget: bool,
}

impl Delta {
    pub fn changed(&self) -> bool {
        !self.removed.is_empty() || !self.added.is_empty()
    }

    fn finish(&mut self) {
        self.removed.sort_unstable();
        self.removed.dedup();
    }
}

/// The folders to read again for the changed `paths`, each with whether its
/// subfolders are read again too; `None` when a path is empty, the watcher's
/// way of saying changes were lost (see [`super::watch::FileWatch::start`]).
pub(crate) fn folders(paths: &[PathBuf]) -> Option<Vec<(PathBuf, bool)>> {
    let mut folders: HashMap<PathBuf, bool> = HashMap::new();
    for path in paths {
        if path.as_os_str().is_empty() {
            return None;
        }
        let Some(parent) = path.parent() else {
            continue;
        };
        let deep = path
            .file_name()
            .is_some_and(|name| IGNORE_FILES.iter().any(|file| name == *file));
        *folders.entry(parent.to_path_buf()).or_default() |= deep;
    }
    Some(folders.into_iter().collect())
}

/// The folders whose time is not the one recorded -- moved, vanished or
/// never recorded -- among the scopes' own folders and `stored`, every
/// indexed folder. How many were compared, and those.
pub(crate) fn moved(
    rules: &WalkRules,
    meta: &Meta,
    stored: Vec<(PathBuf, Option<i64>)>,
) -> (usize, Vec<(PathBuf, bool)>) {
    let mut checked = 0;
    let mut moved = Vec::new();
    for index in walk::effective_scopes(rules) {
        let scope = &rules.scopes[index];
        checked += 1;
        if walk::modified_time(scope) != meta.roots.get(index).copied().flatten() {
            moved.push((scope.clone(), false));
        }
    }
    for (folder, recorded) in stored {
        checked += 1;
        let now = walk::modified_time(&folder);
        // A folder that vanished is removed when its parent, whose time
        // moved with it, is read.
        if now.is_some() && now != recorded {
            moved.push((folder, false));
        }
    }
    (checked, moved)
}

/// Read `folders` again by `rules` and make the store match; `meta`'s
/// counts by scope follow.
pub(crate) fn apply(
    batch: &Batch<'_>,
    rules: &WalkRules,
    mut folders: Vec<(PathBuf, bool)>,
    meta: &mut Meta,
) -> Result<Delta> {
    let scopes = rules.scopes.len();
    meta.per_scope.resize(scopes, 0);
    meta.roots.resize(scopes, None);
    let effective = walk::effective_scopes(rules);

    folders.sort_by_key(|(folder, _)| folder.components().count());
    let mut delta = Delta::default();
    for (folder, deep) in folders {
        let Some(scope) = owner(rules, &effective, &folder) else {
            continue;
        };
        let at = Reread {
            batch,
            rules,
            scope,
            nested: nested(rules, &effective, scope),
        };
        at.folder(&folder, deep, meta, &mut delta)?;
    }
    delta.finish();
    Ok(delta)
}

/// The scope `folder` belongs to: the deepest walked scope it is in, as the
/// walk gave a scope inside another its own entries.
fn owner(rules: &WalkRules, effective: &[usize], folder: &Path) -> Option<usize> {
    effective
        .iter()
        .copied()
        .filter(|&index| folder.starts_with(&rules.scopes[index]))
        .max_by_key(|&index| rules.scopes[index].components().count())
}

/// The walked scopes inside scope `scope`: listed as its folders, walked
/// on their own.
fn nested(rules: &WalkRules, effective: &[usize], scope: usize) -> Vec<PathBuf> {
    let path = &rules.scopes[scope];
    effective
        .iter()
        .map(|&index| &rules.scopes[index])
        .filter(|other| other.starts_with(path) && *other != path)
        .cloned()
        .collect()
}

/// Reading folders of one scope again.
struct Reread<'a, 'b> {
    batch: &'a Batch<'b>,
    rules: &'a WalkRules,
    scope: usize,
    nested: Vec<PathBuf>,
}

impl Reread<'_, '_> {
    fn scope_path(&self) -> &Path {
        &self.rules.scopes[self.scope]
    }

    /// Read `folder` again (see "A folder at a time" above).
    fn folder(&self, folder: &Path, deep: bool, meta: &mut Meta, delta: &mut Delta) -> Result<()> {
        let is_scope = folder == self.scope_path();
        if !is_scope && !self.batch.is_folder(folder)? {
            return Ok(());
        }
        // Taken before the read, so a change made during it moves the time
        // again and is read at the next reconcile.
        let modified = walk::modified_time(folder);
        let listed = if folder.is_dir() {
            walk::list(folder, self.scope_path(), self.rules, &self.nested)
        } else if is_scope {
            // A scope that is gone is empty, as the walk has it.
            Vec::new()
        } else {
            // Gone: removed when its parent is read.
            return Ok(());
        };
        delta.reread += 1;
        delta.retarget |= is_scope;

        let on_disk = listed
            .iter()
            .map(|entry| (name(entry), entry.is_dir))
            .collect::<HashSet<_>>();
        // What the store has and the disk still has, of the same kind, is
        // kept -- a folder too, unless an ignore file changed beside it.
        let kept = |child: &Child| {
            !(deep && child.is_dir)
                && on_disk.contains(&(Cow::Borrowed(child.name.as_str()), child.is_dir))
        };
        let mut present = HashSet::new();
        for child in &self.batch.children(folder)? {
            if kept(child) {
                present.insert((child.name.clone(), child.is_dir));
                continue;
            }
            for (row, scope) in self.batch.remove(folder, child)? {
                delta.removed.push(row);
                if let Some(counted) = meta.per_scope.get_mut(scope) {
                    *counted = counted.saturating_sub(1);
                }
            }
        }
        for entry in &listed {
            if present.contains(&(name(entry).into_owned(), entry.is_dir)) {
                continue;
            }
            self.add(entry, meta, delta)?;
            if entry.is_dir {
                let below =
                    walk::walk_below(&entry.path, self.scope_path(), self.rules, &self.nested);
                for entry in &below {
                    self.add(entry, meta, delta)?;
                }
            }
        }

        if is_scope {
            meta.roots[self.scope] = modified;
        } else {
            self.batch.set_modified(folder, modified)?;
        }
        Ok(())
    }

    /// Add `entry`, counted to its scope.
    fn add(&self, entry: &Entry, meta: &mut Meta, delta: &mut Delta) -> Result<()> {
        let row = self.batch.insert(self.scope, entry)?;
        delta
            .added
            .push((row, name(entry).into_owned(), entry.is_dir));
        meta.per_scope[self.scope] += 1;
        Ok(())
    }
}

fn name(entry: &Entry) -> Cow<'_, str> {
    entry
        .path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default()
}
