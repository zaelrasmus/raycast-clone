//! The *Search Emoji & Symbols* command in the root list (F18).
//!
//! A catalogue row, not an answer: choosing it opens the view, and root
//! search never reads the emoji catalogue (I-3). The entries are in
//! [`crate::emoji`], and the view is [`crate::emoji::session`].

use crate::item::{Action, Item, Kind, ViewSource};
use crate::providers::Provider;

pub struct EmojiProvider;

impl Provider for EmojiProvider {
    fn id(&self) -> &'static str {
        "emoji"
    }

    fn catalogue(&self) -> Vec<Item> {
        vec![Item {
            id: "emoji:search".into(),
            title: "Search Emoji & Symbols".into(),
            subtitle: Some("Find an emoji or a symbol and paste it".into()),
            icon: None,
            keywords: vec![
                "emoji".into(),
                "symbols".into(),
                "characters".into(),
                "unicode".into(),
            ],
            actions: vec![Action::OpenView(ViewSource::Emoji)],
            kind: Kind::Command,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::Providers;

    #[test]
    fn the_command_opens_the_view_and_is_found_by_emoji() {
        let items = EmojiProvider.catalogue();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].primary(), Action::OpenView(ViewSource::Emoji));
        assert!(items[0].keywords.iter().any(|keyword| keyword == "emoji"));
    }

    #[test]
    fn the_command_is_built_in() {
        assert!(
            Providers::builtin()
                .catalogue()
                .iter()
                .any(|item| item.primary() == Action::OpenView(ViewSource::Emoji))
        );
    }
}
