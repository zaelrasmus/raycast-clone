//! Geometry types that cross the platform boundary.
//!
//! Deliberately independent of GPUI's geometry: `core` and `platform` must not
//! depend on the presentation layer (AD-03).

/// A rectangle in physical screen coordinates, origin top-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn size(width: i32, height: i32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    pub const fn center_x(&self) -> i32 {
        self.x + self.width / 2
    }

    pub const fn center_y(&self) -> i32 {
        self.y + self.height / 2
    }
}

/// A monitor, with the scale factor that applies to it.
///
/// `work_area` excludes the taskbar; the launcher is positioned against it
/// rather than against the full bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorInfo {
    pub bounds: Rect,
    pub work_area: Rect,
    /// 1.0 at 96 DPI, 1.5 at 144 DPI, and so on. Per-monitor, never global:
    /// a dual-monitor setup with differing scale factors is an F0 acceptance
    /// criterion.
    pub scale_factor: f32,
}

impl MonitorInfo {
    /// Place a window of `size` logical pixels horizontally centred on this
    /// monitor and vertically at `vertical_bias` of the work area.
    ///
    /// A launcher sits above the optical centre: 0.5 reads as low, because the
    /// eye weights the top of the screen.
    pub fn place(&self, size: Rect, vertical_bias: f32) -> Rect {
        // Clamped to the work area first. A window asked for at a size that does
        // not fit -- the settings editor on a small laptop, or any window on a
        // scaled display where the logical size multiplies up past the screen --
        // must be shrunk rather than hung off the bottom edge with its footer
        // and its buttons unreachable.
        let width =
            ((size.width as f32 * self.scale_factor).round() as i32).min(self.work_area.width);
        let height =
            ((size.height as f32 * self.scale_factor).round() as i32).min(self.work_area.height);

        let x = self.work_area.x + (self.work_area.width - width) / 2;
        let y = self.work_area.y
            + ((self.work_area.height - height) as f32 * vertical_bias).round() as i32;

        Rect::new(x, y.max(self.work_area.y), width, height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_centres_horizontally_and_biases_vertically() {
        let monitor = MonitorInfo {
            bounds: Rect::new(0, 0, 1920, 1080),
            work_area: Rect::new(0, 0, 1920, 1040),
            scale_factor: 1.0,
        };

        let placed = monitor.place(Rect::size(700, 400), 0.25);

        assert_eq!(placed.x, (1920 - 700) / 2);
        assert_eq!(placed.y, ((1040 - 400) as f32 * 0.25).round() as i32);
        assert_eq!(placed.width, 700);
    }

    #[test]
    fn place_scales_logical_size_by_monitor_scale_factor() {
        let monitor = MonitorInfo {
            bounds: Rect::new(1920, 0, 2560, 1440),
            work_area: Rect::new(1920, 0, 2560, 1400),
            scale_factor: 1.5,
        };

        let placed = monitor.place(Rect::size(700, 400), 0.25);

        assert_eq!(placed.width, 1050);
        assert_eq!(placed.height, 600);
        // Still inside the monitor it was placed on, not the primary one.
        assert!(placed.x >= monitor.work_area.x);
    }

    /// A window larger than the screen must be shrunk to fit, not left hanging
    /// off the edge with its footer unreachable.
    #[test]
    fn place_clamps_a_window_too_large_for_the_work_area() {
        let monitor = MonitorInfo {
            bounds: Rect::new(0, 0, 1280, 800),
            work_area: Rect::new(0, 0, 1280, 760),
            scale_factor: 1.5,
        };

        // 900 x 620 logical is 1350 x 930 physical: wider and taller than this.
        let placed = monitor.place(Rect::size(900, 620), 0.5);

        assert!(placed.width <= monitor.work_area.width);
        assert!(placed.height <= monitor.work_area.height);
        assert!(placed.x >= monitor.work_area.x);
        assert!(placed.y >= monitor.work_area.y);
        assert!(placed.y + placed.height <= monitor.work_area.y + monitor.work_area.height);
    }

    #[test]
    fn place_leaves_a_window_that_already_fits_alone() {
        let monitor = MonitorInfo {
            bounds: Rect::new(0, 0, 1920, 1080),
            work_area: Rect::new(0, 0, 1920, 1040),
            scale_factor: 1.0,
        };

        let placed = monitor.place(Rect::size(900, 620), 0.5);

        assert_eq!(placed.width, 900);
        assert_eq!(placed.height, 620);
    }

    #[test]
    fn place_never_lands_above_the_work_area() {
        let monitor = MonitorInfo {
            bounds: Rect::new(0, 0, 800, 600),
            work_area: Rect::new(0, 40, 800, 560),
            scale_factor: 1.0,
        };

        let placed = monitor.place(Rect::size(700, 600), 0.25);

        assert!(placed.y >= monitor.work_area.y);
    }
}
