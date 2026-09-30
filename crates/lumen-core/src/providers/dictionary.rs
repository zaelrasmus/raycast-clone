//! The *Dictionary* command in the root list (F10).
//!
//! A catalogue row, not an answer: choosing it opens the view, and root
//! search never looks a word up (I-3). `define hola` as an inline prefix is
//! deferred for F7's reason -- it is a request in the one list that must
//! never wait.

use crate::item::{Action, Item, Kind, ViewSource};
use crate::providers::Provider;

pub struct DictionaryProvider;

impl Provider for DictionaryProvider {
    fn id(&self) -> &'static str {
        "dictionary"
    }

    fn catalogue(&self) -> Vec<Item> {
        vec![Item {
            id: "dictionary:open".into(),
            title: "Dictionary".into(),
            subtitle: Some("Look up what a word means, in any language".into()),
            icon: None,
            keywords: vec![
                "define".into(),
                "dictionary".into(),
                "definition".into(),
                "meaning".into(),
                "word".into(),
            ],
            actions: vec![Action::OpenView(ViewSource::Dictionary)],
            kind: Kind::Command,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_opens_the_view_and_is_found_by_define() {
        let items = DictionaryProvider.catalogue();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].primary(), Action::OpenView(ViewSource::Dictionary));
        assert!(items[0].keywords.iter().any(|keyword| keyword == "define"));
    }
}
