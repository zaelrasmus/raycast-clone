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

/// The action panel, opened with Ctrl+K over the bottom-right of the launcher.
pub mod action_panel {
    /// One action row.
    pub const ROW_HEIGHT: f32 = 30.0;

    /// Padding inside the panel.
    pub const PADDING: f32 = 6.0;

    pub const ROW_RADIUS: f32 = 5.0;
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

    /// The fallback checkbox, and the gap before its caption.
    pub const CHECKBOX_SIZE: f32 = 16.0;
    pub const CHECKBOX_RADIUS: f32 = 4.0;
    pub const CHECKBOX_GAP: f32 = 8.0;
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

    /// Gap between an accessory and the next thing to its left.
    pub const ACCESSORY_GAP: f32 = 8.0;

    /// Padding inside a tag pill.
    pub const TAG_PADDING_X: f32 = 7.0;
    pub const TAG_PADDING_Y: f32 = 2.0;
    pub const TAG_RADIUS: f32 = 4.0;

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
}

/// Type sizes. Not themeable (I-11); a theme may change the family only.
pub mod text {
    pub const QUERY: f32 = 17.0;
    pub const TITLE: f32 = 13.5;
    pub const SUBTITLE: f32 = 12.0;
    pub const FOOTER: f32 = 11.5;
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
    pub const HEIGHT: f32 = 620.0;

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

    pub mod text {
        pub const TITLE: f32 = 15.0;
        pub const SECTION: f32 = 11.0;
        pub const FIELD: f32 = 12.5;
        pub const LABEL: f32 = 12.0;
        pub const HINT: f32 = 11.5;
    }
}

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
