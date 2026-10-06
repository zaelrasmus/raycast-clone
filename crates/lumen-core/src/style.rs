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
//! in views, and where the action panel's popover stands, how wide it is and
//! how many rows it shows (7.47). The popover's rows, the forms, the overlay
//! and the settings window keep the constants (7.41).

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
/// Logical pixels, except the weights and the tracking, which is in em. Read by
/// the renderer as it draws, so a field here is a token a style may set; the
/// table in [`Metrics::slot`] is what makes it settable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub window_width: f32,
    /// How tall the list may grow before it scrolls; it ends on whole rows.
    pub list_height: f32,
    pub window_radius: f32,
    pub search_height: f32,
    pub gutter: f32,
    pub list_padding: f32,
    /// The space under a list's last row, when a style sets it; unset, it
    /// is decided by [`Metrics::list_padding_bottom`].
    pub list_padding_bottom: Option<f32>,
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
    /// Extra space after every glyph, in em: a multiple of the type size, so
    /// one number holds at every size. Zero adds none.
    pub text_tracking: f32,
    pub weight_query: f32,
    pub weight_title: f32,
    pub weight_section: f32,
    /// The weight of the card's two values.
    pub weight_card: f32,
    /// The thickness of the line under the search field and over the footer.
    pub rule_width: f32,
    /// How many rows the calculator's card takes. A float so it sits in the
    /// table with the rest; read as whole rows through [`Metrics::card_rows`].
    pub card_rows: f32,
    /// Between each of the card's values and its label.
    pub card_label_gap: f32,
    /// The action panel's popover, before the window narrows it: see
    /// [`Metrics::actions_width`].
    pub actions_width: f32,
    /// How far the popover stands in from the edges its anchor names, across
    /// and down. Always inward: from a right edge it moves left, from a bottom
    /// edge up. On an axis the anchor centres, it moves the popover right or
    /// down from the centre. The window keeps this much clear on both sides
    /// of the popover, whatever the anchor (`Config::panel_height`).
    pub actions_offset_x: f32,
    pub actions_offset_y: f32,
    /// How many of the popover's rows are shown before the rest scroll. A
    /// float so it sits in the table; read as whole rows through
    /// [`Metrics::actions_rows`]. The popover's height is computed from it,
    /// never set (I-10).
    pub actions_max_rows: f32,
    /// A grid's square cell, the space between cells and a cell's corner
    /// (F18). `grid_cell` is the minimum cell side; cells stretch to fill
    /// the row, through `layout::view::grid_cell_side`. How many columns
    /// there are is not a token: as many as fit, through
    /// `layout::view::grid_columns`, since structure is not themeable
    /// (I-11). The glyph is a fixed share of the stretched cell.
    pub grid_cell: f32,
    pub grid_gap: f32,
    pub grid_radius: f32,
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
        list_padding_bottom: None,
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
        text_tracking: 0.0,
        weight_query: 400.0,
        weight_title: 400.0,
        weight_section: 400.0,
        weight_card: 400.0,
        rule_width: 1.0,
        card_rows: layout::CARD_ROWS as f32,
        card_label_gap: layout::CARD_LABEL_GAP,
        actions_width: layout::action_panel::WIDTH,
        actions_offset_x: layout::action_panel::INSET,
        actions_offset_y: layout::action_panel::INSET,
        actions_max_rows: layout::action_panel::MAX_ROWS as f32,
        grid_cell: layout::view::GRID_CELL,
        grid_gap: layout::view::GRID_GAP,
        grid_radius: layout::view::GRID_RADIUS,
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
            "text.tracking" => (&mut self.text_tracking, -0.05, 0.1),
            "weight.query" => (&mut self.weight_query, 400.0, 600.0),
            "weight.title" => (&mut self.weight_title, 400.0, 600.0),
            "weight.section" => (&mut self.weight_section, 400.0, 600.0),
            "weight.card" => (&mut self.weight_card, 400.0, 600.0),
            "rule.width" => (&mut self.rule_width, 1.0, 2.0),
            "card.label-gap" => (&mut self.card_label_gap, 2.0, 24.0),
            "card.rows" => (&mut self.card_rows, 2.0, 4.0),
            "actions.width" => (&mut self.actions_width, 280.0, 720.0),
            "actions.offset-x" => (&mut self.actions_offset_x, 0.0, 64.0),
            "actions.offset-y" => (&mut self.actions_offset_y, 0.0, 64.0),
            "actions.max-rows" => (&mut self.actions_max_rows, 3.0, 12.0),
            // Down to where a glyph, at its share, still reads as a picture
            // (31 px); up to the 128 px a Fluent image would be drawn from,
            // where the narrowest window still fits three columns. Every cell
            // is shorter than the shortest list, so none needs a relation.
            "grid.cell" => (&mut self.grid_cell, 56.0, 128.0),
            // Zero tiles the cells; past three times the base, the gaps read
            // as columns of their own.
            "grid.gap" => (&mut self.grid_gap, 0.0, 24.0),
            // Up to a circle in a cell near the smallest; the relation below
            // keeps it within the cell actually set.
            "grid.radius" => (&mut self.grid_radius, 0.0, 32.0),
            _ => return None,
        })
    }

    /// The tokens a style may leave unset, with where they land and their
    /// range. Unset, each is derived from the others, so no relation names
    /// one and the base sets none (I-12).
    fn optional_slot(&mut self, name: &str) -> Option<(&mut Option<f32>, f32, f32)> {
        Some(match name {
            "list.padding-bottom" => (&mut self.list_padding_bottom, 0.0, 16.0),
            _ => return None,
        })
    }

    /// The space under a list's last row: `list.padding-bottom` when the
    /// style sets it. Otherwise at least the row's side inset, so the
    /// selected row sits in the same frame on every side, and never less
    /// than the list padding, so a style with a roomier list keeps it. The
    /// space above the first row is the list padding alone.
    pub fn list_padding_bottom(&self) -> f32 {
        self.list_padding_bottom
            .unwrap_or(self.list_padding.max(self.row_inset))
    }

    /// A list's padding above and below its rows together: what every height
    /// adds to the rows, so the window is exactly what is drawn (I-10).
    pub fn list_padding_vertical(&self) -> f32 {
        self.list_padding + self.list_padding_bottom()
    }

    /// The rows the card takes, whole.
    pub fn card_rows(&self) -> usize {
        self.card_rows.round() as usize
    }

    /// The action panel's rows shown before it scrolls, whole.
    pub fn actions_rows(&self) -> usize {
        self.actions_max_rows.round() as usize
    }

    /// The popover's width in a window this wide: the style's, but never so
    /// wide that it leaves less than its horizontal offset either side. The
    /// window's width can come from `config.json`, so this is not a relation
    /// checked when the style loads.
    pub fn actions_width(&self, window_width: f32) -> f32 {
        self.actions_width
            .min(window_width - self.actions_offset_x * 2.0)
            .max(0.0)
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
        // A chip's padding is part of the label it holds.
        let chip = match variants.card_labels {
            CardLabels::Plain => 0.0,
            CardLabels::Chips => layout::CARD_CHIP_PADDING_Y * 2.0,
        };
        let checks: [(bool, &'static str, &'static [&'static str]); 13] = [
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
                line(self.text_card) + self.card_label_gap + line(self.text_footer) + chip
                    <= self.row_height * self.card_rows().max(1) as f32,
                "the card's value and label fit in its rows",
                &[
                    "text.card",
                    "text.footer",
                    "row.height",
                    "card.rows",
                    "card.label-gap",
                ],
            ),
            (
                self.grid_radius * 2.0 <= self.grid_cell,
                "a cell's corner fits in the cell",
                &["grid.radius", "grid.cell"],
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

/// Whether the calculator's card has a section header over it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CardHeader {
    #[default]
    Hidden,
    /// *Calculator*, drawn as a section's header is.
    Shown,
}

/// How the card names its two values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CardLabels {
    /// Muted text.
    #[default]
    Plain,
    /// Primary text on a small rounded fill.
    Chips,
}

/// Whether a line runs between the card's two values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CardDivider {
    #[default]
    None,
    /// Down the middle, interrupted where the arrow sits.
    Line,
}

/// How the operators in the card's expression are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CardOperators {
    #[default]
    Plain,
    /// Muted, so the operands read first.
    Dimmed,
}

