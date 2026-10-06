//! The emoji and symbols Lumen can search and paste (F18).
//!
//! # Bundled, not fetched
//!
//! Unicode's `emoji-test.txt`, named and keyworded by the CLDR annotations,
//! plus the symbol blocks people reach for -- arrows, maths, currency, box
//! drawing -- reduced by `scripts/make-emoji.ps1` to one line per entry and
//! compiled into the binary, as the places are (F13).
//!
//! Skin-tone variants are not in it: an entry that takes a tone says so, and
//! the tones are a later decision (F18, deferred).
//!
//! # Loaded once, when first asked for
//!
//! A few thousand lines is little, but still not done at start: a daemon
//! nobody asks for an emoji never pays for it. [`catalogue`] reads it on
//! first use and keeps it, and [`loaded`] answers without waiting (I-3).

mod search;
pub mod session;
pub mod store;

use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

use crate::places::fold;

const EMOJI: &[u8] = include_bytes!("../../../../assets/emoji/emoji.tsv.gz");

/// Where an entry is filed: Unicode's emoji groups, then the symbol blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    SmileysPeople,
    AnimalsNature,
    FoodDrink,
    TravelPlaces,
    Activities,
    Objects,
    Symbols,
    Flags,
    Arrows,
    Math,
    Currency,
    Letterlike,
    GeometricShapes,
    Punctuation,
    Technical,
    BoxDrawing,
}

impl Category {
    /// In the order the dropdown lists them, which is the catalogue's.
    pub const ALL: [Category; 16] = [
        Category::SmileysPeople,
        Category::AnimalsNature,
        Category::FoodDrink,
        Category::TravelPlaces,
        Category::Activities,
        Category::Objects,
        Category::Symbols,
        Category::Flags,
        Category::Arrows,
        Category::Math,
        Category::Currency,
        Category::Letterlike,
        Category::GeometricShapes,
        Category::Punctuation,
        Category::Technical,
        Category::BoxDrawing,
    ];

    /// What the generator writes in the second column.
    pub fn slug(self) -> &'static str {
        match self {
            Category::SmileysPeople => "smileys-people",
            Category::AnimalsNature => "animals-nature",
            Category::FoodDrink => "food-drink",
            Category::Activities => "activities",
            Category::TravelPlaces => "travel-places",
            Category::Objects => "objects",
            Category::Symbols => "symbols",
            Category::Flags => "flags",
            Category::Arrows => "arrows",
            Category::Math => "math",
            Category::Currency => "currency",
            Category::Letterlike => "letterlike",
            Category::GeometricShapes => "geometric-shapes",
            Category::Punctuation => "punctuation",
            Category::Technical => "technical",
            Category::BoxDrawing => "box-drawing",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Category::ALL
            .into_iter()
            .find(|category| category.slug() == slug)
    }

    pub fn label(self) -> &'static str {
        match self {
            Category::SmileysPeople => "Smileys & People",
            Category::AnimalsNature => "Animals & Nature",
            Category::FoodDrink => "Food & Drink",
            Category::Activities => "Activities",
            Category::TravelPlaces => "Travel & Places",
            Category::Objects => "Objects",
            Category::Symbols => "Symbols",
            Category::Flags => "Flags",
            Category::Arrows => "Arrows",
            Category::Math => "Math",
            Category::Currency => "Currency",
            Category::Letterlike => "Letterlike",
            Category::GeometricShapes => "Geometric Shapes",
            Category::Punctuation => "Punctuation",
            Category::Technical => "Technical",
            Category::BoxDrawing => "Box Drawing",
        }
    }

    /// An emoji group rather than a symbol block: drawn from an image where
    /// there is one, where a symbol is always drawn with the font.
    pub fn is_emoji(self) -> bool {
        self <= Category::Flags
    }
}

/// One emoji or symbol.
#[derive(Debug)]
pub struct Entry {
    /// What is pasted: one code point, or a sequence with its joiners and
    /// variation selectors.
    pub chars: Box<str>,
    /// CLDR's short name, lowercase: *grinning face*.
    pub name: Box<str>,
    pub keywords: Box<[Box<str>]>,
    pub category: Category,
    /// Unicode's subgroup -- *face-smiling* -- or the symbol block's.
    pub subgroup: Box<str>,
    /// Whether the entry takes a skin tone. The tones are not in the
    /// catalogue; this is what a later version would offer them on.
    pub skin_tones: bool,
    /// The name and keywords, folded, for matching.
    folded_name: Box<str>,
    folded_keywords: Box<[Box<str>]>,
}

/// Keywords the user added, by an entry's [`Entry::chars`] (F18, task 8).
/// Matched as the entry's own are.
pub type CustomKeywords = HashMap<String, Vec<String>>;

/// Every entry, in Unicode's order: the emoji groups as `emoji-test.txt`
/// lists them, then the symbol blocks.
#[derive(Debug)]
pub struct Catalogue {
    entries: Vec<Entry>,
}

static CATALOGUE: OnceLock<Catalogue> = OnceLock::new();

/// The catalogue if it has been read, without reading it: what the UI
/// thread asks, which must not wait for it (I-3).
pub fn loaded() -> Option<&'static Catalogue> {
    CATALOGUE.get()
}

