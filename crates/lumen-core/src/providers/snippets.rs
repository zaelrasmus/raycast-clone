//! The *Search Snippets* and *Create Snippet* commands in the root list (F11).
//!
//! Snippets themselves are no longer in the root list: a hundred of them
//! matched beside the applications is 7.14's problem again, and a snippet is
//! something you search for in its own view. What stays here is the one row
//! that opens it. The snippets are in [`crate::snippets`].

use crate::item::{Action, Item, Kind, ViewSource};
use crate::providers::Provider;

pub struct SnippetsProvider;

impl Provider for SnippetsProvider {
    fn id(&self) -> &'static str {
        "snippets"
    }

    fn catalogue(&self) -> Vec<Item> {
        vec![
            Item {
                id: "snippets:search".into(),
                title: "Search Snippets".into(),
                subtitle: Some("Find a saved block of text and paste it".into()),
                icon: None,
                keywords: vec!["snippet".into(), "text".into(), "paste".into()],
                actions: vec![Action::OpenView(ViewSource::Snippets)],
                kind: Kind::Command,
            },
            Item {
                id: "snippets:create".into(),
                title: "Create Snippet".into(),
                subtitle: Some("Save a block of text to paste later".into()),
                icon: None,
                keywords: vec!["snippet".into(), "new".into(), "add".into()],
                actions: vec![Action::CreateSnippet],
                kind: Kind::Command,
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_commands_open_the_view_and_the_form() {
        let items = SnippetsProvider.catalogue();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].primary(), Action::OpenView(ViewSource::Snippets));
        assert_eq!(items[1].primary(), Action::CreateSnippet);
    }
}
