//! The *Clipboard History* command in the root list (F12).
//!
//! Entries were matched in the root list as an aside from F3 to F11 (7.14).
//! F12 moves them into a view of their own, the way F11 moved snippets: a
//! paste is something you look for among other pastes, with its type and its
//! source beside it, and the root list answers "open something". What stays
//! here is the one row that opens the view. The entries are in
//! [`crate::clipboard`], and the view is [`crate::clipboard::session`].

use crate::item::{Action, Item, Kind, ViewSource};
use crate::providers::Provider;

pub struct ClipboardProvider;

impl Provider for ClipboardProvider {
    fn id(&self) -> &'static str {
        "clipboard"
    }

    fn catalogue(&self) -> Vec<Item> {
        vec![Item {
            id: "clipboard:history".into(),
            title: "Clipboard History".into(),
            subtitle: Some("Find something you copied and paste it again".into()),
            icon: None,
            keywords: vec![
                "clipboard".into(),
                "paste".into(),
                "copy".into(),
                "history".into(),
            ],
            actions: vec![Action::OpenView(ViewSource::Clipboard)],
            kind: Kind::Clipboard,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_opens_the_view() {
        let items = ClipboardProvider.catalogue();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].primary(), Action::OpenView(ViewSource::Clipboard));
    }
}