/// The catalogue, read on first use and kept. Blocks for as long as
/// reading takes, so the UI thread asks [`loaded`] instead.
pub fn catalogue() -> &'static Catalogue {
    CATALOGUE.get_or_init(|| {
        let started = std::time::Instant::now();
        let catalogue = Catalogue::bundled();
        log::info!(
            "read {} emoji and symbols in {:?}",
            catalogue.entries.len(),
            started.elapsed()
        );
        catalogue
    })
}

impl Catalogue {
    /// The catalogue compiled into the binary.
    fn bundled() -> Self {
        let mut text = String::new();
        if let Err(error) = flate2::read::GzDecoder::new(EMOJI).read_to_string(&mut text) {
            // The file is the binary's own, so this is a broken build, not
            // a broken machine: say so, and run with no emoji.
            log::error!("the bundled emoji are unreadable: {error}");
            text.clear();
        }
        Self::parse(&text)
    }

    /// Read the file `scripts/make-emoji.ps1` writes. A line that does not
    /// parse is skipped, not fatal: one bad row must not cost every entry.
    fn parse(text: &str) -> Self {
        let mut entries = Vec::new();
        for line in text.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            let [chars, category, subgroup, name, keywords, skin_tones] = fields[..] else {
                continue;
            };
            let Some(category) = Category::from_slug(category) else {
                continue;
            };
            if chars.is_empty() || name.is_empty() {
                continue;
            }
            let keywords: Box<[Box<str>]> = keywords
                .split('|')
                .filter(|keyword| !keyword.is_empty())
                .map(Into::into)
                .collect();
            entries.push(Entry {
                chars: chars.into(),
                name: name.into(),
                folded_name: fold(name).into(),
                folded_keywords: keywords.iter().map(|k| fold(k).into()).collect(),
                keywords,
                category,
                subgroup: subgroup.into(),
                skin_tones: skin_tones == "1",
            });
        }
        Catalogue { entries }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
}

/// The code points of `chars` as Unicode writes them: `U+1F600`, a
/// sequence separated by spaces. A variation selector is shown where it is,
/// because it is part of what gets pasted.
pub fn codepoints(chars: &str) -> String {
    chars
        .chars()
        .map(|c| format!("U+{:04X}", u32::from(c)))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_catalogue_reads_whole() {
        let catalogue = catalogue();
        let entries = catalogue.entries();
        assert!(entries.len() > 2_000, "{} entries", entries.len());
        assert!(entries.len() < 10_000, "{} entries", entries.len());

        let emoji = entries.iter().filter(|e| e.category.is_emoji()).count();
        assert!(emoji > 1_000, "{emoji} emoji");
        assert!(
            entries.len() - emoji > 500,
            "{} symbols",
            entries.len() - emoji
        );

        for category in Category::ALL {
            assert!(
                entries.iter().any(|entry| entry.category == category),
                "nothing in {category:?}"
            );
        }
    }

    #[test]
    fn every_entry_is_named() {
        for entry in catalogue().entries() {
            assert!(!entry.name.trim().is_empty(), "{:?}", entry.chars);
            assert!(!entry.folded_name.is_empty(), "{:?}", entry.name);
        }
    }

    /// The tones are deferred (F18), so a modifier in the catalogue is a
    /// generator that let one through.
    #[test]
    fn no_entry_carries_a_skin_tone() {
        for entry in catalogue().entries() {
            assert!(
                !entry
                    .chars
                    .chars()
                    .any(|c| ('\u{1F3FB}'..='\u{1F3FF}').contains(&c)),
                "{} ({}) carries a tone",
                entry.chars,
                entry.name
            );
        }
    }

    #[test]
    fn the_categories_round_trip_and_split_emoji_from_symbols() {
        for category in Category::ALL {
            assert_eq!(Category::from_slug(category.slug()), Some(category));
        }
        assert!(Category::SmileysPeople.is_emoji());
        assert!(Category::Flags.is_emoji());
        assert!(!Category::Arrows.is_emoji());
        assert!(!Category::BoxDrawing.is_emoji());
        assert_eq!(Category::GeometricShapes.label(), "Geometric Shapes");
    }

    #[test]
    fn a_malformed_line_is_skipped_not_fatal() {
        let catalogue = Catalogue::parse(
            "\u{1F600}\tsmileys-people\tface-smiling\tgrinning face\tface|grin\t0\n\
             broken line\n\
             x\tnowhere\tnone\tunknown\t\t0\n\
             \u{2192}\tarrows\tarrows\trightwards arrow\t\t0\n",
        );
        let names: Vec<&str> = catalogue.entries().iter().map(|e| &*e.name).collect();
        assert_eq!(names, ["grinning face", "rightwards arrow"]);
        assert_eq!(
            &*catalogue.entries()[0].keywords,
            ["face".into(), "grin".into()]
        );
        assert!(catalogue.entries()[1].keywords.is_empty());
    }

    #[test]
    fn codepoints_are_written_as_unicode_writes_them() {
        assert_eq!(codepoints("\u{1F600}"), "U+1F600");
        assert_eq!(codepoints("\u{2600}\u{FE0F}"), "U+2600 U+FE0F");
        assert_eq!(
            codepoints("\u{1F468}\u{200D}\u{1F469}"),
            "U+1F468 U+200D U+1F469"
        );
        assert_eq!(codepoints("\u{2192}"), "U+2192");
    }
}
