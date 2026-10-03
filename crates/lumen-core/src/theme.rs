//! Themes as pure data (AD-12).
//!
//! A theme is a colour and typography definition file and nothing else. It
//! installs by being dropped into a folder: no code, no runtime, no sandbox,
//! nothing to review. Four rules hold this together, and each one is load
//! bearing:
//!
//! - **Semantic tokens.** `surface.background`, never `list_item_hover`. A token
//!   that names a component turns every new component into a break for every
//!   existing theme (AD-12).
//! - **Default resolution.** Every missing token falls back to the base theme,
//!   so widening the palette never invalidates a theme written a year ago (I-12).
//! - **Versioned schema** from the very first file, so migration replaces
//!   breakage.
//! - **No geometry.** Colour and type family exclusively (AD-13, I-11). This is
//!   the rule that protects the thing the project exists for: if a third-party
//!   theme can change row heights, the calibrated layout dies the first day
//!   somebody publishes one.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::style::Style;

/// The schema version this build writes and understands.
pub const SCHEMA_VERSION: u32 = 1;

/// The typeface compiled into the binary, used unless a theme names another.
///
/// Declared here rather than in the presentation layer because it is the
/// default *value* of a theme token, and themes are core's concern. The font
/// files themselves live with the renderer that loads them.
pub const BUNDLED_FONT: &str = "Inter";

/// The monospace family, used unless a palette names another: a file's
/// preview in *Search Files* (F16). Not bundled: Windows 11 ships it. A
/// family that is not installed falls back as the interface family does --
/// gpui logs it once and draws in the system's interface font.
pub const MONO_FONT: &str = "Cascadia Mono";

/// A colour as straight, non-premultiplied RGBA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::rgba(r, g, b, 255)
    }

    /// Parse `#RGB`, `#RRGGBB` or `#RRGGBBAA`.
    ///
    /// The alpha suffix is not decoration: over an acrylic backdrop, a surface
    /// token *must* be able to say "80% opaque" or the blur is buried.
    pub fn parse(value: &str) -> Result<Self> {
        let hex = value.strip_prefix('#').unwrap_or(value);
        let digits: Vec<u8> = hex
            .chars()
            .map(|c| {
                c.to_digit(16)
                    .map(|d| d as u8)
                    .with_context(|| format!("{value:?} is not a hex colour"))
            })
            .collect::<Result<_>>()?;

        Ok(match digits.len() {
            3 => Self::rgb(digits[0] * 17, digits[1] * 17, digits[2] * 17),
            6 => Self::rgb(
                digits[0] * 16 + digits[1],
                digits[2] * 16 + digits[3],
                digits[4] * 16 + digits[5],
            ),
            8 => Self::rgba(
                digits[0] * 16 + digits[1],
                digits[2] * 16 + digits[3],
                digits[4] * 16 + digits[5],
                digits[6] * 16 + digits[7],
            ),
            other => anyhow::bail!("{value:?} has {other} hex digits; expected 3, 6 or 8"),
        })
    }

    /// Pack as `0xAARRGGBB`, which is what GPUI's `rgba` helper consumes.
    /// `#RRGGBB`, or `#RRGGBBAA` when it is not opaque.
    pub fn hex(self) -> String {
        if self.a == 255 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }

    pub const fn to_argb(self) -> u32 {
        ((self.a as u32) << 24) | ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }

    /// This colour at a different alpha. Used for states a theme should not have
    /// to enumerate, such as a pressed row derived from a hovered one.
    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }
}

impl Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Color::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// Whether a theme is meant for a light or a dark desktop.
///
/// Drives the system's dark-mode window attribute, so it has to be declared
/// rather than guessed from the background's luminance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    Light,
    #[default]
    Dark,
}

/// A theme exactly as it appears on disk.
///
/// Every colour is optional: absence means "inherit from the base theme", which
/// is what makes I-12 true rather than aspirational.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeFile {
    pub schema_version: u32,
    pub name: String,
    #[serde(default)]
    pub appearance: Appearance,
    #[serde(default)]
    pub colors: BTreeMap<String, Color>,
    /// The type family. The *only* typographic property a theme may set, and
    /// deliberately not a size: sizes are geometry (AD-13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    /// The monospace family: text shown as written, such as a file's
    /// preview (F16). A family, like `font_family`, never a size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_mono: Option<String>,
    /// OpenType features of the family, by their four-letter tags: Inter's
    /// `cv11` (a single-storey a), `ss03` (rounded quotes), `tnum` (figures
    /// of one width). 1 turns one on, 0 off (F15). Typography, not geometry.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub font_features: BTreeMap<String, u32>,
}

