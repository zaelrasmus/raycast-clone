//! Styles: the launcher's geometry and variants, as data (F15).
//!
//! A palette -- what [`crate::theme`] loads -- is colour and the type family.
//! A style is the rest of the look: how tall a row is, how large the query is
//! set, whether the subtitle sits under the title or beside it. The two are
//! chosen apart, so either can change without the other (AD-13, revised).
//!
//! The rules are the palette's, and for the same reasons:
//!
//! - **Default resolution.** Every token a style does not set comes from the
//!   base style, which is the constants in [`crate::layout`] as they were. A
//!   style that sets nothing draws the launcher exactly as it was (I-12).
//! - **A closed vocabulary.** Tokens and variants are listed here. A style
//!   cannot describe a layout, only pick numbers and variants, so the UI tree
//!   still exists in one place (I-13).
//! - **Bounds instead of distance.** AD-13 used to keep geometry out of reach
//!   of a theme altogether. Here every token has a range, and the relations
//!   the constants were asserted to keep are checked when a style loads. What
//!   is out of range is clamped and what breaks a relation goes back to the
//!   base, with a warning each, never an error: a style written for a later
//!   Lumen must still load.
//!
//! Only the launcher's surface reads a style: the search field, the rows, the
//! section headers, the calculator's card and the footer, in the root list and
//! in views. The action panel, the forms, the overlay and the settings window
//! keep the constants (7.41).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::layout;

/// The schema version this build writes and understands.
pub const SCHEMA_VERSION: u32 = 1;

/// The renderer's line height, as a multiple of the type size.
///
/// gpui's default (`phi()`), which Lumen never overrides. Used only to check
/// that type fits the line it is set in: a 28 pt query in a 40 pt field is a
/// field that clips its own text.
pub const LINE_HEIGHT: f32 = 1.618;

/// How much taller than its label's type size a keycap must be.
pub const KEYCAP_FIT: f32 = 1.2;

/// The weights the bundled typeface ships (`assets/fonts`). A weight between
/// them would be synthesised, which is what makes type look cheap.
pub const WEIGHTS: [f32; 3] = [400.0, 500.0, 600.0];

/// The launcher's geometry, resolved: every token present.
///
/// Logical pixels, except the weights. Read by the renderer as it draws, so
/// a field here is a token a style may set; the table in [`Metrics::slot`] is
/// what makes it settable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub window_width: f32,
    /// How tall the list may grow before it scrolls; it ends on whole rows.
    pub list_height: f32,
    pub window_radius: f32,
    pub search_height: f32,
    pub gutter: f32,
    pub list_padding: f32,
    pub row_height: f32,
    pub row_radius: f32,
    pub row_inset: f32,
    pub icon_size: f32,
    pub icon_gap: f32,
    pub subtitle_gap: f32,
    pub section_height: f32,
    pub footer_height: f32,
    pub footer_icon: f32,
    pub keycap_height: f32,
    pub keycap_padding: f32,
    pub keycap_radius: f32,
    pub keycap_gap: f32,
    pub text_query: f32,
    pub text_title: f32,
    pub text_subtitle: f32,
    pub text_accessory: f32,
    pub text_section: f32,
    pub text_footer: f32,
    pub text_card: f32,
    pub weight_query: f32,
    pub weight_title: f32,
    pub weight_section: f32,
}

impl Metrics {
    /// The constants as F14 left them. The base every style resolves from.
    pub const BASE: Metrics = Metrics {
        // The config's defaults before F15: 720 wide, eight rows.
        window_width: 720.0,
        list_height: 8.0 * layout::ROW_HEIGHT,
        window_radius: layout::WINDOW_RADIUS,
        search_height: layout::SEARCH_HEIGHT,
        gutter: layout::GUTTER,
        list_padding: layout::LIST_PADDING,
        row_height: layout::ROW_HEIGHT,
        row_radius: layout::ROW_RADIUS,
        row_inset: layout::ROW_INSET,
        icon_size: layout::ICON_SIZE,
        icon_gap: layout::ICON_GAP,
        // Only drawn when the subtitle is inline; the stacked base never
        // reads it.
        subtitle_gap: 8.0,
        section_height: layout::view::SECTION_HEIGHT,
        footer_height: layout::FOOTER_HEIGHT,
        // Drawn only when `footer.icon` is not `none`.
        footer_icon: 14.0,
        keycap_height: 20.0,
        keycap_padding: layout::overlay::KEYCAP_PADDING_X,
        keycap_radius: layout::overlay::KEYCAP_RADIUS,
        keycap_gap: layout::overlay::KEYCAP_GAP,
        text_query: layout::text::QUERY,
        text_title: layout::text::TITLE,
        text_subtitle: layout::text::SUBTITLE,
        // The kind label and a view's section title were set at the footer's
        // size; they are tokens of their own now, at that size.
        text_accessory: layout::text::FOOTER,
        text_section: layout::text::FOOTER,
        text_footer: layout::text::FOOTER,
        text_card: layout::text::CARD,
        weight_query: 400.0,
        weight_title: 400.0,
        weight_section: 400.0,
    };

