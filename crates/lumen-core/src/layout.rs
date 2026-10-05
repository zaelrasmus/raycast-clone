//! Geometry tokens.
//!
//! # Reconciling I-5 with I-11
//!
//! Two invariants meet here and look, at first, like they disagree. I-5 says no
//! colour, spacing, radius or type size may appear literally in UI code:
//! everything comes from the token file. I-11 says a theme may define only
//! colour and type family, and no geometric property is themeable.
//!
//! Both are satisfied by having *two* token sources with different authors.
//! Colour and type family come from the theme file, which anyone may write.
//! Geometry comes from this module, which only the project may change. What I-5
//! actually forbids is a literal `px(38.)` buried in a render function, where it
//! cannot be seen next to the value it has to line up with -- and that stays
//! forbidden either way.
//!
//! # Why these are constants and not configuration
//!
//! Row height and icon size are the calibration AD-13 exists to protect. They
//! are also what makes I-10 -- zero layout shift when results change -- possible
//! at all: every row is the same height whether or not it has a subtitle, and
//! the space for icons and accessories is reserved before it is known whether
//! there are any.

/// Height of one result row. Fixed, always (I-10).
pub const ROW_HEIGHT: f32 = 44.0;

/// The calculator's answer, drawn as a card above the list (F14): this many
/// rows tall, and in their place, so the window keeps its height when one
/// appears (7.40).
pub const CARD_ROWS: usize = 2;

/// Between a value in the card and the label beneath it.
pub const CARD_LABEL_GAP: f32 = 6.0;

/// A card's label drawn as a chip (`card.labels`): the room around the text,
/// and the corner of the fill.
pub const CARD_CHIP_PADDING_X: f32 = 8.0;
pub const CARD_CHIP_PADDING_Y: f32 = 2.0;
pub const CARD_CHIP_RADIUS: f32 = 4.0;

/// Above and below the arrow where the card's divider leaves a gap for it
/// (`card.divider`).
pub const CARD_ARROW_GAP: f32 = 8.0;

/// The card's divider: a hairline, not a rule. It does not follow
/// `rule.width`, which thickens the bars between the window's regions.
pub const CARD_DIVIDER_WIDTH: f32 = 1.0;

/// Height of the search field.
pub const SEARCH_HEIGHT: f32 = 56.0;

/// Width of the text caret in every field.
///
/// One pixel, not two: at the sizes Lumen sets text, a wider caret reads as a
/// character of its own.
pub const CARET_WIDTH: f32 = 1.0;

/// Height of the footer carrying the action hints.
pub const FOOTER_HEIGHT: f32 = 38.0;

/// Vertical padding above and below the list as a whole.
pub const LIST_PADDING: f32 = 6.0;

/// Horizontal inset applied to the search field, rows and footer alike, so that
/// text in all three lines up on a single left edge.
pub const GUTTER: f32 = 14.0;

/// The reserved square for a row's icon. Reserved whether or not the icon has
/// loaded yet, so a late icon does not shift the title (I-10).
pub const ICON_SIZE: f32 = 26.0;

/// Physical pixels requested from the platform's icon extractor.
///
/// Deliberately far above `ICON_SIZE`: a 26 pt icon is 52 physical pixels at
/// 200%, and asking for exactly what a 100% display needs is what makes icons
/// look soft on the machine the author actually uses. Section F1 names icons as
/// the risk that is always underestimated.
pub const ICON_RESOLUTION: u32 = 96;

/// Gap between an icon and the text that follows it.
pub const ICON_GAP: f32 = 12.0;

/// Corner radius of the window itself.
///
/// Windows 11 rounds the window with `DWMWCP_ROUND` at roughly this radius; the
/// value is matched here so the surface the application paints does not peek
/// out past the system's corner.
pub const WINDOW_RADIUS: f32 = 10.0;

/// Corner radius of a selected row.
pub const ROW_RADIUS: f32 = 7.0;

/// Inset of the selected row's highlight from the window edge.
pub const ROW_INSET: f32 = 7.0;