/// A fully resolved theme: every token present, nothing left to fall back on.
///
/// The UI only ever sees this type, so no rendering code has to consider what
/// happens when a token is missing (I-5 stays easy to obey).
///
/// It carries the palette and the style together (F15). They are chosen and
/// loaded apart, but a component receives one value, so nothing that draws
/// has to be handed two and keep them in step.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub appearance: Appearance,
    pub font_family: Option<String>,
    /// The monospace family; [`MONO_FONT`] unless the palette names one.
    pub font_mono: String,
    /// OpenType features, as (tag, value); empty is the family's defaults.
    pub font_features: Vec<(String, u32)>,
    pub colors: Colors,
    /// The launcher's geometry and variants, from the style file `style` in
    /// `config.json` names. The base style until one is loaded.
    pub style: Style,
}

/// The semantic colour palette.
///
/// Adding a field here is safe by construction: existing theme files simply
/// inherit the new token from the base theme.
#[derive(Debug, Clone)]
pub struct Colors {
    pub surface_background: Color,
    pub surface_elevated: Color,
    pub border_subtle: Color,
    pub border_strong: Color,
    pub text_primary: Color,
    pub text_muted: Color,
    /// The bundled glyphs. Between primary and muted on purpose: an icon at
    /// this size carries less ink than a word, so muted makes it disappear and
    /// primary makes it shout over the title beside it.
    pub icon: Color,
    pub text_placeholder: Color,
    pub text_on_accent: Color,
    pub accent: Color,
    pub selection_background: Color,
    pub selection_border: Color,
    pub match_highlight: Color,
    /// Behind text selected in a field. Translucent, so the glyphs it covers
    /// keep their own colour and stay legible.
    pub text_selection: Color,
    pub shadow: Color,
    /// What went wrong, where it went wrong: a form's refusal to save (F6).
    pub text_error: Color,
    /// Behind the launcher's footer, over the surface (F15). Transparent in
    /// the base themes: the footer is the surface unless a palette says
    /// otherwise, as Asyar's and Vicinae's do.
    pub surface_footer: Color,
    /// The window's own edge (F15). Unset, it is the palette's
    /// `border.subtle` -- not the base theme's -- so a palette that set its
    /// separators keeps them and its edge one colour, as before the token.
    pub border_window: Color,
}

impl Colors {
    /// The token names a theme file may set, paired with where they land.
    ///
    /// One table, used both to apply a theme file and to report unknown tokens
    /// back to its author. Two parallel lists would drift.
    fn apply(&mut self, name: &str, value: Color) -> bool {
        let slot = match name {
            "surface.background" => &mut self.surface_background,
            "surface.elevated" => &mut self.surface_elevated,
            "border.subtle" => &mut self.border_subtle,
            "border.strong" => &mut self.border_strong,
            "text.primary" => &mut self.text_primary,
            "text.muted" => &mut self.text_muted,
            "icon" => &mut self.icon,
            "text.placeholder" => &mut self.text_placeholder,
            "text.on-accent" => &mut self.text_on_accent,
            "accent" => &mut self.accent,
            "selection.background" => &mut self.selection_background,
            "selection.border" => &mut self.selection_border,
            "match.highlight" => &mut self.match_highlight,
            "text.selection" => &mut self.text_selection,
            "shadow" => &mut self.shadow,
            "text.error" => &mut self.text_error,
            "surface.footer" => &mut self.surface_footer,
            "border.window" => &mut self.border_window,
            _ => return false,
        };
        *slot = value;
        true
    }
}