    /// The token names a style may set, each with where it lands and its
    /// range.
    ///
    /// One table, used both to apply a style and to report unknown tokens,
    /// as [`crate::theme`] does for colours: two lists would drift.
    fn slot(&mut self, name: &str) -> Option<(&mut f32, f32, f32)> {
        Some(match name {
            "window.width" => (&mut self.window_width, 560.0, 1000.0),
            "list.height" => (&mut self.list_height, 200.0, 640.0),
            "window.radius" => (&mut self.window_radius, 0.0, 16.0),
            "search.height" => (&mut self.search_height, 40.0, 96.0),
            "gutter" => (&mut self.gutter, 8.0, 32.0),
            "list.padding" => (&mut self.list_padding, 0.0, 16.0),
            "row.height" => (&mut self.row_height, 28.0, 72.0),
            "row.radius" => (&mut self.row_radius, 0.0, 18.0),
            "row.inset" => (&mut self.row_inset, 0.0, 16.0),
            "icon.size" => (&mut self.icon_size, 14.0, 40.0),
            "icon.gap" => (&mut self.icon_gap, 4.0, 24.0),
            "subtitle.gap" => (&mut self.subtitle_gap, 4.0, 16.0),
            "section.height" => (&mut self.section_height, 16.0, 48.0),
            "footer.height" => (&mut self.footer_height, 28.0, 64.0),
            "footer.icon-size" => (&mut self.footer_icon, 10.0, 24.0),
            "keycap.height" => (&mut self.keycap_height, 14.0, 32.0),
            "keycap.padding" => (&mut self.keycap_padding, 2.0, 10.0),
            "keycap.radius" => (&mut self.keycap_radius, 0.0, 8.0),
            "keycap.gap" => (&mut self.keycap_gap, 0.0, 12.0),
            "text.query" => (&mut self.text_query, 12.0, 28.0),
            "text.title" => (&mut self.text_title, 11.0, 18.0),
            "text.subtitle" => (&mut self.text_subtitle, 10.0, 16.0),
            "text.accessory" => (&mut self.text_accessory, 10.0, 16.0),
            "text.section" => (&mut self.text_section, 10.0, 16.0),
            "text.footer" => (&mut self.text_footer, 10.0, 16.0),
            "text.card" => (&mut self.text_card, 14.0, 48.0),
            "weight.query" => (&mut self.weight_query, 400.0, 600.0),
            "weight.title" => (&mut self.weight_title, 400.0, 600.0),
            "weight.section" => (&mut self.weight_section, 400.0, 600.0),
            _ => return None,
        })
    }

    /// How much room a view's empty state is given: three rows, as
    /// `layout::view::EMPTY_HEIGHT` said when rows had one height.
    pub fn empty_height(&self) -> f32 {
        self.row_height * 3.0
    }

    /// Physical pixels to ask the icon extractor for.
    ///
    /// Three and a half times the square, never under what it was: a 26 pt
    /// icon is 52 physical pixels at 200%, and asking for exactly that is
    /// what makes icons soft (F1).
    pub fn icon_resolution(&self) -> u32 {
        (self.icon_size * 3.5)
            .ceil()
            .max(layout::ICON_RESOLUTION as f32) as u32
    }