/// The action panel, opened with Ctrl+K: a popover floating over the list,
/// by default at its bottom-right just above the footer (7.45).
///
/// Its rows and padding are also the scope picker's, which is drawn the same
/// way but in the list's place. Where it stands, how wide it is and how many
/// rows it shows are the style's (`actions.*`, 7.47); the constants for those
/// here are the base they resolve from.
pub mod action_panel {
    use crate::item::ActionGroup;

    /// One action row.
    pub const ROW_HEIGHT: f32 = 30.0;

    /// Padding inside the panel.
    pub const PADDING: f32 = 6.0;

    pub const ROW_RADIUS: f32 = 5.0;

    /// The popover's width in the base style (`actions.width`). Fixed rather
    /// than the window's share, so a wider window shows the same popover and a
    /// narrower one is never covered by it.
    pub const WIDTH: f32 = 440.0;

    /// How far the popover stands from the window's right edge and from the
    /// footer in the base style (`actions.offset-x`, `actions.offset-y`); also
    /// the gap kept above it, so it never touches the search row.
    pub const INSET: f32 = 8.0;

    /// The popover's outline. Part of its height, which is border-box.
    pub const BORDER: f32 = 1.0;

    pub const RADIUS: f32 = 10.0;

    /// The line naming what the actions are for. Reserved even when the name
    /// is empty (I-10).
    pub const HEADER_HEIGHT: f32 = 30.0;

    /// The panel's own search field, under the rows: Raycast's "Search for
    /// actions...". Its rule above is drawn inside it (border-box).
    pub const SEARCH_HEIGHT: f32 = 48.0;

    /// The rule where the group changes, and the air above and below it.
    pub const SEPARATOR_HEIGHT: f32 = 1.0;
    pub const SEPARATOR_MARGIN: f32 = 4.0;

    /// How many rows are shown before the rest scroll, in the base style
    /// (`actions.max-rows`).
    pub const MAX_ROWS: usize = 7;

    /// The icon at the start of a row: smaller than a result's, because the
    /// row is.
    pub const ICON_SIZE: f32 = 16.0;
    pub const ICON_GAP: f32 = 10.0;

    /// Which rows have a rule above them: wherever the group differs from the
    /// row before. The first never does.
    pub fn breaks(groups: &[ActionGroup]) -> Vec<bool> {
        groups
            .iter()
            .enumerate()
            .map(|(index, group)| index > 0 && groups[index - 1] != *group)
            .collect()
    }

    /// Where `entry` sits among the scroll container's children, which hold a
    /// rule wherever the group changes as well as the rows. What
    /// `ScrollHandle::scroll_to_item` is given to keep a row in view.
    pub fn child_index(groups: &[ActionGroup], entry: usize) -> usize {
        let rules = breaks(groups)
            .into_iter()
            .take(entry + 1)
            .filter(|rule| *rule)
            .count();
        entry + rules
    }

    /// The popover's height for rows of these groups: the header, the padding
    /// and outline, the rows that are shown with the rules among them, and the
    /// search field under them.
    ///
    /// Capped at `max_rows`, the style's; only the rules among the rows shown
    /// count, since the others are reached by scrolling. Given every row, not
    /// the ones a search leaves, so typing in the field never resizes the
    /// window. Always computed, never set: a height of its own could clip a
    /// row (I-10).
    pub fn height(groups: &[ActionGroup], max_rows: usize) -> f32 {
        let shown = &groups[..groups.len().min(max_rows)];
        let rules = breaks(shown).into_iter().filter(|rule| *rule).count();

        HEADER_HEIGHT
            + PADDING * 2.0
            + BORDER * 2.0
            + shown.len() as f32 * ROW_HEIGHT
            + rules as f32 * (SEPARATOR_HEIGHT + SEPARATOR_MARGIN * 2.0)
            + SEARCH_HEIGHT
    }
}

/// The pill naming a quicklink while the search field holds its argument (F6).
pub mod argument {
    /// Padding inside the pill. Wider than tall: it sits on the query's
    /// baseline and must not make the search row look taller.
    pub const PILL_PADDING_X: f32 = 8.0;
    pub const PILL_PADDING_Y: f32 = 3.0;
    pub const PILL_RADIUS: f32 = 5.0;

    /// Between the pill and the caret.
    pub const PILL_GAP: f32 = 10.0;
}

