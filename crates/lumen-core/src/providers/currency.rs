//! Currency in the root list (F9).
//!
//! The *Currency* row opens the view. Nothing here fetches: the view's
//! driver in the daemon does, off the UI thread (I-3).

use crate::item::{Action, Item, Kind, ViewSource};
use crate::providers::Provider;

pub struct CurrencyProvider;

impl Provider for CurrencyProvider {
    fn id(&self) -> &'static str {
        "currency"
    }

    fn catalogue(&self) -> Vec<Item> {
        vec![Item {
            id: "currency:open".into(),
            title: "Currency".into(),
            subtitle: Some("Convert an amount into every currency".into()),
            icon: None,
            keywords: vec![
                "currency".into(),
                "exchange".into(),
                "convert".into(),
                "money".into(),
                "rates".into(),
            ],
            actions: vec![Action::OpenView(ViewSource::Currency)],
            kind: Kind::Command,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_opens_the_view() {
        let items = CurrencyProvider.catalogue();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].primary(), Action::OpenView(ViewSource::Currency));
    }
}