impl Theme {
    /// The base dark theme. Every other theme resolves its gaps from here.
    pub fn base_dark() -> Self {
        Self {
            name: "Nocturne".into(),
            appearance: Appearance::Dark,
            // The bundled face, so the interface looks the same on Windows and
            // on Hyprland. A palette may name a different family; sizes are
            // the style's (AD-13, I-11).
            font_family: Some(BUNDLED_FONT.into()),
            font_mono: MONO_FONT.into(),
            font_features: Vec::new(),
            style: Style::base(),
            colors: Colors {
                // Alpha below 255 on purpose: this surface sits over the system
                // acrylic, and an opaque value would hide the backdrop that F0
                // exists to obtain.
                surface_background: Color::rgba(0x0E, 0x0E, 0x12, 0xD8),
                surface_elevated: Color::rgba(0x17, 0x17, 0x1B, 0xF0),
                border_subtle: Color::rgba(0xFF, 0xFF, 0xFF, 0x14),
                border_strong: Color::rgba(0xFF, 0xFF, 0xFF, 0x24),
                text_primary: Color::rgb(0xED, 0xED, 0xF2),
                text_muted: Color::rgb(0x8A, 0x8A, 0x96),
                icon: Color::rgb(0xB4, 0xB4, 0xC0),
                text_placeholder: Color::rgb(0x60, 0x60, 0x6C),
                text_on_accent: Color::rgb(0xFF, 0xFF, 0xFF),
                accent: Color::rgb(0x7C, 0x6B, 0xD6),
                selection_background: Color::rgba(0xFF, 0xFF, 0xFF, 0x14),
                selection_border: Color::rgba(0xFF, 0xFF, 0xFF, 0x1A),
                match_highlight: Color::rgb(0xA8, 0x9B, 0xEC),
                text_selection: Color::rgba(0x7C, 0x6B, 0xD6, 0x66),
                shadow: Color::rgba(0x00, 0x00, 0x00, 0x99),
                text_error: Color::rgb(0xF0, 0x8C, 0x8C),
                surface_footer: Color::rgba(0, 0, 0, 0),
                border_window: Color::rgba(0xFF, 0xFF, 0xFF, 0x14),
            },
        }
    }

    /// The base light theme, used as the fallback for light theme files.
    pub fn base_light() -> Self {
        Self {
            name: "Daylight".into(),
            appearance: Appearance::Light,
            font_family: Some(BUNDLED_FONT.into()),
            font_mono: MONO_FONT.into(),
            font_features: Vec::new(),
            style: Style::base(),
            colors: Colors {
                surface_background: Color::rgba(0xF7, 0xF7, 0xFA, 0xDE),
                surface_elevated: Color::rgba(0xFF, 0xFF, 0xFF, 0xF2),
                border_subtle: Color::rgba(0x1A, 0x1A, 0x2E, 0x14),
                border_strong: Color::rgba(0x1A, 0x1A, 0x2E, 0x24),
                text_primary: Color::rgb(0x1A, 0x1A, 0x24),
                text_muted: Color::rgb(0x6B, 0x6B, 0x7B),
                icon: Color::rgb(0x4A, 0x4A, 0x5A),
                text_placeholder: Color::rgb(0x9A, 0x9A, 0xA8),
                text_on_accent: Color::rgb(0xFF, 0xFF, 0xFF),
                accent: Color::rgb(0x5B, 0x4A, 0xC4),
                selection_background: Color::rgba(0x1A, 0x1A, 0x2E, 0x0E),
                selection_border: Color::rgba(0x1A, 0x1A, 0x2E, 0x14),
                match_highlight: Color::rgb(0x4A, 0x39, 0xB0),
                text_selection: Color::rgba(0x5B, 0x4A, 0xC4, 0x40),
                shadow: Color::rgba(0x1A, 0x1A, 0x2E, 0x33),
                text_error: Color::rgb(0xB4, 0x2B, 0x2B),
                surface_footer: Color::rgba(0, 0, 0, 0),
                border_window: Color::rgba(0x1A, 0x1A, 0x2E, 0x14),
            },
        }
    }

