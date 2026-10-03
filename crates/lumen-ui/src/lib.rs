//! The GPUI presentation layer.
//!
//! This is the only crate that depends on `gpui`. If GPUI turns out to be
//! unworkable at some point, this crate is replaced and the project is not lost
//! (AD-03).
//!
//! # Text input
//!
//! Every editable field is a [`text_field::TextField`]: a real GPUI
//! `EntityInputHandler` with its own shaped text element, so dead keys, AltGr
//! characters and input-method composition all work, and the caret moves and
//! selects like any other field on the system. The editing arithmetic is in
//! [`edit`], apart from GPUI, where it is tested without a window.

pub mod appearance_page;
pub mod assets;
pub mod calculator_page;
pub mod clipboard_page;
pub mod clock_page;
pub mod currency_page;
pub mod duration_input;
pub mod edit;
pub mod file_search_page;
pub mod fonts;
pub mod home_choice;
pub mod icons;
pub mod images;
pub mod keymap;
pub mod launcher;
pub mod overlay;
pub mod quicklink_form;
pub mod quicklinks_page;
pub mod select;
pub mod settings;
pub mod settings_ui;
pub mod snippet_form;
pub mod styling;
pub mod text_area;
pub mod text_field;
pub mod timer_form;
pub mod timers_page;
pub mod translate_page;
pub mod view;
pub mod weather_page;

pub use assets::Assets;
pub use fonts::FAMILY as FONT_FAMILY;
pub use launcher::{Launcher, LauncherEvent, ViewSink};
pub use overlay::{Overlay, OverlayEvent};
pub use settings::{Settings, SettingsEvent};
pub use text_field::TextField;

/// I-5 and I-14, checked against the source rather than trusted to review.
///
/// Both invariants are about what UI code must *not* contain, which is exactly
/// the kind of rule a single edit six months from now breaks without anyone
/// noticing. Reading the files is crude, and that is the point: it catches the
/// literal `px(4.0)` and the `keystroke.key ==` that a reviewer skims past.
#[cfg(test)]
mod invariants {
    const SOURCES: &[(&str, &str)] = &[
        ("appearance_page.rs", include_str!("appearance_page.rs")),
        ("assets.rs", include_str!("assets.rs")),
        ("edit.rs", include_str!("edit.rs")),
        ("fonts.rs", include_str!("fonts.rs")),
        ("icons.rs", include_str!("icons.rs")),
        ("keymap.rs", include_str!("keymap.rs")),
        ("launcher.rs", include_str!("launcher.rs")),
        ("overlay.rs", include_str!("overlay.rs")),
        ("quicklink_form.rs", include_str!("quicklink_form.rs")),
        ("quicklinks_page.rs", include_str!("quicklinks_page.rs")),
        ("select.rs", include_str!("select.rs")),
        ("settings.rs", include_str!("settings.rs")),
        ("settings_ui.rs", include_str!("settings_ui.rs")),
        ("styling.rs", include_str!("styling.rs")),
        ("text_field.rs", include_str!("text_field.rs")),
        ("view.rs", include_str!("view.rs")),
    ];

    /// Lines of code, with comments dropped, up to the file's test module.
    fn code(source: &str) -> impl Iterator<Item = (usize, &str)> {
        source
            .lines()
            .take_while(|line| !line.starts_with("#[cfg(test)]"))
            .enumerate()
            .map(|(index, line)| (index + 1, line.split("//").next().unwrap_or("")))
    }

    /// I-5: geometry comes from `lumen_core::layout`. Zero is not a size, so
    /// `px(0.0)` is allowed; any other number written into `px(...)` is not.
    #[test]
    fn no_geometry_literal_in_ui_code() {
        let mut found = Vec::new();
        for (file, source) in SOURCES {
            for (number, line) in code(source) {
                for (at, _) in line.match_indices("px(") {
                    let argument = &line[at + 3..];
                    let literal = argument.starts_with(|c: char| c.is_ascii_digit());
                    if literal && !argument.starts_with("0.0)") {
                        found.push(format!("{file}:{number}: {}", line.trim()));
                    }
                }
            }
        }
        assert!(
            found.is_empty(),
            "literal geometry (I-5):\n{}",
            found.join("\n")
        );
    }

    /// I-14: commands are actions bound from the keymap.
    #[test]
    fn no_hand_written_key_comparison_in_ui_code() {
        let mut found = Vec::new();
        for (file, source) in SOURCES {
            for (number, line) in code(source) {
                if line.contains("keystroke.key") || line.contains("on_key_down(") {
                    found.push(format!("{file}:{number}: {}", line.trim()));
                }
            }
        }
        assert!(
            found.is_empty(),
            "key comparison (I-14):\n{}",
            found.join("\n")
        );
    }
}
