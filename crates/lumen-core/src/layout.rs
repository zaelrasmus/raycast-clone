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
const _: () = assert!(settings::KEYS_FIELD_WIDTH * 2.0 < settings::WIDTH - settings::SIDEBAR_WIDTH);
