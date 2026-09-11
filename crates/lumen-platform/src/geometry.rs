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
    /// Place a window of `size` logical pixels within this monitor's work
    /// area, at the given fractions across and down.
    ///
    /// Both biases are "how far through the *free* space", not "where the
    /// centre goes": 0.0 is flush against the left or top edge, 1.0 flush
    /// against the right or bottom, 0.5 centred. That definition is what makes
    /// the window stay on screen at every value, whatever its size -- a centre
    /// coordinate at 1.0 would hang half the window off the edge.
    ///
    /// A launcher sits above the optical centre: 0.5 vertically reads as low,
    /// because the eye weights the top of the screen.
    pub fn place(&self, size: Rect, horizontal_bias: f32, vertical_bias: f32) -> Rect {
        // Clamped to the work area first. A window asked for at a size that does
        // not fit -- the settings editor on a small laptop, or any window on a
        // scaled display where the logical size multiplies up past the screen --
        // must be shrunk rather than hung off the bottom edge with its footer
        // and its buttons unreachable.
        let width =
            ((size.width as f32 * self.scale_factor).round() as i32).min(self.work_area.width);
        let height =
            ((size.height as f32 * self.scale_factor).round() as i32).min(self.work_area.height);

        // Clamped rather than trusted: these come from a file the user edits,
        // and a typed `5` instead of `0.5` must not put the launcher off the
        // side of the screen where it cannot be dismissed.
        let across = horizontal_bias.clamp(0.0, 1.0);
        let down = vertical_bias.clamp(0.0, 1.0);

        let free_x = (self.work_area.width - width).max(0);
        let free_y = (self.work_area.height - height).max(0);

        let x = self.work_area.x + (free_x as f32 * across).round() as i32;
        let y = self.work_area.y + (free_y as f32 * down).round() as i32;

        Rect::new(x, y, width, height)
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

        let placed = monitor.place(Rect::size(700, 400), 0.5, 0.25);

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

        let placed = monitor.place(Rect::size(700, 400), 0.5, 0.25);

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
        let placed = monitor.place(Rect::size(900, 620), 0.5, 0.5);

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

        let placed = monitor.place(Rect::size(900, 620), 0.5, 0.5);

        assert_eq!(placed.width, 900);
        assert_eq!(placed.height, 620);
    }

    /// The two extremes and the middle, which is the whole of what the setting
    /// promises: flush left, centred, flush right, and never off the screen.
    #[test]
    fn horizontal_bias_runs_from_one_edge_to_the_other() {
        let monitor = MonitorInfo {
            bounds: Rect::new(0, 0, 1920, 1080),
            work_area: Rect::new(0, 0, 1920, 1040),
            scale_factor: 1.0,
        };
        let size = Rect::size(720, 400);

        assert_eq!(monitor.place(size, 0.0, 0.0).x, 0);
        assert_eq!(monitor.place(size, 0.5, 0.0).x, (1920 - 720) / 2);
        assert_eq!(monitor.place(size, 1.0, 0.0).x, 1920 - 720);
    }

    #[test]
    fn vertical_bias_runs_from_one_edge_to_the_other() {
        let monitor = MonitorInfo {
            bounds: Rect::new(0, 0, 1920, 1080),
            work_area: Rect::new(0, 40, 1920, 1000),
            scale_factor: 1.0,
        };
        let size = Rect::size(720, 400);

        assert_eq!(monitor.place(size, 0.5, 0.0).y, 40);
        assert_eq!(monitor.place(size, 0.5, 1.0).y, 40 + 1000 - 400);
    }

    /// The setting comes from a file somebody edits by hand, so a `5` typed
    /// where `0.5` was meant must not put the launcher somewhere it cannot be
    /// reached or dismissed.
    #[test]
    fn a_bias_outside_the_range_is_clamped_rather_than_obeyed() {
        let monitor = MonitorInfo {
            bounds: Rect::new(0, 0, 1920, 1080),
            work_area: Rect::new(0, 0, 1920, 1040),
            scale_factor: 1.0,
        };
        let size = Rect::size(720, 400);

        let far = monitor.place(size, 5.0, 5.0);
        assert_eq!(far.x, 1920 - 720);
        assert_eq!(far.y, 1040 - 400);

        let negative = monitor.place(size, -3.0, -3.0);
        assert_eq!(negative.x, 0);
        assert_eq!(negative.y, 0);
    }

    /// Every position, on an awkward monitor, with a window that only just
    /// fits: the window stays entirely on the work area at all of them.
    #[test]
    fn no_bias_can_push_the_window_off_the_screen() {
        let monitor = MonitorInfo {
            bounds: Rect::new(-1920, 137, 1919, 1081),
            work_area: Rect::new(-1920, 137, 1919, 1037),
            scale_factor: 1.0,
        };
        let size = Rect::size(900, 700);

        for across in [0.0, 0.25, 0.5, 0.75, 1.0] {
            for down in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let placed = monitor.place(size, across, down);

                assert!(placed.x >= monitor.work_area.x, "{across} {down}");
                assert!(placed.y >= monitor.work_area.y, "{across} {down}");
                assert!(
                    placed.x + placed.width <= monitor.work_area.x + monitor.work_area.width,
                    "{across} {down}"
                );
                assert!(
                    placed.y + placed.height <= monitor.work_area.y + monitor.work_area.height,
                    "{across} {down}"
                );
            }
        }
    }

    #[test]
    fn place_never_lands_above_the_work_area() {
        let monitor = MonitorInfo {
            bounds: Rect::new(0, 0, 800, 600),
            work_area: Rect::new(0, 40, 800, 560),
            scale_factor: 1.0,
        };

        let placed = monitor.place(Rect::size(700, 600), 0.5, 0.25);

        assert!(placed.y >= monitor.work_area.y);
    }
}