/// The quicklink form, in the launcher and in settings (F6).
pub mod form {
    /// One labelled field. Taller than the field itself, which is the settings
    /// window's field (`settings::FIELD_HEIGHT`), so rows have air between them.
    pub const ROW_HEIGHT: f32 = 40.0;
    pub const LABEL_WIDTH: f32 = 96.0;
    pub const PADDING_Y: f32 = 10.0;

    /// The line under the fields: a hint, or why saving was refused. Reserved
    /// whether or not it has anything to say, so a refusal does not resize
    /// the window under the user's hands (I-10).
    pub const MESSAGE_HEIGHT: f32 = 30.0;

    pub const FIELDS: usize = 4;

    /// The whole form, which is also the region it takes over in the launcher.
    pub const HEIGHT: f32 = PADDING_Y * 2.0 + ROW_HEIGHT * FIELDS as f32 + MESSAGE_HEIGHT;

    /// The form in settings, which has two more rows: the icon and the
    /// fallback checkbox.
    pub const HEIGHT_WITH_SETTINGS_ROWS: f32 = HEIGHT + ROW_HEIGHT * 2.0;

    /// One of a duration's three fields -- hours, minutes, seconds -- and
    /// the gap before and after its unit (F8).
    pub const DURATION_FIELD_WIDTH: f32 = 56.0;
    pub const DURATION_GAP: f32 = 6.0;

    /// The fallback checkbox, and the gap before its caption.
    pub const CHECKBOX_SIZE: f32 = 16.0;
    pub const CHECKBOX_RADIUS: f32 = 4.0;
    pub const CHECKBOX_GAP: f32 = 8.0;
}

/// The snippet editor (F11): a multi-line field with a formatting toolbar
/// under the text.
pub mod text_area {
    /// Inside the border, around the text and the toolbar.
    pub const PADDING: f32 = 10.0;
    pub const RADIUS: f32 = 6.0;
    pub const TOOLBAR_HEIGHT: f32 = 30.0;
    /// One button: square for a letter, as wide as it must be for a word,
    /// with this much either side of it.
    pub const TOOL_SIZE: f32 = 24.0;
    pub const TOOL_PADDING_X: f32 = 5.0;
    pub const TOOL_GAP: f32 = 4.0;
    pub const TOOL_RADIUS: f32 = 4.0;
    /// The rule between groups of buttons.
    pub const TOOL_DIVIDER_HEIGHT: f32 = 16.0;
}

/// The snippet form (F11): the editor on the left, name, keyword and tags
/// on the right, as Raycast lays it out.
pub mod snippet_form {
    pub const PADDING_Y: f32 = 14.0;
    pub const COLUMN_GAP: f32 = 20.0;
    /// A label above its field.
    pub const LABEL_HEIGHT: f32 = 22.0;
    pub const EDITOR_HEIGHT: f32 = 200.0;
    /// The line under the editor saying what placeholders are.
    pub const HINT_HEIGHT: f32 = 38.0;
    /// Between one labelled field and the next.
    pub const GROUP_GAP: f32 = 12.0;
    /// The editor's share of the width.
    pub const EDITOR_SHARE: f32 = 0.52;

    /// The whole form, which is also the region it takes over in the launcher.
    pub const HEIGHT: f32 = PADDING_Y * 2.0 + LABEL_HEIGHT + EDITOR_HEIGHT + HINT_HEIGHT;
}

/// A dropdown's list of options (F6).
pub mod select {
    pub const ROW_HEIGHT: f32 = 28.0;
    pub const ROW_RADIUS: f32 = 5.0;
    /// Inside the list, around its rows.
    pub const PADDING: f32 = 4.0;
    /// Between the field and the list below it.
    pub const GAP: f32 = 4.0;
    /// Fixed rather than the field's: the list is laid out apart from it, and
    /// browser names are short.
    pub const MENU_WIDTH: f32 = 280.0;

    /// Rows shown before the list scrolls. Five fit above the launcher form's
    /// last field, which is where the list opens there.
    pub const VISIBLE_ROWS: usize = 5;
    pub const MAX_HEIGHT: f32 = ROW_HEIGHT * VISIBLE_ROWS as f32 + PADDING * 2.0;
}

