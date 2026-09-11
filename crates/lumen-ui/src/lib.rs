//! The GPUI presentation layer.
//!
//! This is the only crate that depends on `gpui`. If GPUI turns out to be
//! unworkable at some point, this crate is replaced and the project is not lost
//! (AD-03).
//!
//! # Known gap: text input
//!
//! The search field is a plain string with a drawn caret, not a GPUI
//! `EntityInputHandler`. That means no IME, no mouse text selection and no
//! composition -- writing Japanese into the search field does not work today.
//!
//! This is a deliberate F1 trade, not an oversight. A correct input handler
//! requires a custom `Element` that shapes its own text and reports selection
//! geometry back to the system, which is a few hundred lines whose absence
//! costs nothing for the ASCII queries that make up the entire F1 acceptance
//! criteria. It is the right thing to fix before the launcher is offered to
//! anyone else, and `gpui`'s own `examples/input.rs` is the reference for doing
//! it.

pub mod assets;
pub mod fonts;
pub mod icons;
pub mod keymap;
pub mod launcher;
pub mod overlay;
pub mod settings;
pub mod styling;
pub mod text_field;
pub mod view;

pub use assets::Assets;
pub use fonts::FAMILY as FONT_FAMILY;
pub use launcher::{Launcher, LauncherEvent};
pub use overlay::{Overlay, OverlayEvent};
pub use settings::{Settings, SettingsEvent};
pub use text_field::TextField;
