//! Bridging Lumen's tokens into GPUI's types.
//!
//! Every colour and every measurement the UI draws with passes through this
//! module. That is what makes I-5 -- no literal colour, spacing, radius or type
//! size in interface code -- checkable by reading the imports of a file rather
//! than by auditing every expression in it.

use gpui::{Hsla, Pixels, Rgba, px};
use lumen_core::layout;
use lumen_core::theme::Color;

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

/// Geometry tokens, pre-converted to `Pixels`.
///
/// Named rather than inlined so a render function reads `metrics::ROW_HEIGHT`
/// and not `px(44.)`, which is the literal I-5 forbids.
pub mod metrics {
    use super::*;

    pub fn row_height() -> Pixels {
        px(layout::ROW_HEIGHT)
    }

    pub fn search_height() -> Pixels {
        px(layout::SEARCH_HEIGHT)
    }

    pub fn footer_height() -> Pixels {
        px(layout::FOOTER_HEIGHT)
    }

    pub fn list_padding() -> Pixels {
        px(layout::LIST_PADDING)
    }

    pub fn gutter() -> Pixels {
        px(layout::GUTTER)
    }

    pub fn icon_size() -> Pixels {
        px(layout::ICON_SIZE)
    }

    pub fn icon_gap() -> Pixels {
        px(layout::ICON_GAP)
    }

    pub fn window_radius() -> Pixels {
        px(layout::WINDOW_RADIUS)
    }

    pub fn row_radius() -> Pixels {
        px(layout::ROW_RADIUS)
    }

    pub fn row_inset() -> Pixels {
        px(layout::ROW_INSET)
    }
}

/// Type sizes, pre-converted.
pub mod type_scale {
    use super::*;

    pub fn query() -> Pixels {
        px(layout::text::QUERY)
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
