//! Clipboard history as searchable results (F3).
//!
//! # Why this is not a [`Provider`](super::Provider)
//!
//! It looks like one, and it very nearly was. A provider's catalogue joins the
//! application index and is matched with it, which is right for Lumen's own
//! commands, for scripts and for snippets: they are a fixed set that changes
//! when the user edits a file.
//!
//! The clipboard is neither fixed nor small. It is rewritten every time the
//! user copies anything, so folding it into the application index would mean
//! re-preparing several hundred application haystacks on every copy. And it
//! must not appear in the root list: with an empty query the index is shown in
//! frecency order, and a launcher that opens on a hundred old pastes is not a
//! launcher.
//!
//! Both of those are ranking rules, not result-building rules, so the view
//! holds the history as its own index and this module only says what a row
//! looks like.

use crate::clipboard::Entry;
use crate::item::{Action, Item, Kind};

/// How much of an entry is shown in a row.
const PREVIEW_LIMIT: usize = 80;

/// Turn a page of history into rows.
pub fn items(entries: Vec<Entry>) -> Vec<Item> {
    entries.into_iter().map(into_item).collect()
}

fn into_item(entry: Entry) -> Item {
    Item {
        // Keyed on the database id, which is stable for as long as the entry
        // exists and is never reused after it is pruned.
        id: format!("clipboard:{}", entry.id),
        title: entry.preview(PREVIEW_LIMIT),
        // Where it came from, which is usually how people remember it: "the
        // thing I copied out of the terminal".
        subtitle: entry.source.clone(),
        icon: None,
        // The full text is matched even though only a preview is shown, so a
        // long entry can be found by something in its middle.
        keywords: vec![entry.text.clone()],
        actions: vec![Action::CopyToClipboard(entry.text)],
        kind: Kind::Clipboard,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: i64, text: &str) -> Entry {
        Entry {
            id,
            text: text.into(),
            copied_at: 0,
            source: Some("code.exe".into()),
        }
    }

    #[test]
    fn an_entry_becomes_a_copyable_row() {
        let items = items(vec![entry(1, "hello")]);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "hello");
        assert_eq!(items[0].subtitle.as_deref(), Some("code.exe"));
        assert_eq!(items[0].kind, Kind::Clipboard);
        assert_eq!(items[0].primary(), Action::CopyToClipboard("hello".into()));
    }

    /// Only a preview is shown, but the whole entry is searchable -- otherwise
    /// a long paste can only be found by its first line.
    #[test]
    fn the_whole_text_is_matched_even_though_only_a_preview_is_shown() {
        let long = format!("{}NEEDLE", "x".repeat(200));
        let items = items(vec![entry(1, &long)]);

        assert!(items[0].title.chars().count() <= PREVIEW_LIMIT + 1);
        assert!(items[0].keywords[0].contains("NEEDLE"));
    }

    #[test]
    fn ids_are_namespaced_so_they_cannot_collide_with_an_application() {
        let items = items(vec![entry(7, "x")]);
        assert_eq!(items[0].id, "clipboard:7");
    }
}