/// Extension views (F4).
///
/// A view's list is drawn with the same row height and gutter as the root list,
/// deliberately: an extension's results must not look like a different
/// application, and they cannot if the geometry is not theirs to choose (AD-07).
/// What is here is only what a view has and the root list does not.
pub mod view {
    /// Height of a section header.
    ///
    /// Shorter than a row and not a whole row's worth: a header is a label, not
    /// an entry, and giving it the same height makes a two-item section look
    /// like a four-item one.
    pub const SECTION_HEIGHT: f32 = 28.0;

    /// Between a section's title and its subtitle (F9).
    pub const SECTION_SUBTITLE_GAP: f32 = 8.0;

    /// Gap between an accessory and the next thing to its left.
    pub const ACCESSORY_GAP: f32 = 8.0;

    /// Padding inside a tag pill.
    pub const TAG_PADDING_X: f32 = 7.0;
    pub const TAG_PADDING_Y: f32 = 2.0;
    pub const TAG_RADIUS: f32 = 4.0;

    /// A colour's swatch in the icon square (F12): smaller than the square,
    /// so it sits with the glyphs beside it rather than filling past them.
    pub const SWATCH_SIZE: f32 = 18.0;
    pub const SWATCH_RADIUS: f32 = 5.0;

    /// The picture at the top of the pane (F12), fitted inside and centred.
    /// A fixed height, so the metadata under it does not move when a picture
    /// of another shape is selected, or when it finishes loading (I-10).
    pub const DETAIL_IMAGE_HEIGHT: f32 = 220.0;

    /// A picture's thumbnail in the icon square (F12): cropped to fill it,
    /// with the corners the swatch has.
    pub const THUMBNAIL_RADIUS: f32 = SWATCH_RADIUS;

    /// The empty state, centred where the list would be.
    pub const EMPTY_ICON_SIZE: f32 = 32.0;
    pub const EMPTY_GAP: f32 = 8.0;

    /// How much room an empty state is given.
    ///
    /// A list with no rows would otherwise size the window to nothing, and the
    /// icon, the title and the description would spill over the footer. Three
    /// rows is enough for all three with air around them, and small enough
    /// that a search matching nothing does not open a tall empty box.
    pub const EMPTY_HEIGHT: f32 = super::ROW_HEIGHT * 3.0;

    /// Options a scope's picker shows before it scrolls (F7). Drawn with the
    /// action panel's rows, so this many of those, plus its padding.
    pub const PICKER_ROWS: usize = 10;

    /// The list's share of the width when a detail pane is beside it (F10).
    /// The pane gets the rest: it holds sentences, the rows hold one line.
    pub const DETAIL_LIST_SHARE: f32 = 0.4;

    /// Padding inside the pane, above and below; the sides use the gutter.
    pub const DETAIL_PADDING_Y: f32 = 12.0;

    /// Between one block of a detail's body and the next.
    pub const DETAIL_BLOCK_GAP: f32 = 8.0;

    /// Between a bullet's mark and its text.
    pub const DETAIL_BULLET_GAP: f32 = 6.0;

    /// Between a metadata row's label and its value.
    pub const DETAIL_METADATA_GAP: f32 = 12.0;

    /// A file's preview in the pane (F16): text in a card on a raised
    /// surface, as Raycast shows it. Padded wider than tall, so the first
    /// column is not against the edge.
    pub const DETAIL_CARD_PADDING_X: f32 = 16.0;
    pub const DETAIL_CARD_PADDING_Y: f32 = 12.0;
    pub const DETAIL_CARD_RADIUS: f32 = 8.0;

    /// The tallest a card grows: a picture's height, so a preview of either
    /// kind leaves the metadata where the eye found it, a few rows down.
    /// Its lines past it are clipped; the pane scrolls to what is under it.
    pub const DETAIL_CARD_MAX_HEIGHT: f32 = DETAIL_IMAGE_HEIGHT;
}

