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

use crate::favorites::Favorites;
use crate::frecency::Frecency;
use crate::item::{Action, Item, Kind};
use crate::providers::Provider;

/// Every layout, its slug, its title, and what else finds it.
///
/// A table rather than a `match` with seventeen arms so that adding a layout is
/// one line and cannot half-happen: a variant with no row here fails the
/// exhaustiveness test below.
///
/// The slug is the item id, written out rather than derived from the title:
/// ids are persisted in frecency, favourites and hotkeys, and a title is
/// wording that may be changed. Once shipped, a slug never changes.
const LAYOUTS: &[(WindowLayout, &str, &str, &[&str])] = &[
    (
        WindowLayout::Maximize,
        "maximize",
        "Maximise Window",
        &["maximize", "full", "fill", "fullscreen"],
    ),
    (
        WindowLayout::Center,
        "center",
        "Centre Window",
        &["center", "middle", "focus"],
    ),
    (
        WindowLayout::LeftHalf,
        "left-half",
        "Left Half",
        &["snap", "tile"],
    ),
    (
        WindowLayout::RightHalf,
        "right-half",
        "Right Half",
        &["snap", "tile"],
    ),
    (
        WindowLayout::TopHalf,
        "top-half",
        "Top Half",
        &["snap", "tile", "upper"],
    ),
    (
        WindowLayout::BottomHalf,
        "bottom-half",
        "Bottom Half",
        &["snap", "tile", "lower"],
    ),
    (
        WindowLayout::TopLeftQuarter,
        "top-left-quarter",
        "Top Left Quarter",
        &["snap", "corner"],
    ),
    (
        WindowLayout::TopRightQuarter,
        "top-right-quarter",
        "Top Right Quarter",
        &["snap", "corner"],
    ),
    (
        WindowLayout::BottomLeftQuarter,
        "bottom-left-quarter",
        "Bottom Left Quarter",
        &["snap", "corner"],
    ),
    (
        WindowLayout::BottomRightQuarter,
        "bottom-right-quarter",
        "Bottom Right Quarter",
        &["snap", "corner"],
    ),
    (
        WindowLayout::LeftThird,
        "left-third",
        "Left Third",
        &["snap", "column"],
    ),
    (
        WindowLayout::MiddleThird,
        "middle-third",
        "Middle Third",
        &["snap", "column"],
    ),
    (
        WindowLayout::RightThird,
        "right-third",
        "Right Third",
        &["snap", "column"],
    ),
    (
        WindowLayout::LeftTwoThirds,
        "left-two-thirds",
        "Left Two Thirds",
        &["snap", "column"],
    ),
    (
        WindowLayout::RightTwoThirds,
        "right-two-thirds",
        "Right Two Thirds",
        &["snap", "column"],
    ),
    (
        WindowLayout::NextMonitor,
        "next-monitor",
        "Move to Next Monitor",
        &["screen", "display", "throw"],
    ),
    (
        WindowLayout::PreviousMonitor,
        "previous-monitor",
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

/// The titles the layouts had when their ids were built from them, frozen.
///
/// A copy rather than a read of `LAYOUTS`, so a title renamed later cannot
/// quietly stop its old history from being carried over.
const LEGACY_TITLES: &[(&str, &str)] = &[
    ("maximize", "Maximise Window"),
    ("center", "Centre Window"),
    ("left-half", "Left Half"),
    ("right-half", "Right Half"),
    ("top-half", "Top Half"),
    ("bottom-half", "Bottom Half"),
    ("top-left-quarter", "Top Left Quarter"),
    ("top-right-quarter", "Top Right Quarter"),
    ("bottom-left-quarter", "Bottom Left Quarter"),
    ("bottom-right-quarter", "Bottom Right Quarter"),
    ("left-third", "Left Third"),
    ("middle-third", "Middle Third"),
    ("right-third", "Right Third"),
    ("left-two-thirds", "Left Two Thirds"),
    ("right-two-thirds", "Right Two Thirds"),
    ("next-monitor", "Move to Next Monitor"),
    ("previous-monitor", "Move to Previous Monitor"),
];

/// Pairs of (title-based id, slug id) for every layout that had one.
///
/// The one-time migration from the ids these items had before they had
/// slugs, `window:{title}`, which is what frecency and favourites written by
/// older builds hold (F17).
pub fn legacy_ids() -> impl Iterator<Item = (String, String)> {
    LEGACY_TITLES
        .iter()
        .map(|&(slug, title)| (format!("window:{title}"), id(slug)))
}

/// Move `frecency` off the legacy ids. Whether anything changed.
pub fn migrate_frecency(frecency: &mut Frecency) -> bool {
    legacy_ids().fold(false, |changed, (from, to)| {
        frecency.rename(&from, &to) | changed
    })
}

/// Move `favorites` off the legacy ids. Whether anything changed.
pub fn migrate_favorites(favorites: &mut Favorites) -> bool {
    legacy_ids().fold(false, |changed, (from, to)| {
        favorites.rename(&from, &to) | changed
    })
}

fn id(slug: &str) -> String {
    format!("window:{slug}")
}

fn item(&(layout, slug, title, keywords): &(WindowLayout, &str, &str, &[&str])) -> Item {
    Item {
        id: id(slug),
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
    fn ids_are_window_and_a_kebab_slug() {
        for item in ArrangeProvider.catalogue() {
            let slug = item.id.strip_prefix("window:").unwrap();
            assert!(!slug.is_empty(), "{} has no slug", item.id);
            assert!(
                slug.split('-')
                    .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase())),
                "{} is not lowercase kebab",
                item.id
            );
        }
        let left = ArrangeProvider
            .catalogue()
            .into_iter()
            .find(|item| item.title == "Left Half")
            .unwrap();
        assert_eq!(left.id, "window:left-half");
    }

    #[test]
    fn legacy_ids_map_title_ids_to_slug_ids_that_exist() {
        let ids: Vec<String> = ArrangeProvider
            .catalogue()
            .into_iter()
            .map(|item| item.id)
            .collect();
        let pairs: Vec<(String, String)> = legacy_ids().collect();
        assert_eq!(pairs.len(), 17, "the layouts that had title-based ids");
        for (from, to) in &pairs {
            assert!(ids.contains(to), "{from} migrates to missing {to}");
        }
        assert!(pairs.contains(&("window:Left Half".into(), "window:left-half".into())));
    }

    #[test]
    fn migration_moves_frecency_and_favourites() {
        let mut frecency = crate::frecency::Frecency::new();
        frecency.record("window:Maximise Window");
        assert!(migrate_frecency(&mut frecency));
        assert!(frecency.boost("window:maximize") > 0);
        assert_eq!(frecency.boost("window:Maximise Window"), 0);
        assert!(!migrate_frecency(&mut frecency));

        let mut favorites = crate::favorites::Favorites::new();
        favorites.apply(crate::favorites::Change::Add, "app:code", &[]);
        favorites.apply(crate::favorites::Change::Add, "window:Left Half", &[]);
        assert!(migrate_favorites(&mut favorites));
        assert_eq!(favorites.ids(), ["app:code", "window:left-half"]);
        assert!(!migrate_favorites(&mut favorites));
    }

    #[test]
    fn ids_are_unique() {
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
