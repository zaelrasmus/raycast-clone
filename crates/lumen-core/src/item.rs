//! The one result type the UI knows how to draw.
//!
//! Providers (F3) and, later, extensions (F4) all produce this. Keeping the
//! result shape closed rather than letting each source define its own is the
//! same reasoning as AD-07: consistency is obtained by construction, not by
//! asking every contributor to be disciplined.

use lumen_platform::{AppEntry, IconSource, LaunchTarget};

/// A single row in the result list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Stable across reindexes: this is the frecency key.
    pub id: String,
    pub title: String,
    /// The secondary line. Reserved space is allocated whether or not it is
    /// present, so that a row with a subtitle and one without are the same
    /// height (I-10).
    pub subtitle: Option<String>,
    pub icon: Option<IconSource>,
    /// Matched against, never displayed.
    pub keywords: Vec<String>,
    pub action: Action,
}

/// What activating a row does.
///
/// An enum rather than a boxed closure: actions have to be comparable, cheap to
/// clone, and eventually serialisable across the extension boundary (AD-06).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Launch(LaunchTarget),
    CopyToClipboard(String),
}

impl Action {
    /// The verb shown for this action in the footer and the action panel.
    pub fn verb(&self) -> &'static str {
        match self {
            Action::Launch(_) => "Open",
            Action::CopyToClipboard(_) => "Copy",
        }
    }
}

impl Item {
    pub fn from_app(entry: AppEntry) -> Self {
        Self {
            id: entry.id,
            title: entry.name,
            subtitle: entry.detail,
            icon: entry.icon,
            keywords: entry.keywords,
            action: Action::Launch(entry.target),
        }
    }

    /// The right-hand label describing what this row is.
    ///
    /// Fixed per action kind rather than per item, so the column never changes
    /// width between keystrokes (I-10).
    pub fn kind_label(&self) -> &'static str {
        match self.action {
            Action::Launch(_) => "Application",
            Action::CopyToClipboard(_) => "Text",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_entry_becomes_a_launchable_item() {
        let entry = AppEntry {
            id: "app:Firefox.lnk".into(),
            name: "Firefox".into(),
            detail: None,
            target: LaunchTarget::ShellExecute("shell:AppsFolder\\Firefox.lnk".into()),
            icon: None,
            keywords: vec!["browser".into()],
        };

        let item = Item::from_app(entry);

        assert_eq!(item.title, "Firefox");
        assert_eq!(item.kind_label(), "Application");
        assert_eq!(item.action.verb(), "Open");
        // The id must survive the conversion, or frecency resets on reindex.
        assert_eq!(item.id, "app:Firefox.lnk");
    }
}