    /// Resolve a theme file against the appropriate base theme.
    ///
    /// Unknown tokens are reported, never fatal: a theme written for a later
    /// version of Lumen must still load on this one (I-12).
    pub fn resolve(file: &ThemeFile) -> Self {
        let mut theme = match file.appearance {
            Appearance::Dark => Self::base_dark(),
            Appearance::Light => Self::base_light(),
        };

        theme.name = file.name.clone();
        theme.appearance = file.appearance;
        if let Some(family) = &file.font_family {
            theme.font_family = Some(family.clone());
        }
        if let Some(family) = &file.font_mono {
            theme.font_mono = family.clone();
        }
        // A tag is four printable ASCII characters; anything else would be
        // handed to the shaper as garbage, so it is reported and dropped.
        theme.font_features = file
            .font_features
            .iter()
            .filter(|(tag, _)| {
                let valid = tag.len() == 4 && tag.bytes().all(|byte| byte.is_ascii_graphic());
                if !valid {
                    log::warn!(
                        "theme {:?} sets font feature {tag:?}, which is not a four-letter tag; ignoring it",
                        file.name
                    );
                }
                valid
            })
            .map(|(tag, value)| (tag.clone(), *value))
            .collect();

        // The window's edge follows the separators unless it is set apart.
        if !file.colors.contains_key("border.window") {
            theme.colors.border_window = file
                .colors
                .get("border.subtle")
                .copied()
                .unwrap_or(theme.colors.border_subtle);
        }

        for (token, value) in &file.colors {
            if !theme.colors.apply(token, *value) {
                log::warn!(
                    "theme {:?} sets unknown token {token:?}; ignoring it",
                    file.name
                );
            }
        }

        theme
    }

    /// This palette with `style`'s geometry and variants.
    pub fn with_style(self, style: Style) -> Self {
        Self { style, ..self }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("reading theme {}", path.display()))?;
        let file: ThemeFile = serde_json::from_str(&source)
            .with_context(|| format!("parsing theme {}", path.display()))?;

        // A file from a *newer* schema still loads: unknown tokens are already
        // ignored, so the worst case is a theme that renders with some base
        // colours. Refusing outright would be the breakage I-12 forbids.
        if file.schema_version > SCHEMA_VERSION {
            log::warn!(
                "theme {:?} declares schema version {} but this build understands {SCHEMA_VERSION}; \
                 unknown tokens will fall back to the base theme",
                file.name,
                file.schema_version
            );
        }

        Ok(Self::resolve(&file))
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::base_dark()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_accepted_hex_shape() {
        assert_eq!(Color::parse("#fff").unwrap(), Color::rgb(255, 255, 255));
        assert_eq!(
            Color::parse("#0E0E12").unwrap(),
            Color::rgb(0x0E, 0x0E, 0x12)
        );
        assert_eq!(
            Color::parse("#0E0E12D8").unwrap(),
            Color::rgba(0x0E, 0x0E, 0x12, 0xD8)
        );
        // The leading hash is conventional, not required.
        assert_eq!(
            Color::parse("7C6BD6").unwrap(),
            Color::rgb(0x7C, 0x6B, 0xD6)
        );
    }

    #[test]
    fn rejects_malformed_colours_instead_of_guessing() {
        assert!(Color::parse("#12345").is_err());
        assert!(Color::parse("#zzzzzz").is_err());
    }

    #[test]
    fn packs_argb_for_the_renderer() {
        assert_eq!(Color::rgba(0x12, 0x34, 0x56, 0x78).to_argb(), 0x78123456);
    }

    /// I-12, stated as a test: a theme that sets one token keeps the base value
    /// for every other one.
    #[test]
    fn missing_tokens_resolve_from_the_base_theme() {
        let file = ThemeFile {
            schema_version: 1,
            name: "Sparse".into(),
            appearance: Appearance::Dark,
            colors: BTreeMap::from([("accent".to_string(), Color::rgb(1, 2, 3))]),
            font_family: None,
            font_mono: None,
            font_features: BTreeMap::new(),
        };

        let theme = Theme::resolve(&file);

        assert_eq!(theme.colors.accent, Color::rgb(1, 2, 3));
        assert_eq!(
            theme.colors.surface_background,
            Theme::base_dark().colors.surface_background
        );
        assert_eq!(theme.name, "Sparse");
    }

    /// A palette that set its separators before `border.window` existed
    /// keeps its window's edge the same colour (F15).
    #[test]
    fn the_window_edge_follows_the_palettes_own_separators() {
        let file = ThemeFile {
            schema_version: 1,
            name: "Old".into(),
            appearance: Appearance::Dark,
            colors: BTreeMap::from([("border.subtle".to_string(), Color::rgb(1, 2, 3))]),
            font_family: None,
            font_mono: None,
            font_features: BTreeMap::new(),
        };
        assert_eq!(
            Theme::resolve(&file).colors.border_window,
            Color::rgb(1, 2, 3)
        );

        let apart = ThemeFile {
            colors: BTreeMap::from([
                ("border.subtle".to_string(), Color::rgb(1, 2, 3)),
                ("border.window".to_string(), Color::rgb(4, 5, 6)),
            ]),
            ..file
        };
        assert_eq!(
            Theme::resolve(&apart).colors.border_window,
            Color::rgb(4, 5, 6)
        );
    }