/// Type sizes. Not themeable (I-11); a theme may change the family only.
pub mod text {
    pub const QUERY: f32 = 17.0;
    /// A detail's first-level heading (F10): the query's size, so the word
    /// being defined reads at the size it was typed.
    pub const HEADING: f32 = QUERY;
    pub const TITLE: f32 = 13.5;
    pub const SUBTITLE: f32 = 12.0;
    pub const FOOTER: f32 = 11.5;
    /// The expression and the result in the calculator's card (F14).
    pub const CARD: f32 = 22.0;
    /// A file's preview card (F16): a monospace face at the subtitle's size
    /// is as wide as the title's proportional one, and fits a line of code.
    pub const CODE: f32 = SUBTITLE;
}

/// Appearance animation, per F2: "fade plus a 0.98-to-1.0 scale over roughly
/// 120 ms. Nothing more."
///
/// The restraint is the feature. F2 names the temptation to animate everything
/// as its principal risk, and excess motion as the most common error and the
/// hardest to walk back.
pub mod motion {
    use std::time::Duration;

    pub const APPEAR: Duration = Duration::from_millis(120);
    pub const APPEAR_SCALE_FROM: f32 = 0.98;

    /// The overlay fades in over this long, with ease-out.
    pub const OVERLAY_ENTER: Duration = Duration::from_millis(150);

    /// ...and out over this long. Deliberately shorter than the entry:
    /// "Equal durations feel heavy" (F3).
    pub const OVERLAY_EXIT: Duration = Duration::from_millis(90);
}

// These relationships between the tokens are checked at compile time rather
// than in a test, because every value involved is a constant: a change that
// breaks one is a mistake that should never produce a binary at all, let alone
// reach a test run.

/// The icon request must stay comfortably above what any sane display scale
/// would need, or icons go soft and the whole surface looks cheap.
const _: () = assert!(ICON_RESOLUTION as f32 >= ICON_SIZE * 3.0);

/// A row's highlight has to fit inside the row it highlights.
const _: () = assert!(ROW_RADIUS < ROW_HEIGHT / 2.0);
const _: () = assert!(ROW_INSET * 2.0 < ROW_HEIGHT);

/// The window's own radius must exceed a row's, or the selected row's corners
/// cut outside the window's at the top and bottom of the list.
const _: () = assert!(WINDOW_RADIUS > ROW_RADIUS);

/// A row's highlight is inset from the window edge, so the inset has to leave
/// room for the text that follows the gutter.
const _: () = assert!(GUTTER > ROW_INSET);

/// Geometry for the shortcut overlay.
///
/// Separate from the launcher's because the two surfaces are not variations on
/// each other: one is a single scrolling column sized to its results, the other
/// is a static multi-column sheet sized to its content.
pub mod overlay {
    /// Logical width of the sheet panel.
    pub const WIDTH: f32 = 920.0;

    /// Padding inside the panel, on every side.
    pub const PADDING: f32 = 26.0;

    /// The heading naming the matched application.
    pub const HEADER_HEIGHT: f32 = 38.0;

    /// How many columns the groups are distributed across.
    pub const COLUMNS: usize = 3;

    /// Horizontal gap between columns.
    pub const COLUMN_GAP: f32 = 26.0;

    /// One shortcut row.
    pub const ROW_HEIGHT: f32 = 24.0;

    /// A group's title row.
    pub const GROUP_TITLE_HEIGHT: f32 = 26.0;

    /// Vertical gap below a finished group, before the next one starts.
    pub const GROUP_GAP: f32 = 16.0;

    /// Corner radius of the panel.
    pub const RADIUS: f32 = 14.0;

    /// Gap between the keycaps of one chord.
    pub const KEYCAP_GAP: f32 = 4.0;

    /// Horizontal padding inside a keycap.
    pub const KEYCAP_PADDING_X: f32 = 5.0;

    /// Corner radius of a keycap.
    pub const KEYCAP_RADIUS: f32 = 4.0;

    /// Where the panel sits vertically in the work area.
    ///
    /// Nearer the middle than the launcher: the overlay is read, not typed
    /// into, so it is not competing with the user's gaze at a text field.
    pub const VERTICAL_BIAS: f32 = 0.3;

    pub mod text {
        pub const HEADING: f32 = 15.0;
        pub const GROUP: f32 = 11.0;
        pub const DESCRIPTION: f32 = 12.5;
        pub const KEYS: f32 = 11.5;
    }
}

