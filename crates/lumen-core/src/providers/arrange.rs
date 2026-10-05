//! Window management as commands (F3).
//!
//! A genuine catalogue provider, unlike the clipboard: forty-one fixed entries
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
/// A table rather than a `match` with forty-one arms so that adding a layout is
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
        &["snap", "column", "first"],
    ),
    (
        WindowLayout::MiddleThird,
        "middle-third",
        "Middle Third",
        &["snap", "column", "center", "centre"],
    ),
    (
        WindowLayout::RightThird,
        "right-third",
        "Right Third",
        &["snap", "column", "last"],
    ),
    (
        WindowLayout::LeftTwoThirds,
        "left-two-thirds",
        "Left Two Thirds",
        &["snap", "column", "first"],
    ),
    (
        WindowLayout::RightTwoThirds,
        "right-two-thirds",
        "Right Two Thirds",
        &["snap", "column", "last"],
    ),
    (
        WindowLayout::MaximizeHeight,
        "maximize-height",
        "Maximise Height",
        &["maximize", "vertical", "tall", "stretch"],
    ),
    (
        WindowLayout::MaximizeWidth,
        "maximize-width",
        "Maximise Width",
        &["maximize", "horizontal", "wide", "stretch"],
    ),
    (
        WindowLayout::AlmostMaximize,
        "almost-maximize",
        "Almost Maximise",
        &["maximize", "almost", "nearly", "large"],
    ),
    (
        WindowLayout::ReasonableSize,
        "reasonable-size",
        "Reasonable Size",
        &["resize", "medium", "center", "centre"],
    ),
    (
        WindowLayout::CenterKeepSize,
        "center-keep-size",
        "Centre Without Resizing",
        &["center", "middle", "keep", "size"],
    ),
    (
        WindowLayout::FirstFourth,
        "first-fourth",
        "First Fourth",
        &["snap", "column", "quarter", "left"],
    ),
    (
        WindowLayout::SecondFourth,
        "second-fourth",
        "Second Fourth",
        &["snap", "column", "quarter"],
    ),
    (
        WindowLayout::ThirdFourth,
        "third-fourth",
        "Third Fourth",
        &["snap", "column", "quarter"],
    ),
    (
        WindowLayout::LastFourth,
        "last-fourth",
        "Last Fourth",
        &["snap", "column", "quarter", "right"],
    ),
    (
        WindowLayout::TopLeftSixth,
        "top-left-sixth",
        "Top Left Sixth",
        &["snap", "grid"],
    ),
    (
        WindowLayout::TopCenterSixth,
        "top-center-sixth",
        "Top Centre Sixth",
        &["snap", "grid", "center", "middle"],
    ),
    (
        WindowLayout::TopRightSixth,
        "top-right-sixth",
        "Top Right Sixth",
        &["snap", "grid"],
    ),
    (
        WindowLayout::BottomLeftSixth,
        "bottom-left-sixth",
        "Bottom Left Sixth",
        &["snap", "grid"],
    ),
    (
        WindowLayout::BottomCenterSixth,
        "bottom-center-sixth",
        "Bottom Centre Sixth",
        &["snap", "grid", "center", "middle"],
    ),
    (
        WindowLayout::BottomRightSixth,
        "bottom-right-sixth",
        "Bottom Right Sixth",
        &["snap", "grid"],
    ),
    (
        WindowLayout::MoveUp,
        "move-up",
        "Move Up",
        &["edge", "top", "nudge"],
    ),
    (
        WindowLayout::MoveDown,
        "move-down",
        "Move Down",
        &["edge", "bottom", "nudge"],
    ),
    (
        WindowLayout::MoveLeft,
        "move-left",
        "Move Left",
        &["edge", "nudge"],
    ),
    (
        WindowLayout::MoveRight,
        "move-right",
        "Move Right",
        &["edge", "nudge"],
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
    (
        WindowLayout::Minimize,
        "minimize",
        "Minimise Window",
        &["minimize", "hide"],
    ),
    (
        WindowLayout::MinimizeDisplay,
        "minimize-display",
        "Minimise on This Display",
        &[
            "minimize",
            "show desktop",
            "minimize all",
            "monitor",
            "screen",
        ],
    ),
    (
        WindowLayout::Restore,
        "restore",
        "Restore Window",
        &["undo", "previous", "unsnap", "back"],
    ),
    (
        WindowLayout::FocusNextAppWindow,
        "focus-next-app-window",
        "Focus Next App Window",
        &["switch", "cycle", "next"],
    ),
    (
        WindowLayout::FocusPreviousAppWindow,
        "focus-previous-app-window",
        "Focus Previous App Window",
        &["switch", "cycle", "previous"],
    ),
];

