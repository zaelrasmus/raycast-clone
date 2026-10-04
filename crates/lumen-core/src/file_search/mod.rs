//! *Search Files* (F16, `docs/FILE-SEARCH.md`, "The *Search Files* view").
//!
//! The view over the whole file index: no eight-row cap, any query length,
//! a *Type* filter, *Recently Used* before anything is typed, and a pane
//! with a preview and the metadata. The decisions are in [`session`], where
//! they are tested; the host holds the index and the platform, and does
//! what the session asks off the UI thread.

pub mod browse;
pub mod category;
pub mod drives;
pub mod exclude;
pub mod icon;
pub mod preview;
pub mod recents;
pub mod session;
pub mod settings;

use std::path::PathBuf;

pub use category::{CATEGORIES, Category};

use crate::file_index::{FileHit, FileIndex, Kinds};
use crate::item::Kind;

/// The most rows a query shows: far more than anyone scrolls, and few
/// enough that the list is built in a frame.
pub const LIMIT: usize = 200;

/// The user's home folder, which *Where* is shortened against.
pub fn home_dir() -> Option<PathBuf> {
    directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
}

/// A file or folder the view shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Found {
    pub path: PathBuf,
    pub is_dir: bool,
}

impl Found {
    /// Its id in the view, and in frecency: the index's, so opening it here
    /// counts in root as well.
    pub fn id(&self) -> String {
        let prefix = if self.is_dir { "folder" } else { "file" };
        format!("{prefix}:{}", self.path.display())
    }

    /// The entry an id names, if it names one.
    pub fn from_id(id: &str) -> Option<Found> {
        if let Some(path) = id.strip_prefix("file:") {
            Some(Found {
                path: path.into(),
                is_dir: false,
            })
        } else {
            id.strip_prefix("folder:").map(|path| Found {
                path: path.into(),
                is_dir: true,
            })
        }
    }

    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }

    fn from_hit(hit: FileHit) -> Option<Found> {
        let found = Found::from_id(&hit.item.id)?;
        (found.is_dir == (hit.item.kind == Kind::Folder)).then_some(found)
    }
}

/// The first phase of a query: the best [`LIMIT`] entries for `query`, of
/// `filter`'s category when there is one -- every match in the hot layer,
/// and the disk's by a word's start ([`FileIndex::query_where`]). The
/// filter is applied inside the scan, not to its answer, so a filtered
/// query still finds its best `LIMIT` rather than what was left of
/// everyone's.
///
/// Milliseconds, and shown as soon as it answers. When
/// [`searches_inside`] says so, [`search_inside`] follows for the rest.
/// Off the UI thread, and stops once `cancelled` says the query was
/// superseded.
pub fn search(
    index: &FileIndex,
    query: &str,
    filter: Option<Category>,
    cancelled: &dyn Fn() -> bool,
) -> Vec<Found> {
    let kinds = kinds(filter);
    let accept = |name: &str, is_dir: bool| filter.is_none_or(|filter| filter.holds(name, is_dir));
    index
        .query_where(query, kinds, LIMIT, &accept, cancelled)
        .into_iter()
        .filter_map(Found::from_hit)
        .collect()
}

/// Whether `query` has a second phase on `index`: an index larger than its
/// hot layer, and a word long enough to match inside another
/// ([`FileIndex::searches_inside`]).
pub fn searches_inside(index: &FileIndex, query: &str) -> bool {
    index.searches_inside(query)
}

/// The second phase of a query: at most `limit` matches inside a word that
/// [`search`] could not find, of `filter`'s category, shortest first
/// ([`FileIndex::query_inside`]). Shown below the first phase's, never
/// among them. A scan of the disk -- up to a few hundred milliseconds on a
/// large index -- so it stops as soon as `cancelled` says so.
pub fn search_inside(
    index: &FileIndex,
    query: &str,
    filter: Option<Category>,
    limit: usize,
    cancelled: &dyn Fn() -> bool,
) -> Vec<Found> {
    let accept = |name: &str, is_dir: bool| filter.is_none_or(|filter| filter.holds(name, is_dir));
    index
        .query_inside(query, kinds(filter), limit, &accept, cancelled)
        .into_iter()
        .filter_map(Found::from_hit)
        .collect()
}

/// A typed path's answer (step 6d): the best [`LIMIT`] entries of the
/// folder it names, of `filter`'s category, read from the disk rather than
/// the index ([`browse::list_where`]). Off the UI thread; nothing once
/// `cancelled` says the query was superseded.
pub fn list_folder(
    typed: &browse::Typed,
    include_hidden: bool,
    filter: Option<Category>,
    cancelled: &dyn Fn() -> bool,
) -> Vec<Found> {
    let accept = |name: &str, is_dir: bool| filter.is_none_or(|filter| filter.holds(name, is_dir));
    browse::list_where(typed, include_hidden, LIMIT, &accept, cancelled)
}

/// The kinds `filter` asks of the index: folders alone for *Folders*.
fn kinds(filter: Option<Category>) -> Kinds {
    match filter {
        Some(Category::Folders) => Kinds::Folders,
        _ => Kinds::FoldersAndFiles,
    }
}