    /// The monospace family is a palette's, like the interface family, and
    /// a palette written before it existed draws previews in the base's
    /// (I-12).
    #[test]
    fn the_monospace_family_resolves_from_the_base() {
        let old: ThemeFile =
            serde_json::from_str(r#"{ "schema_version": 1, "name": "Old" }"#).unwrap();
        assert_eq!(Theme::resolve(&old).font_mono, MONO_FONT);
        assert_eq!(Theme::base_light().font_mono, MONO_FONT);

        let set: ThemeFile = serde_json::from_str(
            r#"{ "schema_version": 1, "name": "Set", "font_mono": "Consolas" }"#,
        )
        .unwrap();
        assert_eq!(Theme::resolve(&set).font_mono, "Consolas");
    }

    #[test]
    fn font_features_keep_valid_tags_only() {
        let json = r##"{
            "schema_version": 1,
            "name": "Featured",
            "font_features": { "cv11": 1, "ss03": 1, "toolong": 1 }
        }"##;
        let file: ThemeFile = serde_json::from_str(json).unwrap();
        let theme = Theme::resolve(&file);
        assert_eq!(
            theme.font_features,
            [("cv11".to_string(), 1), ("ss03".to_string(), 1)]
        );
    }

    /// A theme written against a future, wider palette must still load today.
    #[test]
    fn unknown_tokens_are_ignored_not_fatal() {
        let file = ThemeFile {
            schema_version: 99,
            name: "FromTheFuture".into(),
            appearance: Appearance::Light,
            colors: BTreeMap::from([
                ("accent".to_string(), Color::rgb(9, 9, 9)),
                ("token.invented.later".to_string(), Color::rgb(1, 1, 1)),
            ]),
            font_family: None,
            font_mono: None,
            font_features: BTreeMap::new(),
        };

        let theme = Theme::resolve(&file);

        assert_eq!(theme.colors.accent, Color::rgb(9, 9, 9));
        assert_eq!(theme.appearance, Appearance::Light);
    }

    #[test]
    fn light_files_resolve_against_the_light_base() {
        let file = ThemeFile {
            schema_version: 1,
            name: "Sparse Light".into(),
            appearance: Appearance::Light,
            colors: BTreeMap::new(),
            font_family: None,
            font_mono: None,
            font_features: BTreeMap::new(),
        };

        let theme = Theme::resolve(&file);

        assert_eq!(
            theme.colors.text_primary,
            Theme::base_light().colors.text_primary
        );
    }

    /// Every shipped palette sets only tokens this build knows: a typo would
    /// be a colour silently left at the base.
    #[test]
    fn the_shipped_palettes_name_only_known_tokens() {
        for source in [
            include_str!("../../../assets/themes/nocturne.json"),
            include_str!("../../../assets/themes/daylight.json"),
            include_str!("../../../assets/themes/carbon.json"),
            include_str!("../../../assets/themes/graphite.json"),
            include_str!("../../../assets/themes/ink.json"),
        ] {
            let file: ThemeFile = serde_json::from_str(source).unwrap();
            let mut colors = Theme::base_dark().colors;
            for (token, value) in &file.colors {
                assert!(colors.apply(token, *value), "{} sets {token}", file.name);
            }
        }
    }

    #[test]
    fn round_trips_through_json() {
        // Doubled hashes: a hex colour contains `"#`, which would otherwise
        // terminate the raw string early.
        let json = r##"{
            "schema_version": 1,
            "name": "Nocturne",
            "appearance": "dark",
            "colors": { "accent": "#7C6BD6", "surface.background": "#0E0E12D8" }
        }"##;

        let file: ThemeFile = serde_json::from_str(json).unwrap();
        let theme = Theme::resolve(&file);

        assert_eq!(theme.colors.accent, Color::rgb(0x7C, 0x6B, 0xD6));
        assert_eq!(theme.colors.surface_background.a, 0xD8);
    }
}