/// What the subtitle says on every one of these rows.
///
/// The same line on all of them on purpose. It is not describing the layout --
/// the title does that, and "Left Half" needs no gloss -- it is answering the
/// question the user actually has the first time one of these appears, which
/// is *which* window this is about.
pub(crate) const SUBTITLE: &str = "The window you were just using";

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

        // Every British spelling in a title, on every row that has one.
        for item in &items {
            let title = item.title.to_lowercase();
            for (british, american) in [
                ("maximise", "maximize"),
                ("minimise", "minimize"),
                ("centre", "center"),
            ] {
                if title.contains(british) {
                    assert!(
                        item.keywords.iter().any(|k| k == american),
                        "{} has no {american} keyword",
                        item.title
                    );
                }
            }
        }
    }

    /// Raycast's names for the thirds find this project's.
    #[test]
    fn the_thirds_answer_to_first_centre_and_last() {
        let items = ArrangeProvider.catalogue();
        let keywords = |title: &str| {
            items
                .iter()
                .find(|item| item.title == title)
                .unwrap()
                .keywords
                .clone()
        };
        assert!(keywords("Left Third").contains(&"first".to_string()));
        assert!(keywords("Middle Third").contains(&"center".to_string()));
        assert!(keywords("Right Third").contains(&"last".to_string()));
        assert!(keywords("Left Two Thirds").contains(&"first".to_string()));
        assert!(keywords("Right Two Thirds").contains(&"last".to_string()));
    }

    /// The slug is the layout's own serialised name, so an id, a stored
    /// layout and a hotkey binding all spell a layout the same way.
    #[test]
    fn every_slug_is_the_layouts_serialised_name() {
        for &(layout, slug, ..) in LAYOUTS {
            assert_eq!(serde_json::to_value(layout).unwrap(), slug, "{layout:?}");
        }
    }

    #[test]
    fn the_raycast_layouts_are_offered_by_their_slugs() {
        let ids: Vec<String> = ArrangeProvider
            .catalogue()
            .into_iter()
            .map(|item| item.id)
            .collect();
        for slug in [
            "maximize-height",
            "maximize-width",
            "almost-maximize",
            "reasonable-size",
            "center-keep-size",
            "first-fourth",
            "second-fourth",
            "third-fourth",
            "last-fourth",
            "top-left-sixth",
            "top-center-sixth",
            "top-right-sixth",
            "bottom-left-sixth",
            "bottom-center-sixth",
            "bottom-right-sixth",
            "move-up",
            "move-down",
            "move-left",
            "move-right",
        ] {
            assert!(ids.contains(&id(slug)), "window:{slug} is not offered");
        }
        assert_eq!(ids.len(), 41);
    }

    /// Minimise, restore and focus, by their slugs and British titles.
    #[test]
    fn the_window_commands_are_offered() {
        let items = ArrangeProvider.catalogue();
        for (slug, title) in [
            ("minimize", "Minimise Window"),
            ("minimize-display", "Minimise on This Display"),
            ("restore", "Restore Window"),
            ("focus-next-app-window", "Focus Next App Window"),
            ("focus-previous-app-window", "Focus Previous App Window"),
        ] {
            let item = items
                .iter()
                .find(|item| item.id == id(slug))
                .unwrap_or_else(|| panic!("window:{slug} is not offered"));
            assert_eq!(item.title, title);
        }
    }

    /// What people type for these, Raycast's words and the system's.
    #[test]
    fn the_window_commands_answer_to_what_people_type() {
        let items = ArrangeProvider.catalogue();
        let keywords = |slug: &str| {
            items
                .iter()
                .find(|item| item.id == id(slug))
                .unwrap()
                .keywords
                .clone()
        };
        let has = |slug: &str, word: &str| keywords(slug).iter().any(|k| k == word);

        assert!(has("minimize", "minimize"));
        assert!(has("minimize", "hide"));
        assert!(has("minimize-display", "show desktop"));
        assert!(has("minimize-display", "minimize all"));
        assert!(has("minimize-display", "monitor"));
        assert!(has("restore", "undo"));
        assert!(has("restore", "unsnap"));
        assert!(has("focus-next-app-window", "switch"));
        assert!(has("focus-previous-app-window", "switch"));
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
