//! Window management as commands (F3).
//!
//! A genuine catalogue provider, unlike the clipboard: seventeen fixed entries
//! that exist whether or not anyone asked, found by typing part of their name.
//!
//! # Why the names are spelled twice
//!
//! Every layout carries the American spelling of its own title as a keyword.
//! The interface says "Maximise" because the rest of this project's prose does,
//! and someone who types `maximize` -- which is most people on Windows -- must
//! still find it. That is a two-word change here and an unanswerable support
//! question if it is left out.

use lumen_platform::WindowLayout;

use crate::item::{Action, Item, Kind};
use crate::providers::Provider;

/// Every layout, its title, and what else finds it.
///
/// A table rather than a `match` with seventeen arms so that adding a layout is
/// one line and cannot half-happen: a variant with no row here fails the
/// exhaustiveness test below.
const LAYOUTS: &[(WindowLayout, &str, &[&str])] = &[
    (
        WindowLayout::Maximize,
        "Maximise Window",
        &["maximize", "full", "fill", "fullscreen"],
    ),
    (
        WindowLayout::Center,
        "Centre Window",
        &["center", "middle", "focus"],
    ),
    (WindowLayout::LeftHalf, "Left Half", &["snap", "tile"]),
    (WindowLayout::RightHalf, "Right Half", &["snap", "tile"]),
    (
        WindowLayout::TopHalf,
        "Top Half",
        &["snap", "tile", "upper"],
    ),
    (
        WindowLayout::BottomHalf,
        "Bottom Half",
        &["snap", "tile", "lower"],
    ),
    (
        WindowLayout::TopLeftQuarter,
        "Top Left Quarter",
        &["snap", "corner"],
    ),
    (
        WindowLayout::TopRightQuarter,
        "Top Right Quarter",
        &["snap", "corner"],
    ),
    (
        WindowLayout::BottomLeftQuarter,
        "Bottom Left Quarter",
        &["snap", "corner"],
    ),
    (
        WindowLayout::BottomRightQuarter,
        "Bottom Right Quarter",
        &["snap", "corner"],
    ),
    (WindowLayout::LeftThird, "Left Third", &["snap", "column"]),
    (
        WindowLayout::MiddleThird,
        "Middle Third",
        &["snap", "column"],
    ),
    (WindowLayout::RightThird, "Right Third", &["snap", "column"]),
    (
        WindowLayout::LeftTwoThirds,
        "Left Two Thirds",
        &["snap", "column"],
    ),
    (
        WindowLayout::RightTwoThirds,
        "Right Two Thirds",
        &["snap", "column"],
    ),
    (
        WindowLayout::NextMonitor,
        "Move to Next Monitor",
        &["screen", "display", "throw"],
    ),
    (
        WindowLayout::PreviousMonitor,
        "Move to Previous Monitor",
        &["screen", "display", "throw"],
    ),
];

/// What the subtitle says on every one of these rows.
///
/// The same line on all of them on purpose. It is not describing the layout --
/// the title does that, and "Left Half" needs no gloss -- it is answering the
/// question the user actually has the first time one of these appears, which
/// is *which* window this is about.
const SUBTITLE: &str = "The window you were just using";

pub struct ArrangeProvider;

impl Provider for ArrangeProvider {
    fn id(&self) -> &'static str {
        "window"
    }

    fn catalogue(&self) -> Vec<Item> {
        LAYOUTS.iter().map(item).collect()
    }
}

fn item(&(layout, title, keywords): &(WindowLayout, &str, &[&str])) -> Item {
    Item {
        id: format!("window:{title}"),
        title: title.to_string(),
        subtitle: Some(SUBTITLE.to_string()),
        icon: None,
        keywords: std::iter::once("window".to_string())
            .chain(keywords.iter().map(|keyword| keyword.to_string()))
            .collect(),
        actions: vec![Action::ArrangeWindow(layout)],
        kind: Kind::Window,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A layout with no row here would exist in the platform and be
    /// unreachable from the launcher, which is the kind of gap nobody notices
    /// until they go looking for the feature.
    #[test]
    fn every_layout_is_offered() {
        for layout in WindowLayout::ALL {
            assert!(
                LAYOUTS.iter().any(|&(candidate, ..)| candidate == *layout),
                "{layout:?} has no row in the launcher"
            );
        }
        assert_eq!(LAYOUTS.len(), WindowLayout::ALL.len());
    }

    #[test]
    fn titles_are_unique_so_the_ids_cannot_collide() {
        let mut ids: Vec<String> = ArrangeProvider
            .catalogue()
            .into_iter()
            .map(|item| item.id)
            .collect();
        let count = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }

    /// Typing the American spelling has to find the British title.
    #[test]
    fn the_other_spelling_is_a_keyword() {
        let items = ArrangeProvider.catalogue();
        let maximise = items
            .iter()
            .find(|item| item.title == "Maximise Window")
            .unwrap();

        assert!(maximise.keywords.iter().any(|k| k == "maximize"));
    }

    #[test]
    fn every_row_arranges_something_and_says_so() {
        for item in ArrangeProvider.catalogue() {
            assert_eq!(item.kind, Kind::Window);
            assert!(matches!(item.primary(), Action::ArrangeWindow(_)));
            assert_eq!(item.actions.len(), 1);
        }
    }
}
