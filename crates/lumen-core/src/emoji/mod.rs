//! The emoji and symbols Lumen can search and paste (F18).
//!
//! # Bundled, not fetched
//!
//! Unicode's `emoji-test.txt`, named and keyworded by the CLDR annotations,
//! plus the symbol blocks people reach for -- arrows, maths, currency, box
//! drawing -- reduced by `scripts/make-emoji.ps1` to one line per entry and
//! compiled into the binary, as the places are (F13).
//!
//! Skin-tone variants are not entries: an entry that takes a tone carries the
//! five Unicode lists for it, written whole -- a modifier goes after each
//! person in a sequence, which only Unicode's own list gets right -- and the
//! user's [`SkinTone`] picks one (F18, task 8).
//!
//! # Loaded once, when first asked for
//!
//! A few thousand lines is little, but still not done at start: a daemon
//! nobody asks for an emoji never pays for it. [`catalogue`] reads it on
//! first use and keeps it, and [`loaded`] answers without waiting (I-3).

mod search;
pub mod session;
pub mod store;

use std::collections::BTreeMap;
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
    /// The entry in each of the five tones, light to dark, exactly as
    /// `emoji-test.txt` writes them; `None` when it takes no tone. Boxed, so
    /// an entry without tones pays one pointer.
    tones: Option<Box<[Box<str>; 5]>>,
    /// The name and keywords, folded, for matching.
    folded_name: Box<str>,
    folded_keywords: Box<[Box<str>]>,
}

impl Entry {
    /// Whether the entry takes a skin tone.
    pub fn has_tones(&self) -> bool {
        self.tones.is_some()
    }

    /// What is shown and pasted in `tone`: the entry's own characters for
    /// [`SkinTone::Default`] or when it takes no tone.
    pub fn toned(&self, tone: SkinTone) -> &str {
        match (&self.tones, tone.index()) {
            (Some(tones), Some(index)) => &tones[index],
            _ => &self.chars,
        }
    }
}

/// The skin tone emoji are shown and pasted in, as Raycast offers it: the
/// default yellow, or one of the five Fitzpatrick modifiers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SkinTone {
    #[default]
    Default,
    Light,
    MediumLight,
    Medium,
    MediumDark,
    Dark,
}

impl SkinTone {
    pub const ALL: [SkinTone; 6] = [
        SkinTone::Default,
        SkinTone::Light,
        SkinTone::MediumLight,
        SkinTone::Medium,
        SkinTone::MediumDark,
        SkinTone::Dark,
    ];

    /// How `emoji.json` writes it.
    pub fn slug(self) -> &'static str {
        match self {
            SkinTone::Default => "default",
            SkinTone::Light => "light",
            SkinTone::MediumLight => "medium-light",
            SkinTone::Medium => "medium",
            SkinTone::MediumDark => "medium-dark",
            SkinTone::Dark => "dark",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        SkinTone::ALL.into_iter().find(|tone| tone.slug() == slug)
    }

    pub fn label(self) -> &'static str {
        match self {
            SkinTone::Default => "Default",
            SkinTone::Light => "Light",
            SkinTone::MediumLight => "Medium-Light",
            SkinTone::Medium => "Medium",
            SkinTone::MediumDark => "Medium-Dark",
            SkinTone::Dark => "Dark",
        }
    }

    /// Its place among an entry's five variants; `None` for the default.
    fn index(self) -> Option<usize> {
        (self as usize).checked_sub(1)
    }

    /// The Fitzpatrick modifier, U+1F3FB to U+1F3FF; `None` for the default.
    pub fn modifier(self) -> Option<char> {
        self.index()
            .and_then(|index| char::from_u32(0x1F3FB + index as u32))
    }
}

/// Whether `c` is one of the five skin-tone modifiers.
pub fn is_skin_tone(c: char) -> bool {
    ('\u{1F3FB}'..='\u{1F3FF}').contains(&c)
}

/// Written by its slug. Read leniently, as the rest of `emoji.json` is: a
/// tone this version does not know, or not a string at all, is the default
/// rather than a file that fails to load (I-12).
impl serde::Serialize for SkinTone {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.slug())
    }
}

impl<'de> serde::Deserialize<'de> for SkinTone {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(value
            .as_str()
            .and_then(SkinTone::from_slug)
            .unwrap_or_default())
    }
}

/// Keywords the user added, by an entry's [`Entry::chars`] (F18, task 7).
/// Matched as the entry's own are. Ordered, as `emoji.json` keeps them.
pub type CustomKeywords = BTreeMap<String, Vec<String>>;

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
            let [chars, category, subgroup, name, keywords, tones] = fields[..] else {
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
                tones: parse_tones(tones),
            });
        }
        Catalogue { entries }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
}

