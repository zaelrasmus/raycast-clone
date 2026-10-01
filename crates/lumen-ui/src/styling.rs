//! Bridging Lumen's tokens into GPUI's types.
//!
//! Every colour and every measurement the UI draws with passes through this
//! module. That is what makes I-5 -- no literal colour, spacing, radius or type
//! size in interface code -- checkable by reading the imports of a file rather
//! than by auditing every expression in it.

use gpui::{FontWeight, Hsla, Pixels, Rgba, px};
use lumen_core::layout;
use lumen_core::style::{Metrics, Variants};
use lumen_core::theme::{Color, Theme};

/// Convert a token colour to GPUI's colour type.
pub fn color(value: Color) -> Hsla {
    Rgba {
        r: value.r as f32 / 255.0,
        g: value.g as f32 / 255.0,
        b: value.b as f32 / 255.0,
        a: value.a as f32 / 255.0,
    }
    .into()
}

/// The active style's geometry, in GPUI's units (F15).
///
/// What the launcher's surface draws with: the search field, the rows, the
/// section headers, the card and the footer, in the root list and in views.
/// Taken from the theme rather than from constants, so a style changes them;
/// a separate type from [`metrics`] so a launcher function cannot reach for a
/// constant where the style has a token, and a settings page cannot reach
/// for the style where it keeps its constants (7.41).
#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    metrics: Metrics,
    variants: Variants,
}

impl Geometry {
    pub fn of(theme: &Theme) -> Self {
        Self {
            metrics: theme.style.metrics,
            variants: theme.style.variants,
        }
    }

    /// The raw values, for arithmetic that happens before anything is drawn:
    /// the window's height.
    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    pub fn variants(&self) -> &Variants {
        &self.variants
    }

    pub fn window_radius(&self) -> Pixels {
        px(self.metrics.window_radius)
    }

    pub fn search_height(&self) -> Pixels {
        px(self.metrics.search_height)
    }

    pub fn gutter(&self) -> Pixels {
        px(self.metrics.gutter)
    }

    pub fn list_padding(&self) -> Pixels {
        px(self.metrics.list_padding)
    }

    pub fn row_height(&self) -> Pixels {
        px(self.metrics.row_height)
    }

    pub fn row_radius(&self) -> Pixels {
        px(self.metrics.row_radius)
    }

    pub fn row_inset(&self) -> Pixels {
        px(self.metrics.row_inset)
    }

    pub fn icon_size(&self) -> Pixels {
        px(self.metrics.icon_size)
    }

    pub fn icon_gap(&self) -> Pixels {
        px(self.metrics.icon_gap)
    }

    pub fn subtitle_gap(&self) -> Pixels {
        px(self.metrics.subtitle_gap)
    }

    pub fn section_height(&self) -> Pixels {
        px(self.metrics.section_height)
    }

    pub fn footer_height(&self) -> Pixels {
        px(self.metrics.footer_height)
    }

    pub fn keycap_height(&self) -> Pixels {
        px(self.metrics.keycap_height)
    }

    pub fn keycap_padding(&self) -> Pixels {
        px(self.metrics.keycap_padding)
    }

    pub fn keycap_radius(&self) -> Pixels {
        px(self.metrics.keycap_radius)
    }

    pub fn keycap_gap(&self) -> Pixels {
        px(self.metrics.keycap_gap)
    }

    pub fn text_query(&self) -> Pixels {
        px(self.metrics.text_query)
    }

    pub fn text_title(&self) -> Pixels {
        px(self.metrics.text_title)
    }

    pub fn text_subtitle(&self) -> Pixels {
        px(self.metrics.text_subtitle)
    }

    pub fn text_accessory(&self) -> Pixels {
        px(self.metrics.text_accessory)
    }

    pub fn text_section(&self) -> Pixels {
        px(self.metrics.text_section)
    }

    pub fn text_footer(&self) -> Pixels {
        px(self.metrics.text_footer)
    }

    pub fn text_card(&self) -> Pixels {
        px(self.metrics.text_card)
    }

    pub fn weight_query(&self) -> FontWeight {
        FontWeight(self.metrics.weight_query)
    }

    pub fn weight_title(&self) -> FontWeight {
        FontWeight(self.metrics.weight_title)
    }

    pub fn weight_section(&self) -> FontWeight {
        FontWeight(self.metrics.weight_section)
    }
}

/// Geometry tokens, pre-converted to `Pixels`.
///
/// Named rather than inlined so a render function reads `metrics::ROW_HEIGHT`
/// and not `px(44.)`, which is the literal I-5 forbids.
///
/// The constants, for the surfaces a style does not reach (7.41): the
/// action panel, the forms, the overlay and the settings window. The
/// launcher's own surface draws with [`Geometry`].
pub mod metrics {
    use super::*;

    pub fn gutter() -> Pixels {
        px(layout::GUTTER)
    }

    pub fn argument_pill_padding_x() -> Pixels {
        px(layout::argument::PILL_PADDING_X)
    }

    pub fn argument_pill_padding_y() -> Pixels {
        px(layout::argument::PILL_PADDING_Y)
    }

    pub fn argument_pill_radius() -> Pixels {
        px(layout::argument::PILL_RADIUS)
    }

    pub fn argument_pill_gap() -> Pixels {
        px(layout::argument::PILL_GAP)
    }
}

/// Type sizes, pre-converted.
pub mod type_scale {
    use super::*;

    pub fn heading() -> Pixels {
        px(layout::text::HEADING)
    }

    pub fn title() -> Pixels {
        px(layout::text::TITLE)
    }

    pub fn subtitle() -> Pixels {
        px(layout::text::SUBTITLE)
    }

    pub fn footer() -> Pixels {
        px(layout::text::FOOTER)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_opaque_colours() {
        let converted: Rgba = color(Color::rgb(255, 0, 0)).into();
        assert!((converted.r - 1.0).abs() < 0.01);
        assert!(converted.g < 0.01);
        assert!((converted.a - 1.0).abs() < 0.01);
    }

    /// Alpha has to survive the trip: a surface token that loses its alpha
    /// paints opaquely over the acrylic backdrop and F0 is wasted.
    #[test]
    fn preserves_alpha() {
        let converted: Rgba = color(Color::rgba(0, 0, 0, 128)).into();
        assert!((converted.a - 0.502).abs() < 0.01);
    }
}