/// Geometry for the settings window.
///
/// A third set, because the settings window is a different kind of surface
/// again: it is a document the user works in rather than a flyout they glance
/// at, so it is larger, denser, and sized once rather than per invocation.
pub mod settings {
    /// Logical size of the window. Fixed: the content is a two-pane editor whose
    /// proportions are calibrated, not something that benefits from resizing.
    pub const WIDTH: f32 = 900.0;
    /// 674 measured in Raycast's; 672 is the nearest whole number of the
    /// nav's 35-point pitch above the title bar and the search field.
    pub const HEIGHT: f32 = 672.0;

    /// Width of the sheet list down the left-hand side.
    pub const SIDEBAR_WIDTH: f32 = 240.0;

    /// Padding inside each pane.
    pub const PADDING: f32 = 18.0;

    /// The title bar the application draws for itself, since the window has no
    /// system decoration.
    pub const HEADER_HEIGHT: f32 = 46.0;

    /// The footer carrying the save state and its hint.
    pub const FOOTER_HEIGHT: f32 = 44.0;

    /// One row in the sheet list.
    pub const SIDEBAR_ROW_HEIGHT: f32 = 34.0;

    /// A quicklink's icon in the list, and the space after it.
    pub const ICON_SIZE: f32 = 16.0;
    pub const ICON_GAP: f32 = 8.0;

    /// One editable shortcut row.
    pub const ROW_HEIGHT: f32 = 36.0;

    /// Gap between stacked controls.
    pub const GAP: f32 = 8.0;

    /// Gap between one group of shortcuts and the next.
    pub const GROUP_GAP: f32 = 18.0;

    pub const RADIUS: f32 = 10.0;

    /// A text field's height, padding and corner.
    pub const FIELD_HEIGHT: f32 = 28.0;
    pub const FIELD_PADDING: f32 = 8.0;
    pub const FIELD_RADIUS: f32 = 6.0;

    /// Width of the keys field, which holds short chords and should not take
    /// space from the description beside it.
    pub const KEYS_FIELD_WIDTH: f32 = 180.0;

    /// A square button, for the add and remove controls.
    pub const BUTTON_SIZE: f32 = 28.0;

    /// The confirmation dialog, centred over the window.
    pub const DIALOG_WIDTH: f32 = 380.0;

    /// Between the page tabs in the header.
    pub const TAB_GAP: f32 = 4.0;

    // F15's redesign, measured from Raycast's settings window at 1.22
    // screen pixels to the point.

    /// The title bar: the window's name, and a close button.
    pub const TITLE_HEIGHT: f32 = 41.0;

    /// The navigation down the left: a search over the pages, then a row
    /// per page with its icon.
    pub const NAV_WIDTH: f32 = 237.0;
    pub const NAV_PADDING: f32 = 11.0;
    pub const NAV_SEARCH_HEIGHT: f32 = 33.0;
    pub const NAV_ROW_HEIGHT: f32 = 32.0;
    pub const NAV_ROW_GAP: f32 = 3.0;
    pub const NAV_RADIUS: f32 = 7.0;
    pub const NAV_ICON_SIZE: f32 = 18.0;
    pub const NAV_ICON_GAP: f32 = 10.0;

    /// The content column: cards of rows under section titles.
    pub const CONTENT_PADDING: f32 = 16.0;
    pub const CARD_RADIUS: f32 = 8.0;
    pub const CARD_GAP: f32 = 12.0;
    pub const CARD_PADDING_X: f32 = 13.0;
    /// A row with a title; one with a description under it too.
    pub const CARD_ROW_HEIGHT: f32 = 48.0;
    pub const CARD_ROW_TALL_HEIGHT: f32 = 58.0;
    /// Between a row's text and its control.
    pub const CARD_ROW_GAP: f32 = 16.0;
    /// Between a row's title and its description.
    pub const CARD_ROW_LINE_GAP: f32 = 2.0;
    /// A section's title, above its card.
    pub const SECTION_TITLE_HEIGHT: f32 = 30.0;
    /// A control at a row's right edge: a field that would otherwise run the
    /// row's whole width, a compact dropdown's list.
    pub const CONTROL_WIDTH: f32 = 240.0;
    /// A field for a number or a time.
    pub const CONTROL_NARROW_WIDTH: f32 = 96.0;
    /// The line under a page's cards: what the focused row means, or why a
    /// save was refused.
    pub const NOTE_HEIGHT: f32 = 24.0;