/// The last column: the five toned variants joined by `|`, or nothing. Any
/// other count is a generator gone wrong, and the entry takes no tone rather
/// than the wrong one.
fn parse_tones(field: &str) -> Option<Box<[Box<str>; 5]>> {
    let tones: Vec<Box<str>> = field.split('|').map(Into::into).collect();
    if tones.iter().any(|tone| tone.is_empty()) {
        return None;
    }
    <[Box<str>; 5]>::try_from(tones).ok().map(Box::new)
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

    /// The tones are variants of an entry, not entries, so a modifier in an
    /// entry's own characters is a generator that let one through.
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
    fn the_tones_column_reads_five_variants_or_none() {
        let catalogue = Catalogue::parse(
            "\u{270B}\tsmileys-people\thand-fingers-open\traised hand\thand\t\
             \u{270B}\u{1F3FB}|\u{270B}\u{1F3FC}|\u{270B}\u{1F3FD}|\u{270B}\u{1F3FE}|\u{270B}\u{1F3FF}\n\
             \u{1F600}\tsmileys-people\tface-smiling\tgrinning face\tface\t\n\
             \u{1F44D}\tsmileys-people\thand-fingers-closed\tthumbs up\t\t\u{1F44D}\u{1F3FB}|\u{1F44D}\u{1F3FC}\n",
        );
        let [hand, grin, short] = catalogue.entries() else {
            panic!("three entries")
        };
        assert!(hand.has_tones());
        assert_eq!(hand.toned(SkinTone::Default), "\u{270B}");
        assert_eq!(hand.toned(SkinTone::Light), "\u{270B}\u{1F3FB}");
        assert_eq!(hand.toned(SkinTone::Medium), "\u{270B}\u{1F3FD}");
        assert_eq!(hand.toned(SkinTone::Dark), "\u{270B}\u{1F3FF}");
        assert!(!grin.has_tones());
        assert_eq!(grin.toned(SkinTone::Dark), "\u{1F600}");
        // Incomplete: no tone rather than a wrong one.
        assert!(!short.has_tones());
        assert_eq!(short.toned(SkinTone::Light), "\u{1F44D}");
    }

    fn bundled(chars: &str) -> &'static Entry {
        catalogue()
            .entries()
            .iter()
            .find(|entry| &*entry.chars == chars)
            .unwrap_or_else(|| panic!("no {chars}"))
    }

    #[test]
    fn the_bundled_tones_are_unicodes_own_sequences() {
        let thumbs = bundled("\u{1F44D}");
        assert_eq!(thumbs.toned(SkinTone::Medium), "\u{1F44D}\u{1F3FD}");

        // People holding hands: a modifier after each person, which a
        // modifier inserted after the first character would not give.
        let holding = bundled("\u{1F9D1}\u{200D}\u{1F91D}\u{200D}\u{1F9D1}");
        assert_eq!(
            holding.toned(SkinTone::Medium),
            "\u{1F9D1}\u{1F3FD}\u{200D}\u{1F91D}\u{200D}\u{1F9D1}\u{1F3FD}"
        );
        // Index pointing up is written with U+FE0F; toned, Unicode drops it.
        let up = bundled("\u{261D}\u{FE0F}");
        assert_eq!(up.toned(SkinTone::Dark), "\u{261D}\u{1F3FF}");

        assert!(!bundled("\u{1F600}").has_tones());
        let toned = catalogue()
            .entries()
            .iter()
            .filter(|e| e.has_tones())
            .count();
        assert!((250..=450).contains(&toned), "{toned} take a tone");
        for entry in catalogue().entries().iter().filter(|e| e.has_tones()) {
            for tone in &SkinTone::ALL[1..] {
                let modifier = tone.modifier().unwrap();
                let toned = entry.toned(*tone);
                assert!(
                    toned
                        .chars()
                        .filter(|c| is_skin_tone(*c))
                        .all(|c| c == modifier),
                    "{toned} in {tone:?}"
                );
                assert!(toned.contains(modifier), "{toned} in {tone:?}");
            }
        }
    }

    #[test]
    fn skin_tones_have_their_modifiers_and_round_trip() {
        assert_eq!(SkinTone::Default.modifier(), None);
        assert_eq!(SkinTone::Light.modifier(), Some('\u{1F3FB}'));
        assert_eq!(SkinTone::Dark.modifier(), Some('\u{1F3FF}'));
        for tone in SkinTone::ALL {
            assert_eq!(SkinTone::from_slug(tone.slug()), Some(tone));
        }
        assert_eq!(SkinTone::MediumLight.label(), "Medium-Light");
        assert!(is_skin_tone('\u{1F3FD}'));
        assert!(!is_skin_tone('\u{1F44D}'));
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