    /// The relations between tokens, each with the tokens it involves.
    ///
    /// What the compile-time assertions in [`crate::layout`] said about the
    /// constants, said again about whatever a style sets. A relation that
    /// fails sends its tokens back to the base, which keeps every one of them
    /// (tested), so resolution always ends somewhere drawable.
    fn broken(&self, variants: &Variants) -> Option<(&'static str, &'static [&'static str])> {
        let line = |size: f32| size * LINE_HEIGHT;
        let row_text = match variants.row_subtitle {
            RowSubtitle::Stacked => line(self.text_title) + line(self.text_subtitle),
            RowSubtitle::Inline => line(self.text_title.max(self.text_subtitle)),
        };
        let checks: [(bool, &'static str, &'static [&'static str]); 12] = [
            (
                self.list_height >= self.row_height * 3.0,
                "the list holds at least three rows",
                &["list.height", "row.height"],
            ),
            (
                self.row_radius * 2.0 <= self.row_height,
                "a row's corner fits in the row",
                &["row.radius", "row.height"],
            ),
            (
                self.row_inset < self.gutter,
                "the selection's inset is inside the gutter",
                &["row.inset", "gutter"],
            ),
            (
                self.icon_size <= self.row_height,
                "the icon fits in the row",
                &["icon.size", "row.height"],
            ),
            (
                row_text <= self.row_height,
                "a row's text fits in the row",
                &["text.title", "text.subtitle", "row.height"],
            ),
            (
                line(self.text_accessory) <= self.row_height,
                "the accessory fits in the row",
                &["text.accessory", "row.height"],
            ),
            (
                line(self.text_query) <= self.search_height,
                "the query fits in the search field",
                &["text.query", "search.height"],
            ),
            (
                line(self.text_section) <= self.section_height,
                "a section's title fits in its header",
                &["text.section", "section.height"],
            ),
            (
                line(self.text_footer) <= self.footer_height,
                "the footer's text fits in the footer",
                &["text.footer", "footer.height"],
            ),
            (
                self.footer_icon <= self.footer_height,
                "the footer's icon fits in the footer",
                &["footer.icon-size", "footer.height"],
            ),
            // A keycap holds a label, not a line of text: the line's leading
            // may overhang it, the glyphs may not. Raycast sets 13.5 pt keys
            // in caps 18 tall.
            (
                self.keycap_height <= self.footer_height
                    && self.text_footer * KEYCAP_FIT <= self.keycap_height,
                "a keycap fits in the footer and holds its label",
                &["keycap.height", "footer.height", "text.footer"],
            ),
            (
                line(self.text_card) + layout::CARD_LABEL_GAP + line(self.text_footer)
                    <= self.row_height * layout::CARD_ROWS as f32,
                "the card's value and label fit in its rows",
                &["text.card", "text.footer", "row.height"],
            ),
        ];
        checks
            .into_iter()
            .find(|(holds, ..)| !holds)
            .map(|(_, what, tokens)| (what, tokens))
    }
}

/// Where a row's subtitle goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RowSubtitle {
    /// On a line of its own, under the title.
    #[default]
    Stacked,
    /// After the title, on the same line, muted.
    Inline,
}

/// How the selected row is marked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Selection {
    /// A fill and a border.
    #[default]
    FillBorder,
    /// A fill only.
    Fill,
}

/// How the footer shows the keys of what it offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FooterHints {
    /// `Actions  Ctrl+K`.
    #[default]
    Text,
    /// The keys drawn as keycaps.
    Keycaps,
}

/// How a keycap is drawn, when the footer draws them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Keycap {
    /// Filled, with a faint edge.
    #[default]
    Filled,
    /// An outline only, the footer showing through: Raycast's.
    Outline,
}

/// What the search field shows before the query (F15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchIcon {
    #[default]
    None,
    /// A magnifying glass.
    Search,
    /// Lumen's mark, as Raycast shows its own.
    Logo,
}

/// What the footer shows before its status (F15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FooterIcon {
    #[default]
    None,
    Logo,
}

/// Whether the footer says something the references leave out (F15): how
/// many results there are, and that Escape dismisses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shown {
    #[default]
    Shown,
    Hidden,
}

impl Shown {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "shown" => Some(Shown::Shown),
            "hidden" => Some(Shown::Hidden),
            _ => None,
        }
    }
}

/// How a section's title is cased.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SectionCase {
    /// As written: *Commands*.
    #[default]
    Title,
    /// *COMMANDS*.
    Upper,
}

