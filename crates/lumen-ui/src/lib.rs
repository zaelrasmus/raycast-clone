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

pub mod assets;
pub mod edit;
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
pub use launcher::{Launcher, LauncherEvent, ViewSink};
pub use overlay::{Overlay, OverlayEvent};
pub use settings::{Settings, SettingsEvent};
pub use text_field::TextField;
