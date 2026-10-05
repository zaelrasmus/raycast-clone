//! Favourites: the items a person put first, in the order they put them (F15).
//!
//! An ordered list of item ids and nothing else. The order is the user's, so
//! nothing here ranks: a favourite is where it was put until it is moved.
//!
//! An id whose item is gone -- an application uninstalled, a quicklink
//! deleted -- is kept, not dropped. The list is drawn from the items that
//! exist, so it simply does not show, and an application reinstalled comes
//! back where it was. Pruning would make an update that briefly removes a
//! shortcut cost the user their arrangement.

use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

/// What a person asked to do to a favourite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Add,
    Remove,
    MoveUp,
    MoveDown,
}

impl Change {
    pub fn verb(self) -> &'static str {
        match self {
            Change::Add => "Add to Favorites",
            Change::Remove => "Remove from Favorites",
            Change::MoveUp => "Move Up in Favorites",
            Change::MoveDown => "Move Down in Favorites",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Favorites {
    #[serde(default)]
    ids: Vec<String>,
}

impl Favorites {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load from disk, falling back to none. Like frecency, an unreadable
    /// file is reported and survived, not fatal.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(source) => serde_json::from_str(&source).unwrap_or_else(|error| {
                log::warn!(
                    "ignoring unreadable favourites at {}: {error}",
                    path.display()
                );
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let source = serde_json::to_string_pretty(self)?;
        // Beside and renamed over, as frecency is written.
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, source)
            .with_context(|| format!("writing {}", temporary.display()))?;
        std::fs::rename(&temporary, path).with_context(|| format!("replacing {}", path.display()))
    }

    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    pub fn contains(&self, id: &str) -> bool {
        self.ids.iter().any(|each| each == id)
    }

    /// Replace `from` with `to` where it stands, for an item whose id
    /// changed. If `to` is already a favourite it keeps its own place and
    /// `from` is dropped. Whether anything changed.
    pub fn rename(&mut self, from: &str, to: &str) -> bool {
        if from == to {
            return false;
        }
        let Some(at) = self.ids.iter().position(|each| each == from) else {
            return false;
        };
        if self.contains(to) {
            self.ids.retain(|each| each != from);
        } else {
            self.ids[at] = to.to_string();
        }
        true
    }

    /// What can be done to `id` from where it is now, in the order the
    /// action panel offers them. `visible` is the ids of the favourites that
    /// are drawn, in order: moving steps over the ones that are not, or a
    /// move would seem to do nothing.
    pub fn changes(&self, id: &str, visible: &[&str]) -> Vec<Change> {
        let Some(at) = visible.iter().position(|each| *each == id) else {
            return vec![Change::Add];
        };
        let mut changes = vec![Change::Remove];
        if at > 0 {
            changes.push(Change::MoveUp);
        }
        if at + 1 < visible.len() {
            changes.push(Change::MoveDown);
        }
        changes
    }

    /// Apply `change` to `id`. Whether anything changed, so a move at the end
    /// of the list writes nothing.
    ///
    /// A move swaps with the nearest *visible* neighbour, for the reason
    /// [`Favorites::changes`] gives.
    pub fn apply(&mut self, change: Change, id: &str, visible: &[&str]) -> bool {
        match change {
            Change::Add if !self.contains(id) => {
                self.ids.push(id.to_string());
                true
            }
            Change::Add => false,
            Change::Remove => {
                let before = self.ids.len();
                self.ids.retain(|each| each != id);
                self.ids.len() != before
            }
            Change::MoveUp | Change::MoveDown => {
                let Some(at) = visible.iter().position(|each| *each == id) else {
                    return false;
                };
                let neighbour = match change {
                    Change::MoveUp => at.checked_sub(1),
                    _ => Some(at + 1).filter(|next| *next < visible.len()),
                };
                let Some(neighbour) = neighbour.map(|index| visible[index]) else {
                    return false;
                };
                let position = |wanted: &str| self.ids.iter().position(|each| each == wanted);
                match (position(id), position(neighbour)) {
                    (Some(a), Some(b)) => {
                        self.ids.swap(a, b);
                        true
                    }
                    _ => false,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn favorites(ids: &[&str]) -> Favorites {
        Favorites {
            ids: ids.iter().map(|id| id.to_string()).collect(),
        }
    }

    #[test]
    fn adds_at_the_end_once() {
        let mut list = favorites(&["a"]);
        assert!(list.apply(Change::Add, "b", &["a"]));
        assert!(!list.apply(Change::Add, "b", &["a", "b"]));
        assert_eq!(list.ids(), ["a", "b"]);
    }

    #[test]
    fn removes() {
        let mut list = favorites(&["a", "b"]);
        assert!(list.apply(Change::Remove, "a", &["a", "b"]));
        assert_eq!(list.ids(), ["b"]);
        assert!(!list.apply(Change::Remove, "a", &["b"]));
    }

    #[test]
    fn offers_what_can_be_done_from_where_it_is() {
        let list = favorites(&["a", "b", "c"]);
        let visible = ["a", "b", "c"];
        assert_eq!(list.changes("x", &visible), [Change::Add]);
        assert_eq!(
            list.changes("a", &visible),
            [Change::Remove, Change::MoveDown]
        );
        assert_eq!(
            list.changes("b", &visible),
            [Change::Remove, Change::MoveUp, Change::MoveDown]
        );
        assert_eq!(
            list.changes("c", &visible),
            [Change::Remove, Change::MoveUp]
        );
    }

    /// `gone` is uninstalled: it keeps its place in the file, and moving `c`
    /// up passes it to land above `a`'s neighbour, not beside the ghost.
    #[test]
    fn moves_step_over_what_is_not_shown() {
        let mut list = favorites(&["a", "gone", "c"]);
        let visible = ["a", "c"];
        assert!(list.apply(Change::MoveUp, "c", &visible));
        assert_eq!(list.ids(), ["c", "gone", "a"]);
        assert!(!list.apply(Change::MoveUp, "c", &["c", "a"]));
    }

    #[test]
    fn rename_keeps_the_position() {
        let mut list = favorites(&["a", "old", "c"]);
        assert!(list.rename("old", "new"));
        assert_eq!(list.ids(), ["a", "new", "c"]);
        assert!(!list.rename("old", "new"));
    }

    /// The target already has its own place; the old entry is the duplicate.
    #[test]
    fn rename_onto_an_existing_id_drops_the_old_one() {
        let mut list = favorites(&["new", "a", "old"]);
        assert!(list.rename("old", "new"));
        assert_eq!(list.ids(), ["new", "a"]);
    }

    #[test]
    fn round_trips_through_json() {
        let list = favorites(&["app:code", "command:clipboard"]);
        let json = serde_json::to_string(&list).unwrap();
        assert_eq!(serde_json::from_str::<Favorites>(&json).unwrap(), list);
    }
}