/// Where the action panel's popover stands in the region between the search
/// field and the footer (7.47).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActionsAnchor {
    /// Raycast's: over the list's bottom-right, just above the footer.
    #[default]
    BottomRight,
    BottomLeft,
    BottomCenter,
    /// Just under the search field.
    TopRight,
    TopLeft,
    TopCenter,
    /// In the middle of the region.
    Center,
}

impl ActionsAnchor {
    /// The popover's top-left corner within a region of `region` (width,
    /// height), for a popover of `size` standing `offset` (across, down) in
    /// from the edges this anchor names.
    ///
    /// The offset always moves the popover inward from an anchored edge --
    /// left from the right edge, up from the bottom -- and right or down on an
    /// axis this anchor centres. So with an offset of `o`, the popover fits in
    /// any region at least its size plus `2 * o` on that axis, whichever the
    /// anchor: the room `Config::panel_height` gives it.
    pub fn origin(self, region: [f32; 2], size: [f32; 2], offset: [f32; 2]) -> [f32; 2] {
        use ActionsAnchor::*;

        let place = |edge: Edge, axis: usize| match edge {
            Edge::Start => offset[axis],
            Edge::Middle => (region[axis] - size[axis]) / 2.0 + offset[axis],
            Edge::End => region[axis] - size[axis] - offset[axis],
        };
        let (across, down) = match self {
            BottomRight => (Edge::End, Edge::End),
            BottomLeft => (Edge::Start, Edge::End),
            BottomCenter => (Edge::Middle, Edge::End),
            TopRight => (Edge::End, Edge::Start),
            TopLeft => (Edge::Start, Edge::Start),
            TopCenter => (Edge::Middle, Edge::Start),
            Center => (Edge::Middle, Edge::Middle),
        };
        [place(across, 0), place(down, 1)]
    }
}

