//! The *World Clock* command in the root list (F13).
//!
//! A catalogue row that opens the view. The places are read when the view
//! opens, off the UI thread, never by this row.

use crate::item::{Action, Item, Kind, ViewSource};
use crate::providers::Provider;

pub struct ClockProvider;

impl Provider for ClockProvider {
    fn id(&self) -> &'static str {
        "clock"
    }

    fn catalogue(&self) -> Vec<Item> {
        vec![Item {
            id: "clock:open".into(),
            title: "World Clock".into(),
            subtitle: Some("The time anywhere, compared with yours".into()),
            icon: None,
            keywords: vec![
                "time".into(),
                "timezone".into(),
                "time zone".into(),
                "clock".into(),
                "utc".into(),
                "city".into(),
            ],
            actions: vec![Action::OpenView(ViewSource::Clock)],
            kind: Kind::Command,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_opens_the_view_and_is_found_by_timezone() {
        let items = ClockProvider.catalogue();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].primary(), Action::OpenView(ViewSource::Clock));
        assert!(
            items[0]
                .keywords
                .iter()
                .any(|keyword| keyword == "timezone")
        );
    }
}
