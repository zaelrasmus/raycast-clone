//! *Search Files* (F16, `docs/FILE-SEARCH.md`, "The *Search Files* view").
//!
//! The view over the whole file index: no eight-row cap, any query length,
//! a *Type* filter, *Recently Used* before anything is typed, and a pane
//! with a preview and the metadata. The decisions are in [`session`], where
//! they are tested; the host holds the index and the platform, and does
//! what the session asks off the UI thread.

pub mod category;
pub mod exclude;
pub mod preview;
pub mod recents;
pub mod session;

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

/// The best [`LIMIT`] entries of the whole index for `query`, of
/// `filter`'s category when there is one. The filter is applied inside the
/// scan, not to its answer, so a filtered query still finds its best
/// `LIMIT` rather than what was left of everyone's.
///
/// A scan of every name, so it runs off the UI thread, and stops once
/// `cancelled` says the query was superseded.
pub fn search(
    index: &FileIndex,
    query: &str,
    filter: Option<Category>,
    cancelled: &dyn Fn() -> bool,
) -> Vec<Found> {
    let kinds = match filter {
        Some(Category::Folders) => Kinds::Folders,
        _ => Kinds::FoldersAndFiles,
    };
    let accept = |name: &str, is_dir: bool| filter.is_none_or(|filter| filter.holds(name, is_dir));
    index
        .query_where(query, kinds, LIMIT, &accept, cancelled)
        .into_iter()
        .filter_map(Found::from_hit)
        .collect()
}
