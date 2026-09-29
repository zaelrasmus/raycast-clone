//! Currency in the root list (F9).
//!
//! The *Currency* row opens the view, and `10 usd in cad` is answered from
//! the table in memory. Nothing here fetches: the daemon does, off the UI
//! thread, and the answer appears when the table lands (I-3).

use crate::currency::query::Conversion;
use crate::currency::{Rates, format};
use crate::item::{Action, Item, Kind, ViewSource};
use crate::providers::Provider;

/// The root list's answer to a conversion, against `rates`: `14.16 CAD`,
/// and both sides beneath. `None` when the table has no rate for either.
pub fn answer(conversion: &Conversion, rates: &Rates) -> Option<Item> {
    let Conversion { amount, from, to } = *conversion;
    let value = rates.convert(amount, from.code, to.code)?;
    let result = format::with_code(value, to);
    Some(Item {
        id: "currency:answer".into(),
        title: result.clone(),
        subtitle: Some(format!(
            "{} = {result} \u{00b7} rates of {}",
            format::with_code(amount, from),
            rates.date()
        )),
        icon: None,
        keywords: Vec::new(),
        // What is pasted into a form: the number alone, as the view copies.
        actions: vec![
            Action::CopyToClipboard(format::plain(value, to)),
            Action::OpenView(ViewSource::Currency),
        ],
        kind: Kind::Currency,
    })
}

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
            kind: Kind::Currency,
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

    #[test]
    fn a_conversion_says_both_sides_and_copies_the_number() {
        let rates = crate::currency::tests::rates(&[("USD", 1.0), ("CAD", 1.41649)]);
        let conversion = crate::currency::query::recognise("10 usd in cad").unwrap();
        let item = answer(&conversion, &rates).unwrap();

        assert_eq!(item.title, "14.16 CAD");
        assert_eq!(
            item.subtitle.as_deref(),
            Some("10.00 USD = 14.16 CAD \u{00b7} rates of 28 Sep 2026")
        );
        assert_eq!(item.primary(), Action::CopyToClipboard("14.16".into()));
    }
}