    /// A toggle: a track, and the knob inside it.
    pub const TOGGLE_WIDTH: f32 = 32.0;
    pub const TOGGLE_HEIGHT: f32 = 18.0;
    pub const TOGGLE_KNOB: f32 = 14.0;
    pub const TOGGLE_INSET: f32 = 2.0;

    /// A list editor's list: Quicklinks, Translation, Timers, Shortcut
    /// Sheets. A card beside the editor's card, its rows inset from its
    /// edges as the nav's are.
    pub const LIST_WIDTH: f32 = 220.0;
    pub const LIST_INSET: f32 = 4.0;

    /// The line under the content saying what was saved, or why not.
    pub const STATUS_HEIGHT: f32 = 30.0;

    /// The Shortcuts page (F17): a row per command, its title over its kind
    /// or what is wrong with it, then the alias and the hotkey fields. One
    /// height for every row, so the list is virtualised (I-10).
    pub const SHORTCUT_ROW_HEIGHT: f32 = 46.0;
    pub const SHORTCUT_ALIAS_WIDTH: f32 = 120.0;
    pub const SHORTCUT_HOTKEY_WIDTH: f32 = 176.0;
    /// The square that clears a hotkey, inside its field.
    pub const SHORTCUT_CLEAR_SIZE: f32 = 18.0;
    /// The icons on the page are drawn at most this many pixels square.
    pub const SHORTCUT_ICON_RESOLUTION: u32 = 32;

    /// A key cap, in the recorder and in a saved hotkey.
    pub const KEYCAP_HEIGHT: f32 = 20.0;
    pub const KEYCAP_PADDING_X: f32 = 5.0;
    pub const KEYCAP_RADIUS: f32 = 4.0;
    pub const KEYCAP_GAP: f32 = 3.0;

    pub mod text {
        pub const TITLE: f32 = 15.0;
        pub const SECTION: f32 = 11.0;
        pub const FIELD: f32 = 12.5;
        pub const LABEL: f32 = 12.0;
        pub const HINT: f32 = 11.5;
        /// F15: the window's name, a page's row in the nav, a card row's
        /// title and description, a section's title.
        pub const WINDOW: f32 = 12.5;
        pub const NAV: f32 = 13.5;
        pub const ROW_TITLE: f32 = 13.5;
        pub const ROW_DESCRIPTION: f32 = 11.5;
        pub const SECTION_TITLE: f32 = 13.5;
    }
}

/// The toggle's knob fits its track with the inset either side.
const _: () =
    assert!(settings::TOGGLE_KNOB + 2.0 * settings::TOGGLE_INSET <= settings::TOGGLE_HEIGHT);
const _: () = assert!(settings::TOGGLE_HEIGHT < settings::CARD_ROW_HEIGHT);
/// A shortcut row holds a field, and a field a key cap.
const _: () = assert!(settings::FIELD_HEIGHT < settings::SHORTCUT_ROW_HEIGHT);
const _: () = assert!(settings::KEYCAP_HEIGHT < settings::FIELD_HEIGHT);
const _: () = assert!(settings::SHORTCUT_CLEAR_SIZE < settings::FIELD_HEIGHT);
/// A dialog still fits the content column beside the nav.
const _: () = assert!(
    settings::DIALOG_WIDTH
        < settings::WIDTH - settings::NAV_WIDTH - settings::CONTENT_PADDING * 2.0
);

/// The snippet form's right column -- three labelled fields and the line
/// that says why a save was refused -- fits beside the editor.
const _: () = assert!(
    3.0 * (snippet_form::LABEL_HEIGHT + settings::FIELD_HEIGHT)
        + 2.0 * snippet_form::GROUP_GAP
        + form::MESSAGE_HEIGHT
        <= snippet_form::HEIGHT - 2.0 * snippet_form::PADDING_Y
);