/// The launcher's structural choices, resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Variants {
    pub row_subtitle: RowSubtitle,
    pub selection: Selection,
    pub footer_hints: FooterHints,
    pub keycap: Keycap,
    pub section_case: SectionCase,
    pub search_icon: SearchIcon,
    pub footer_icon: FooterIcon,
    /// The result count, where the footer has nothing else to say.
    pub footer_count: Shown,
    /// The *Dismiss* or *Back* hint for Escape.
    pub footer_escape: Shown,
}

impl Variants {
    /// Set `name` to `value`. `None` when either is unknown, which is
    /// reported and ignored: a variant added later must not stop a style
    /// written for it from loading here.
    fn apply(&mut self, name: &str, value: &str) -> Option<()> {
        match name {
            "row.subtitle" => {
                self.row_subtitle = match value {
                    "stacked" => RowSubtitle::Stacked,
                    "inline" => RowSubtitle::Inline,
                    _ => return None,
                }
            }
            "selection" => {
                self.selection = match value {
                    "fill-border" => Selection::FillBorder,
                    "fill" => Selection::Fill,
                    _ => return None,
                }
            }
            "footer.hints" => {
                self.footer_hints = match value {
                    "text" => FooterHints::Text,
                    "keycaps" => FooterHints::Keycaps,
                    _ => return None,
                }
            }
            "keycap" => {
                self.keycap = match value {
                    "filled" => Keycap::Filled,
                    "outline" => Keycap::Outline,
                    _ => return None,
                }
            }
            "search.icon" => {
                self.search_icon = match value {
                    "none" => SearchIcon::None,
                    "search" => SearchIcon::Search,
                    "logo" => SearchIcon::Logo,
                    _ => return None,
                }
            }
            "footer.icon" => {
                self.footer_icon = match value {
                    "none" => FooterIcon::None,
                    "logo" => FooterIcon::Logo,
                    _ => return None,
                }
            }
            "footer.count" => self.footer_count = Shown::parse(value)?,
            "footer.escape" => self.footer_escape = Shown::parse(value)?,
            "section.case" => {
                self.section_case = match value {
                    "title" => SectionCase::Title,
                    "upper" => SectionCase::Upper,
                    _ => return None,
                }
            }
            _ => return None,
        }
        Some(())
    }
}

/// A style exactly as it appears on disk. Every entry is optional.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleFile {
    pub schema_version: u32,
    pub name: String,
    #[serde(default)]
    pub metrics: BTreeMap<String, f32>,
    #[serde(default)]
    pub variants: BTreeMap<String, String>,
}

/// A style, resolved: what the renderer reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub name: String,
    pub metrics: Metrics,
    pub variants: Variants,
}

impl Style {
    /// The base style: the launcher as it was before styles existed.
    pub fn base() -> Self {
        Self {
            name: "Lumen".into(),
            metrics: Metrics::BASE,
            variants: Variants::default(),
        }
    }

    /// Resolve a style file against the base.
    ///
    /// Nothing here fails. An unknown token or variant is ignored, a value out
    /// of range is clamped, a weight the typeface does not have is snapped to
    /// the nearest one it does, and a broken relation sends its tokens back to
    /// the base -- each with a warning naming the style, so its author can
    /// find out why it does not look as written.
    pub fn resolve(file: &StyleFile) -> Self {
        let mut style = Self::base();
        style.name = file.name.clone();

        for (name, value) in &file.variants {
            if style.variants.apply(name, value).is_none() {
                log::warn!(
                    "style {:?} sets unknown variant {name:?} = {value:?}; ignoring it",
                    file.name
                );
            }
        }

        for (name, &value) in &file.metrics {
            let Some((slot, min, max)) = style.metrics.slot(name) else {
                log::warn!(
                    "style {:?} sets unknown token {name:?}; ignoring it",
                    file.name
                );
                continue;
            };
            let mut resolved = if value.is_finite() {
                value.clamp(min, max)
            } else {
                *slot
            };
            if name.starts_with("weight.") {
                resolved = nearest_weight(resolved);
            }
            if resolved != value {
                log::warn!(
                    "style {:?} sets {name:?} to {value}, outside what it can be; using {resolved}",
                    file.name
                );
            }
            *slot = resolved;
        }

        // Each pass reverts at least one token to the base, and the base
        // breaks nothing, so this ends; the bound is only a guard.
        for _ in 0..TOKEN_COUNT {
            let Some((what, tokens)) = style.metrics.broken(&style.variants) else {
                break;
            };
            log::warn!(
                "style {:?} breaks \"{what}\"; using the base for {}",
                file.name,
                tokens.join(", ")
            );
            let mut base = Metrics::BASE;
            for token in tokens {
                let (from, ..) = base.slot(token).expect("relations name real tokens");
                let value = *from;
                let (to, ..) = style
                    .metrics
                    .slot(token)
                    .expect("relations name real tokens");
                *to = value;
            }
        }

        style
    }