/// Where on one axis an anchor holds the popover.
#[derive(Clone, Copy)]
enum Edge {
    Start,
    Middle,
    End,
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
    pub card_header: CardHeader,
    pub card_labels: CardLabels,
    pub card_divider: CardDivider,
    pub card_operators: CardOperators,
    pub actions_anchor: ActionsAnchor,
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
            "card.header" => {
                self.card_header = match value {
                    "hidden" => CardHeader::Hidden,
                    "shown" => CardHeader::Shown,
                    _ => return None,
                }
            }
            "card.labels" => {
                self.card_labels = match value {
                    "plain" => CardLabels::Plain,
                    "chips" => CardLabels::Chips,
                    _ => return None,
                }
            }
            "card.divider" => {
                self.card_divider = match value {
                    "none" => CardDivider::None,
                    "line" => CardDivider::Line,
                    _ => return None,
                }
            }
            "card.operators" => {
                self.card_operators = match value {
                    "plain" => CardOperators::Plain,
                    "dimmed" => CardOperators::Dimmed,
                    _ => return None,
                }
            }
            "actions.anchor" => {
                self.actions_anchor = match value {
                    "bottom-right" => ActionsAnchor::BottomRight,
                    "bottom-left" => ActionsAnchor::BottomLeft,
                    "bottom-center" => ActionsAnchor::BottomCenter,
                    "top-right" => ActionsAnchor::TopRight,
                    "top-left" => ActionsAnchor::TopLeft,
                    "top-center" => ActionsAnchor::TopCenter,
                    "center" => ActionsAnchor::Center,
                    _ => return None,
                }
            }
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
            if let Some((slot, min, max)) = style.metrics.optional_slot(name) {
                if !value.is_finite() {
                    log::warn!(
                        "style {:?} sets {name:?} to {value}, outside what it can be; leaving it unset",
                        file.name
                    );
                    continue;
                }
                let resolved = value.clamp(min, max);
                if resolved != value {
                    log::warn!(
                        "style {:?} sets {name:?} to {value}, outside what it can be; using {resolved}",
                        file.name
                    );
                }
                *slot = Some(resolved);
                continue;
            }
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
const TOKEN_COUNT: usize = 41;

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
        let chips = Variants {
            card_labels: CardLabels::Chips,
            ..Variants::default()
        };
        assert_eq!(Metrics::BASE.broken(&chips), None);
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
            "text.tracking",
            "weight.query",
            "weight.title",
            "weight.section",
            "weight.card",
            "rule.width",
            "card.rows",
            "card.label-gap",
            "actions.width",
            "actions.offset-x",
            "actions.offset-y",
            "actions.max-rows",
            "grid.cell",
            "grid.gap",
            "grid.radius",
        ];
        assert_eq!(names.len(), TOKEN_COUNT);
        let mut metrics = Metrics::BASE;
        for name in names {
            let (value, min, max) = metrics.slot(name).expect(name);
            // The base sits inside every range it sets.
            assert!((min..=max).contains(value), "{name} base outside its range");
        }
        // The optional token is known too, and the base leaves it unset.
        let name = "list.padding-bottom";
        assert!(metrics.slot(name).is_none(), "{name} is in both tables");
        let (value, ..) = metrics.optional_slot(name).expect(name);
        assert_eq!(*value, None, "{name} set in the base");
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

    /// Under the last row there is at least the row's side inset, so the
    /// selected row sits in the same frame on every side; a larger list
    /// padding is kept. The top stays the list padding.
    #[test]
    fn the_list_ends_at_least_a_row_inset_below_its_last_row() {
        let carbon = Style::resolve(&file(&[("list.padding", 2.0), ("row.inset", 7.0)], &[]));
        assert_eq!(carbon.metrics.list_padding_bottom(), 7.0);
        assert_eq!(carbon.metrics.list_padding_vertical(), 2.0 + 7.0);

        let roomy = Style::resolve(&file(&[("list.padding", 10.0), ("row.inset", 7.0)], &[]));
        assert_eq!(roomy.metrics.list_padding_bottom(), 10.0);
        assert_eq!(roomy.metrics.list_padding_vertical(), 20.0);
    }

    /// `list.padding-bottom` is optional: unset, the end of the list is as
    /// above; set, it is the space under the last row, whatever the padding
    /// and the inset are.
    #[test]
    fn list_padding_bottom_is_optional_and_wins_when_set() {
        assert_eq!(Metrics::BASE.list_padding_bottom, None);
        let unset = Style::resolve(&file(&[("list.padding", 2.0), ("row.inset", 7.0)], &[]));
        assert_eq!(unset.metrics.list_padding_bottom, None);
        assert_eq!(unset.metrics.list_padding_bottom(), 7.0);

        let set = |bottom: f32| {
            Style::resolve(&file(
                &[
                    ("list.padding", 2.0),
                    ("row.inset", 7.0),
                    ("list.padding-bottom", bottom),
                ],
                &[],
            ))
            .metrics
        };
        assert_eq!(set(12.0).list_padding_bottom(), 12.0);
        assert_eq!(set(12.0).list_padding_vertical(), 2.0 + 12.0);
        assert_eq!(set(0.0).list_padding_bottom(), 0.0);
        // Out of range is clamped like every other token.
        assert_eq!(set(40.0).list_padding_bottom, Some(16.0));
        assert_eq!(set(-3.0).list_padding_bottom, Some(0.0));
        // Not a number leaves it unset.
        assert_eq!(set(f32::NAN).list_padding_bottom, None);
    }

    #[test]
    fn list_padding_bottom_parses_from_json() {
        let json = r#"{
            "schema_version": 1,
            "name": "Bottom",
            "metrics": { "list.padding-bottom": 12 }
        }"#;
        let file: StyleFile = serde_json::from_str(json).unwrap();
        let style = Style::resolve(&file);
        assert_eq!(style.metrics.list_padding_bottom, Some(12.0));
        assert_eq!(style.metrics.list_padding_bottom(), 12.0);
    }

    /// Tracking is a token like the others: base zero, set from a style, and
    /// clamped to its range, so a style written for a later Lumen still loads.
    #[test]
    fn tracking_is_zero_in_the_base_and_set_by_a_style() {
        assert_eq!(Metrics::BASE.text_tracking, 0.0);
        assert_eq!(Style::resolve(&file(&[], &[])).metrics.text_tracking, 0.0);
        let style = Style::resolve(&file(&[("text.tracking", 0.02)], &[]));
        assert_eq!(style.metrics.text_tracking, 0.02);
        let tighter = Style::resolve(&file(&[("text.tracking", -0.03)], &[]));
        assert_eq!(tighter.metrics.text_tracking, -0.03);
    }

    #[test]
    fn tracking_is_clamped_to_its_range() {
        let wide = Style::resolve(&file(&[("text.tracking", 2.0)], &[]));
        assert_eq!(wide.metrics.text_tracking, 0.1);
        let tight = Style::resolve(&file(&[("text.tracking", -1.0)], &[]));
        assert_eq!(tight.metrics.text_tracking, -0.05);
        // What is not a number leaves the base.
        let nan = Style::resolve(&file(&[("text.tracking", f32::NAN)], &[]));
        assert_eq!(nan.metrics.text_tracking, 0.0);
    }

    /// I-12: a style file written before the token existed loads, and draws
    /// with no tracking.
    #[test]
    fn a_style_without_tracking_still_loads() {
        let json = r#"{
            "schema_version": 1,
            "name": "Old",
            "metrics": { "row.height": 48 }
        }"#;
        let file: StyleFile = serde_json::from_str(json).unwrap();
        let style = Style::resolve(&file);
        assert_eq!(style.metrics.row_height, 48.0);
        assert_eq!(style.metrics.text_tracking, 0.0);
    }

    /// The card's tokens are the card's as it was: a style that sets none of
    /// them draws what F14 drew, and the base is inside every range (I-12).
    #[test]
    fn the_card_and_rule_tokens_default_to_what_was_drawn() {
        let metrics = Style::resolve(&file(&[], &[])).metrics;
        assert_eq!(metrics.rule_width, 1.0);
        assert_eq!(metrics.weight_card, 400.0);
        assert_eq!(metrics.card_rows, layout::CARD_ROWS as f32);
        assert_eq!(metrics.card_rows(), layout::CARD_ROWS);
        assert_eq!(metrics.card_label_gap, layout::CARD_LABEL_GAP);
    }

    #[test]
    fn the_card_and_rule_tokens_are_set_and_clamped() {
        let set = Style::resolve(&file(
            &[
                ("rule.width", 2.0),
                ("weight.card", 600.0),
                ("card.rows", 3.0),
                ("card.label-gap", 20.0),
            ],
            &[],
        ));
        assert_eq!(set.metrics.card_label_gap, 20.0);
        let wide = Style::resolve(&file(&[("card.label-gap", 99.0)], &[]));
        assert_eq!(wide.metrics.card_label_gap, 24.0);
        assert_eq!(set.metrics.rule_width, 2.0);
        assert_eq!(set.metrics.weight_card, 600.0);
        assert_eq!(set.metrics.card_rows(), 3);

        let high = Style::resolve(&file(&[("rule.width", 9.0), ("card.rows", 9.0)], &[]));
        assert_eq!(high.metrics.rule_width, 2.0);
        assert_eq!(high.metrics.card_rows(), 4);
        let low = Style::resolve(&file(&[("rule.width", 0.0), ("card.rows", 1.0)], &[]));
        assert_eq!(low.metrics.rule_width, 1.0);
        assert_eq!(low.metrics.card_rows(), 2);

        // The weight snaps to a face the typeface has, as the others do.
        let snapped = Style::resolve(&file(&[("weight.card", 540.0)], &[]));
        assert_eq!(snapped.metrics.weight_card, 500.0);
    }

    /// The grid's tokens are Raycast's, measured at 750 wide: a style that
    /// sets none of them draws its cells (F18).
    #[test]
    fn the_grid_tokens_default_to_raycasts() {
        let metrics = Style::resolve(&file(&[], &[])).metrics;
        assert_eq!(metrics.grid_cell, 82.0);
        assert_eq!(metrics.grid_gap, 8.0);
        assert_eq!(metrics.grid_radius, 8.0);
        assert_eq!(metrics.grid_cell, layout::view::GRID_CELL);
        assert_eq!(metrics.grid_gap, layout::view::GRID_GAP);
        assert_eq!(metrics.grid_radius, layout::view::GRID_RADIUS);
    }

    #[test]
    fn the_grid_tokens_are_set_and_clamped() {
        let set = Style::resolve(&file(
            &[
                ("grid.cell", 96.0),
                ("grid.gap", 12.0),
                ("grid.radius", 12.0),
            ],
            &[],
        ));
        assert_eq!(set.metrics.grid_cell, 96.0);
        assert_eq!(set.metrics.grid_gap, 12.0);
        assert_eq!(set.metrics.grid_radius, 12.0);

        let high = Style::resolve(&file(
            &[
                ("grid.cell", 400.0),
                ("grid.gap", 99.0),
                ("grid.radius", 99.0),
            ],
            &[],
        ));
        assert_eq!(high.metrics.grid_cell, 128.0);
        assert_eq!(high.metrics.grid_gap, 24.0);
        assert_eq!(high.metrics.grid_radius, 32.0);
        let low = Style::resolve(&file(
            &[
                ("grid.cell", 10.0),
                ("grid.gap", -4.0),
                ("grid.radius", -1.0),
            ],
            &[],
        ));
        assert_eq!(low.metrics.grid_cell, 56.0);
        assert_eq!(low.metrics.grid_gap, 0.0);
        assert_eq!(low.metrics.grid_radius, 0.0);
    }

    /// A corner wider than half the cell: the cell and its corner go back to
    /// the base, and the gap the style set stands.
    #[test]
    fn a_cells_corner_must_fit_the_cell() {
        let style = Style::resolve(&file(
            &[
                ("grid.cell", 56.0),
                ("grid.radius", 30.0),
                ("grid.gap", 4.0),
            ],
            &[],
        ));
        assert_eq!(style.metrics.grid_cell, Metrics::BASE.grid_cell);
        assert_eq!(style.metrics.grid_radius, Metrics::BASE.grid_radius);
        assert_eq!(style.metrics.grid_gap, 4.0);

        // Half the cell exactly is a circle, and fits.
        let round = Style::resolve(&file(&[("grid.cell", 60.0), ("grid.radius", 30.0)], &[]));
        assert_eq!(round.metrics.grid_cell, 60.0);
        assert_eq!(round.metrics.grid_radius, 30.0);
    }

    /// Rows are whole: a fraction rounds, it does not draw half a row.
    #[test]
    fn card_rows_round_to_whole_rows() {
        let style = Style::resolve(&file(&[("card.rows", 2.6)], &[]));
        assert_eq!(style.metrics.card_rows(), 3);
    }

    #[test]
    fn the_card_variants_are_the_base_until_set() {
        let variants = Style::resolve(&file(&[], &[])).variants;
        assert_eq!(variants.card_header, CardHeader::Hidden);
        assert_eq!(variants.card_labels, CardLabels::Plain);
        assert_eq!(variants.card_divider, CardDivider::None);
        assert_eq!(variants.card_operators, CardOperators::Plain);
    }

    #[test]
    fn the_card_variants_parse_and_reject_what_they_do_not_know() {
        let set = Style::resolve(&file(
            &[],
            &[
                ("card.header", "shown"),
                ("card.labels", "chips"),
                ("card.divider", "line"),
                ("card.operators", "dimmed"),
            ],
        ));
        assert_eq!(set.variants.card_header, CardHeader::Shown);
        assert_eq!(set.variants.card_labels, CardLabels::Chips);
        assert_eq!(set.variants.card_divider, CardDivider::Line);
        assert_eq!(set.variants.card_operators, CardOperators::Dimmed);

        let mut variants = Variants::default();
        for name in [
            "card.header",
            "card.labels",
            "card.divider",
            "card.operators",
        ] {
            assert!(variants.apply(name, "glow").is_none(), "{name}");
        }
        assert_eq!(variants, Variants::default());
    }

    /// The card's value and label are checked against its rows, and a chip's
    /// padding is part of the label.
    #[test]
    fn the_card_must_fit_its_rows() {
        let too_large = [("text.card", 48.0), ("row.height", 28.0)];
        let style = Style::resolve(&file(&too_large, &[]));
        assert_eq!(style.metrics.row_height, Metrics::BASE.row_height);
        assert_eq!(style.metrics.text_card, Metrics::BASE.text_card);
        // The same card in more rows fits.
        let roomy = Style::resolve(&file(&[("text.card", 48.0), ("card.rows", 4.0)], &[]));
        assert_eq!(roomy.metrics.text_card, 48.0);
        assert_eq!(roomy.metrics.card_rows(), 4);
    }

    /// I-12: a style file written before these existed loads unchanged.
    #[test]
    fn a_style_without_the_card_tokens_still_loads() {
        let json = r#"{
            "schema_version": 1,
            "name": "Old",
            "metrics": { "row.height": 48 },
            "variants": { "selection": "fill" }
        }"#;
        let file: StyleFile = serde_json::from_str(json).unwrap();
        let style = Style::resolve(&file);
        assert_eq!(style.metrics.row_height, 48.0);
        assert_eq!(style.metrics.rule_width, 1.0);
        assert_eq!(style.metrics.card_rows(), layout::CARD_ROWS);
        assert_eq!(style.variants.card_header, CardHeader::Hidden);
    }

    /// I-12: a style written before the action panel was opened to it loads,
    /// and places the popover where it always was.
    #[test]
    fn a_style_without_the_action_panel_tokens_draws_it_as_it_was() {
        let json = r#"{
            "schema_version": 1,
            "name": "Old",
            "metrics": { "row.height": 48 }
        }"#;
        let file: StyleFile = serde_json::from_str(json).unwrap();
        let style = Style::resolve(&file);
        assert_eq!(style.metrics.actions_width, layout::action_panel::WIDTH);
        assert_eq!(style.metrics.actions_offset_x, layout::action_panel::INSET);
        assert_eq!(style.metrics.actions_offset_y, layout::action_panel::INSET);
        assert_eq!(style.metrics.actions_rows(), layout::action_panel::MAX_ROWS);
        assert_eq!(style.variants.actions_anchor, ActionsAnchor::BottomRight);
    }

    #[test]
    fn the_action_panel_tokens_are_set_and_clamped() {
        let set = Style::resolve(&file(
            &[
                ("actions.width", 520.0),
                ("actions.offset-x", 16.0),
                ("actions.offset-y", 0.0),
                ("actions.max-rows", 10.0),
            ],
            &[("actions.anchor", "top-left")],
        ));
        assert_eq!(set.metrics.actions_width, 520.0);
        assert_eq!(set.metrics.actions_offset_x, 16.0);
        assert_eq!(set.metrics.actions_offset_y, 0.0);
        assert_eq!(set.metrics.actions_rows(), 10);
        assert_eq!(set.variants.actions_anchor, ActionsAnchor::TopLeft);

        let high = Style::resolve(&file(
            &[
                ("actions.width", 2000.0),
                ("actions.offset-x", 500.0),
                ("actions.offset-y", 99.0),
                ("actions.max-rows", 40.0),
            ],
            &[],
        ));
        assert_eq!(high.metrics.actions_width, 720.0);
        assert_eq!(high.metrics.actions_offset_x, 64.0);
        assert_eq!(high.metrics.actions_offset_y, 64.0);
        assert_eq!(high.metrics.actions_rows(), 12);

        let low = Style::resolve(&file(
            &[
                ("actions.width", 10.0),
                ("actions.offset-x", -8.0),
                ("actions.offset-y", -1.0),
                ("actions.max-rows", 1.0),
            ],
            &[],
        ));
        assert_eq!(low.metrics.actions_width, 280.0);
        assert_eq!(low.metrics.actions_offset_x, 0.0);
        assert_eq!(low.metrics.actions_offset_y, 0.0);
        assert_eq!(low.metrics.actions_rows(), 3);

        // Rows are whole.
        let half = Style::resolve(&file(&[("actions.max-rows", 8.6)], &[]));
        assert_eq!(half.metrics.actions_rows(), 9);
    }

    #[test]
    fn the_anchor_parses_every_value_and_ignores_what_it_does_not_know() {
        for (value, anchor) in [
            ("bottom-right", ActionsAnchor::BottomRight),
            ("bottom-left", ActionsAnchor::BottomLeft),
            ("bottom-center", ActionsAnchor::BottomCenter),
            ("top-right", ActionsAnchor::TopRight),
            ("top-left", ActionsAnchor::TopLeft),
            ("top-center", ActionsAnchor::TopCenter),
            ("center", ActionsAnchor::Center),
        ] {
            let style = Style::resolve(&file(&[], &[("actions.anchor", value)]));
            assert_eq!(style.variants.actions_anchor, anchor, "{value}");
        }

        let unknown = Style::resolve(&file(&[], &[("actions.anchor", "middle-ish")]));
        assert_eq!(unknown.variants.actions_anchor, ActionsAnchor::BottomRight);
    }

    /// Where the popover's top-left corner lands in a 720 x 300 region, for a
    /// 440 x 200 popover 8 from its edges.
    #[test]
    fn the_popover_is_placed_by_its_anchor_and_offsets() {
        let region = [720.0, 300.0];
        let size = [440.0, 200.0];
        let offset = [8.0, 8.0];
        let at = |anchor: ActionsAnchor| anchor.origin(region, size, offset);

        assert_eq!(at(ActionsAnchor::BottomRight), [272.0, 92.0]);
        assert_eq!(at(ActionsAnchor::BottomLeft), [8.0, 92.0]);
        assert_eq!(at(ActionsAnchor::BottomCenter), [148.0, 92.0]);
        assert_eq!(at(ActionsAnchor::TopRight), [272.0, 8.0]);
        assert_eq!(at(ActionsAnchor::TopLeft), [8.0, 8.0]);
        assert_eq!(at(ActionsAnchor::TopCenter), [148.0, 8.0]);
        // On a centred axis the offset moves it right and down.
        assert_eq!(at(ActionsAnchor::Center), [148.0, 58.0]);
        assert_eq!(
            ActionsAnchor::Center.origin(region, size, [0.0, 0.0]),
            [140.0, 50.0]
        );
    }

    /// Within a region exactly as tall as the popover and twice its vertical
    /// offset -- what a short list grows the window to -- every anchor keeps
    /// it inside, with the offset clear of whichever edge it is anchored to.
    #[test]
    fn every_anchor_fits_the_popover_in_the_room_the_window_gives_it() {
        let size = [440.0, 200.0];
        let offset = [12.0, 10.0];
        let region = [720.0, size[1] + offset[1] * 2.0];
        for anchor in [
            ActionsAnchor::BottomRight,
            ActionsAnchor::BottomLeft,
            ActionsAnchor::BottomCenter,
            ActionsAnchor::TopRight,
            ActionsAnchor::TopLeft,
            ActionsAnchor::TopCenter,
            ActionsAnchor::Center,
        ] {
            let [x, y] = anchor.origin(region, size, offset);
            assert!(x >= 0.0 && x + size[0] <= region[0], "{anchor:?}");
            assert!(y >= 0.0 && y + size[1] <= region[1], "{anchor:?}");
        }
    }

    /// The width never leaves less than the horizontal offset either side.
    #[test]
    fn the_popover_is_never_wider_than_the_window_less_its_offsets() {
        let metrics = Metrics {
            actions_width: 720.0,
            actions_offset_x: 20.0,
            ..Metrics::BASE
        };
        assert_eq!(metrics.actions_width(600.0), 560.0);
        assert_eq!(metrics.actions_width(1000.0), 720.0);
        assert_eq!(Metrics::BASE.actions_width(720.0), 440.0);
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