/// A settings field has to fit inside the row that contains it.
const _: () = assert!(settings::FIELD_HEIGHT < settings::ROW_HEIGHT);
const _: () = assert!(settings::ICON_SIZE < settings::SIDEBAR_ROW_HEIGHT);
const _: () = assert!(settings::FIELD_HEIGHT < form::ROW_HEIGHT);
const _: () = assert!(form::CHECKBOX_SIZE < settings::FIELD_HEIGHT);
/// Opened upward from the launcher form's last field, the list must fit in
/// the rows above it and the search field.
const _: () = assert!(
    select::MAX_HEIGHT + select::GAP
        <= SEARCH_HEIGHT + form::PADDING_Y + form::ROW_HEIGHT * (form::FIELDS - 1) as f32
);
const _: () = assert!(settings::DIALOG_WIDTH < settings::WIDTH - settings::PADDING * 2.0);
const _: () = assert!(settings::KEYS_FIELD_WIDTH * 2.0 < settings::WIDTH - settings::SIDEBAR_WIDTH);

#[cfg(test)]
mod tests {
    use super::action_panel::{self, breaks};
    use crate::item::ActionGroup::{Copy, Favorite, Primary};

    #[test]
    fn a_rule_falls_wherever_the_group_changes() {
        assert_eq!(
            breaks(&[Primary, Primary, Copy, Copy, Favorite]),
            [false, false, true, false, true]
        );
        assert_eq!(breaks(&[Primary, Primary]), [false, false]);
        assert!(breaks(&[]).is_empty());
    }

    /// The scroll container holds the rules as children of their own, so the
    /// n-th entry is not its n-th child.
    #[test]
    fn an_entry_sits_after_the_rules_that_precede_it() {
        let groups = [Primary, Primary, Copy, Copy, Favorite];

        let at: Vec<_> = (0..groups.len())
            .map(|entry| action_panel::child_index(&groups, entry))
            .collect();

        assert_eq!(at, [0, 1, 3, 4, 6]);
    }

    #[test]
    fn the_popover_is_a_header_and_its_rows_and_rules_and_its_field() {
        let chrome = action_panel::HEADER_HEIGHT
            + action_panel::PADDING * 2.0
            + action_panel::BORDER * 2.0
            + action_panel::SEARCH_HEIGHT;
        let rule = action_panel::SEPARATOR_HEIGHT + action_panel::SEPARATOR_MARGIN * 2.0;

        assert_eq!(
            action_panel::height(&[Primary, Primary, Copy], action_panel::MAX_ROWS),
            chrome + 3.0 * action_panel::ROW_HEIGHT + rule
        );
        assert_eq!(
            action_panel::height(&[Primary], action_panel::MAX_ROWS),
            chrome + action_panel::ROW_HEIGHT
        );
    }

    /// Past the cap the popover stops growing and scrolls, so a long menu
    /// cannot ask for a window taller than the screen.
    #[test]
    fn the_popover_stops_growing_at_the_row_cap() {
        let many = vec![Primary; action_panel::MAX_ROWS + 5];
        let capped = vec![Primary; action_panel::MAX_ROWS];

        assert_eq!(
            action_panel::height(&many, action_panel::MAX_ROWS),
            action_panel::height(&capped, action_panel::MAX_ROWS)
        );
    }

    /// Only the rules among the rows that are shown cost height; one further
    /// down is reached by scrolling.
    #[test]
    fn a_rule_beyond_the_cap_does_not_make_the_popover_taller() {
        let mut groups = vec![Primary; action_panel::MAX_ROWS];
        groups.push(Copy);

        assert_eq!(
            action_panel::height(&groups, action_panel::MAX_ROWS),
            action_panel::height(&groups[..action_panel::MAX_ROWS], action_panel::MAX_ROWS)
        );
    }

    /// The cap is the style's: a higher one lets the same menu show more
    /// rows before it scrolls, a row's height for each.
    #[test]
    fn the_row_cap_is_given_not_fixed() {
        let many = vec![Primary; 20];

        assert_eq!(
            action_panel::height(&many, 10) - action_panel::height(&many, 7),
            3.0 * action_panel::ROW_HEIGHT
        );
        assert_eq!(
            action_panel::height(&many, 3),
            action_panel::height(&many[..3], 12)
        );
    }
}