    pub fn load(path: &Path) -> Result<Self> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("reading style {}", path.display()))?;
        let file: StyleFile = serde_json::from_str(&source)
            .with_context(|| format!("parsing style {}", path.display()))?;

        if file.schema_version > SCHEMA_VERSION {
            log::warn!(
                "style {:?} declares schema version {} but this build understands {SCHEMA_VERSION}; \
                 unknown tokens will fall back to the base style",
                file.name,
                file.schema_version
            );
        }

        Ok(Self::resolve(&file))
    }
}

impl Default for Style {
    fn default() -> Self {
        Self::base()
    }
}

/// How many tokens [`Metrics::slot`] knows; bounds the relation passes.
const TOKEN_COUNT: usize = 29;

fn nearest_weight(value: f32) -> f32 {
    WEIGHTS
        .into_iter()
        .min_by(|a, b| (a - value).abs().total_cmp(&(b - value).abs()))
        .expect("there are weights")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(metrics: &[(&str, f32)], variants: &[(&str, &str)]) -> StyleFile {
        StyleFile {
            schema_version: SCHEMA_VERSION,
            name: "Test".into(),
            metrics: metrics.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            variants: variants
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    /// What the compile-time assertions guaranteed, still guaranteed: the
    /// base breaks no relation, in any variant, so reverting to it always
    /// ends the resolution.
    #[test]
    fn the_base_breaks_nothing_in_any_variant() {
        for row_subtitle in [RowSubtitle::Stacked, RowSubtitle::Inline] {
            let variants = Variants {
                row_subtitle,
                ..Variants::default()
            };
            assert_eq!(Metrics::BASE.broken(&variants), None);
        }
    }

    /// I-12 for styles: a style that sets nothing is the launcher as it was.
    #[test]
    fn an_empty_style_is_the_base() {
        let style = Style::resolve(&file(&[], &[]));
        assert_eq!(style.metrics, Metrics::BASE);
        assert_eq!(style.variants, Variants::default());
    }

    #[test]
    fn every_token_in_the_table_is_counted() {
        let names = [
            "window.width",
            "list.height",
            "window.radius",
            "search.height",
            "gutter",
            "list.padding",
            "row.height",
            "row.radius",
            "row.inset",
            "icon.size",
            "icon.gap",
            "subtitle.gap",
            "section.height",
            "footer.height",
            "footer.icon-size",
            "keycap.height",
            "keycap.padding",
            "keycap.radius",
            "keycap.gap",
            "text.query",
            "text.title",
            "text.subtitle",
            "text.accessory",
            "text.section",
            "text.footer",
            "text.card",
            "weight.query",
            "weight.title",
            "weight.section",
        ];
        assert_eq!(names.len(), TOKEN_COUNT);
        let mut metrics = Metrics::BASE;
        for name in names {
            let (value, min, max) = metrics.slot(name).expect(name);
            // The base sits inside every range it sets.
            assert!((min..=max).contains(value), "{name} base outside its range");
        }
    }

    #[test]
    fn sets_tokens_and_variants() {
        let style = Style::resolve(&file(
            &[("row.height", 48.0), ("text.query", 20.0)],
            &[("row.subtitle", "inline"), ("footer.hints", "keycaps")],
        ));
        assert_eq!(style.metrics.row_height, 48.0);
        assert_eq!(style.metrics.text_query, 20.0);
        assert_eq!(style.variants.row_subtitle, RowSubtitle::Inline);
        assert_eq!(style.variants.footer_hints, FooterHints::Keycaps);
        assert_eq!(style.variants.selection, Selection::FillBorder);
    }

    #[test]
    fn clamps_what_is_out_of_range() {
        let style = Style::resolve(&file(&[("row.height", 400.0), ("gutter", -3.0)], &[]));
        assert_eq!(style.metrics.row_height, 72.0);
        assert_eq!(style.metrics.gutter, 8.0);
    }

    #[test]
    fn snaps_weights_to_the_bundled_faces() {
        let style = Style::resolve(&file(
            &[("weight.title", 540.0), ("weight.query", 700.0)],
            &[],
        ));
        assert_eq!(style.metrics.weight_title, 500.0);
        assert_eq!(style.metrics.weight_query, 600.0);
    }

    /// A row too short for its own corner: both go back to the base, and the
    /// rest of the style stands.
    #[test]
    fn a_broken_relation_reverts_its_tokens_only() {
        let style = Style::resolve(&file(
            &[
                ("row.height", 30.0),
                ("row.radius", 18.0),
                ("text.query", 20.0),
            ],
            &[],
        ));
        assert_eq!(style.metrics.row_radius, Metrics::BASE.row_radius);
        assert_eq!(style.metrics.row_height, Metrics::BASE.row_height);
        assert_eq!(style.metrics.text_query, 20.0);
    }

    /// Stacked lines need more height than one line: a short row is fine
    /// inline and reverted stacked.
    #[test]
    fn row_text_is_checked_against_the_variant() {
        let short = [("row.height", 32.0)];
        let inline = Style::resolve(&file(&short, &[("row.subtitle", "inline")]));
        assert_eq!(inline.metrics.row_height, 32.0);
        let stacked = Style::resolve(&file(&short, &[]));
        assert_eq!(stacked.metrics.row_height, Metrics::BASE.row_height);
    }

    #[test]
    fn unknown_names_are_ignored_not_fatal() {
        let style = Style::resolve(&file(
            &[("token.from.later", 3.0), ("row.height", 50.0)],
            &[("variant.from.later", "x"), ("selection", "glow")],
        ));
        assert_eq!(style.metrics.row_height, 50.0);
        // A value this build does not know leaves the variant at the base.
        assert_eq!(style.variants.selection, Selection::FillBorder);
    }

    #[test]
    fn icon_resolution_follows_the_icon_and_never_drops() {
        assert_eq!(Metrics::BASE.icon_resolution(), layout::ICON_RESOLUTION);
        let large = Metrics {
            icon_size: 40.0,
            ..Metrics::BASE
        };
        assert_eq!(large.icon_resolution(), 140);
    }

    /// The style a fresh install selects is the base, so installing F15
    /// changes nothing anybody can see.
    #[test]
    fn the_shipped_lumen_style_is_the_base() {
        let source = include_str!("../../../assets/styles/lumen.json");
        let file: StyleFile = serde_json::from_str(source).unwrap();
        let style = Style::resolve(&file);
        assert_eq!(style.metrics, Metrics::BASE);
        assert_eq!(style.variants, Variants::default());
    }

    /// A preset is drawn as written: no token clamped, none reverted by a
    /// relation. A preset that fell back to the base would look like neither.
    #[test]
    fn the_presets_resolve_exactly_as_written() {
        for source in [
            include_str!("../../../assets/styles/carbon.json"),
            include_str!("../../../assets/styles/graphite.json"),
            include_str!("../../../assets/styles/ink.json"),
        ] {
            let file: StyleFile = serde_json::from_str(source).unwrap();
            let mut style = Style::resolve(&file);
            for (name, value) in &file.metrics {
                let (resolved, ..) = style.metrics.slot(name).expect(name);
                assert_eq!(*resolved, *value, "{} sets {name}", file.name);
            }
            let mut variants = Variants::default();
            for (name, value) in &file.variants {
                assert!(variants.apply(name, value).is_some(), "{name} = {value}");
            }
            assert_eq!(style.variants, variants, "{}", file.name);
        }
    }

    #[test]
    fn round_trips_through_json() {
        let json = r#"{
            "schema_version": 1,
            "name": "Raycast",
            "metrics": { "row.height": 48, "text.query": 20 },
            "variants": { "selection": "fill" }
        }"#;
        let file: StyleFile = serde_json::from_str(json).unwrap();
        let style = Style::resolve(&file);
        assert_eq!(style.name, "Raycast");
        assert_eq!(style.metrics.row_height, 48.0);
        assert_eq!(style.variants.selection, Selection::Fill);
    }
}
