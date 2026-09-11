//! Where a window goes when the user picks a layout (F3, window management).
//!
//! The arithmetic lives here, on the boundary, rather than in either platform
//! implementation. It is the same arithmetic on Windows and on Hyprland -- a
//! half is a half -- and it is the part that is easy to get subtly wrong, so it
//! is the part that should be tested without a compositor anywhere near it.
//!
//! # Why this is not in `lumen-core`
//!
//! `core` is where behaviour lives, and this looks like behaviour. But `core`
//! depends on this crate and not the other way round, and a layout is a value
//! the platform has to be handed. Putting the enum here keeps the direction of
//! the dependency honest; what a layout is *called* is `core`'s business and
//! lives with the provider that offers it.

use serde::{Deserialize, Serialize};

use crate::geometry::Rect;

/// One arrangement of the foreground window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WindowLayout {
    /// Fill the work area. Genuinely maximised, not a window the size of the
    /// screen: the two behave differently when the taskbar auto-hides and when
    /// the user double-clicks the title bar.
    Maximize,
    /// Centred and comfortably smaller than the screen -- the shape you want
    /// for reading something on a monitor too wide to fill.
    Center,

    LeftHalf,
    RightHalf,
    TopHalf,
    BottomHalf,

    TopLeftQuarter,
    TopRightQuarter,
    BottomLeftQuarter,
    BottomRightQuarter,

    LeftThird,
    MiddleThird,
    RightThird,
    LeftTwoThirds,
    RightTwoThirds,

    /// Move to the adjacent monitor, keeping the window's proportions.
    NextMonitor,
    PreviousMonitor,
}

/// How much of the work area the centred layout takes.
///
/// Wide enough to be a working window, narrow enough that the point of the
/// layout -- not filling a 34-inch monitor with one document -- survives.
const CENTER_WIDTH: f32 = 0.6;
const CENTER_HEIGHT: f32 = 0.85;

impl WindowLayout {
    /// Every layout, in the order they are offered.
    pub const ALL: &'static [WindowLayout] = &[
        WindowLayout::Maximize,
        WindowLayout::Center,
        WindowLayout::LeftHalf,
        WindowLayout::RightHalf,
        WindowLayout::TopHalf,
        WindowLayout::BottomHalf,
        WindowLayout::TopLeftQuarter,
        WindowLayout::TopRightQuarter,
        WindowLayout::BottomLeftQuarter,
        WindowLayout::BottomRightQuarter,
        WindowLayout::LeftThird,
        WindowLayout::MiddleThird,
        WindowLayout::RightThird,
        WindowLayout::LeftTwoThirds,
        WindowLayout::RightTwoThirds,
        WindowLayout::NextMonitor,
        WindowLayout::PreviousMonitor,
    ];

    /// Whether this layout moves the window to another monitor rather than
    /// placing it within the one it is already on.
    pub fn changes_monitor(self) -> bool {
        matches!(self, Self::NextMonitor | Self::PreviousMonitor)
    }

    /// The rectangle this layout occupies within `work_area`.
    ///
    /// `None` for the monitor moves, which are not a region of the monitor the
    /// window is currently on and so have nothing to compute here.
    ///
    /// [`WindowLayout::Maximize`] does return the whole work area even though
    /// the platform is expected to genuinely maximise instead: the rectangle is
    /// the right answer for a compositor that has no such concept, and a
    /// fallback for one that does when maximising is refused.
    pub fn tile(self, work_area: Rect) -> Option<Rect> {
        let Rect {
            x,
            y,
            width,
            height,
        } = work_area;

        // Edges rather than widths, and integer division done once per edge.
        // Computing each column as `width / 3` instead leaves a one- or
        // two-pixel seam between the thirds on most screen widths, which is
        // exactly the kind of detail that makes a tiling feature look
        // home-made.
        let (x0, x1, x2, x3) = (x, x + width / 3, x + 2 * width / 3, x + width);
        let (left, middle, right) = (x, x + width / 2, x + width);
        let (top, centre, bottom) = (y, y + height / 2, y + height);

        let rect = match self {
            Self::Maximize => work_area,
            Self::Center => {
                let w = (width as f32 * CENTER_WIDTH).round() as i32;
                let h = (height as f32 * CENTER_HEIGHT).round() as i32;
                Rect::new(x + (width - w) / 2, y + (height - h) / 2, w, h)
            }

            Self::LeftHalf => Rect::new(left, top, middle - left, height),
            Self::RightHalf => Rect::new(middle, top, right - middle, height),
            Self::TopHalf => Rect::new(left, top, width, centre - top),
            Self::BottomHalf => Rect::new(left, centre, width, bottom - centre),

            Self::TopLeftQuarter => Rect::new(left, top, middle - left, centre - top),
            Self::TopRightQuarter => Rect::new(middle, top, right - middle, centre - top),
            Self::BottomLeftQuarter => Rect::new(left, centre, middle - left, bottom - centre),
            Self::BottomRightQuarter => Rect::new(middle, centre, right - middle, bottom - centre),

            Self::LeftThird => Rect::new(x0, top, x1 - x0, height),
            Self::MiddleThird => Rect::new(x1, top, x2 - x1, height),
            Self::RightThird => Rect::new(x2, top, x3 - x2, height),
            Self::LeftTwoThirds => Rect::new(x0, top, x2 - x0, height),
            Self::RightTwoThirds => Rect::new(x1, top, x3 - x1, height),

            Self::NextMonitor | Self::PreviousMonitor => return None,
        };

        Some(rect)
    }
}

/// Carry a window's rectangle from one work area to another.
///
/// Proportional rather than absolute: the monitors may differ in size and in
/// scale factor, and a window that keeps its pixel dimensions moving from a
/// 4K display to a 1080p one arrives larger than the screen. Keeping its
/// *share* of the screen is what the user means by "put this over there".
///
/// The result is clamped so it cannot land partly off the destination.
pub fn carry_between(window: Rect, from: Rect, to: Rect) -> Rect {
    if from.width <= 0 || from.height <= 0 {
        return to;
    }

    let fraction = |value: i32, size: i32| value as f32 / size as f32;

    let width = (fraction(window.width, from.width) * to.width as f32).round() as i32;
    let height = (fraction(window.height, from.height) * to.height as f32).round() as i32;
    let width = width.clamp(1, to.width);
    let height = height.clamp(1, to.height);

    let x = to.x + (fraction(window.x - from.x, from.width) * to.width as f32).round() as i32;
    let y = to.y + (fraction(window.y - from.y, from.height) * to.height as f32).round() as i32;

    Rect::new(
        x.clamp(to.x, to.x + to.width - width),
        y.clamp(to.y, to.y + to.height - height),
        width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A work area deliberately not divisible by two or three, because that is
    /// where seams and overhangs come from.
    fn awkward() -> Rect {
        Rect::new(7, 11, 1919, 1037)
    }

    fn right_edge(rect: Rect) -> i32 {
        rect.x + rect.width
    }

    fn bottom_edge(rect: Rect) -> i32 {
        rect.y + rect.height
    }

    #[test]
    fn every_layout_that_tiles_stays_inside_the_work_area() {
        let area = awkward();
        for layout in WindowLayout::ALL {
            let Some(rect) = layout.tile(area) else {
                assert!(layout.changes_monitor());
                continue;
            };

            assert!(rect.x >= area.x, "{layout:?} starts left of the work area");
            assert!(rect.y >= area.y, "{layout:?} starts above the work area");
            assert!(
                right_edge(rect) <= right_edge(area),
                "{layout:?} runs off the right"
            );
            assert!(
                bottom_edge(rect) <= bottom_edge(area),
                "{layout:?} runs off the bottom"
            );
            assert!(rect.width > 0 && rect.height > 0, "{layout:?} is empty");
        }
    }

    /// The halves must meet exactly. A seam is one pixel of desktop showing
    /// between two windows, and it is visible.
    #[test]
    fn the_halves_meet_with_no_seam_and_no_overlap() {
        let area = awkward();
        let left = WindowLayout::LeftHalf.tile(area).unwrap();
        let right = WindowLayout::RightHalf.tile(area).unwrap();

        assert_eq!(right_edge(left), right.x);
        assert_eq!(left.x, area.x);
        assert_eq!(right_edge(right), right_edge(area));
    }

    #[test]
    fn the_thirds_meet_with_no_seam_and_cover_the_width() {
        let area = awkward();
        let one = WindowLayout::LeftThird.tile(area).unwrap();
        let two = WindowLayout::MiddleThird.tile(area).unwrap();
        let three = WindowLayout::RightThird.tile(area).unwrap();

        assert_eq!(right_edge(one), two.x);
        assert_eq!(right_edge(two), three.x);
        assert_eq!(one.x, area.x);
        assert_eq!(right_edge(three), right_edge(area));
    }

    /// Two thirds plus the remaining third is the whole width, from either end.
    #[test]
    fn the_two_thirds_layouts_complement_the_thirds() {
        let area = awkward();

        let left_two = WindowLayout::LeftTwoThirds.tile(area).unwrap();
        let right_one = WindowLayout::RightThird.tile(area).unwrap();
        assert_eq!(right_edge(left_two), right_one.x);

        let right_two = WindowLayout::RightTwoThirds.tile(area).unwrap();
        let left_one = WindowLayout::LeftThird.tile(area).unwrap();
        assert_eq!(right_edge(left_one), right_two.x);
    }

    #[test]
    fn the_quarters_tile_the_work_area_exactly() {
        let area = awkward();
        let quarters = [
            WindowLayout::TopLeftQuarter,
            WindowLayout::TopRightQuarter,
            WindowLayout::BottomLeftQuarter,
            WindowLayout::BottomRightQuarter,
        ]
        .map(|layout| layout.tile(area).unwrap());

        let covered: i64 = quarters
            .iter()
            .map(|rect| rect.width as i64 * rect.height as i64)
            .sum();
        assert_eq!(covered, area.width as i64 * area.height as i64);
    }

    #[test]
    fn maximize_is_the_whole_work_area() {
        let area = awkward();
        assert_eq!(WindowLayout::Maximize.tile(area), Some(area));
    }

    #[test]
    fn center_is_smaller_than_the_screen_and_actually_centred() {
        let area = Rect::new(0, 0, 2000, 1000);
        let rect = WindowLayout::Center.tile(area).unwrap();

        assert!(rect.width < area.width && rect.height < area.height);
        assert_eq!(rect.x + rect.width / 2, area.center_x());
        assert_eq!(rect.y + rect.height / 2, area.center_y());
    }

    #[test]
    fn the_monitor_moves_have_no_tile() {
        assert_eq!(WindowLayout::NextMonitor.tile(awkward()), None);
        assert_eq!(WindowLayout::PreviousMonitor.tile(awkward()), None);
        assert!(WindowLayout::NextMonitor.changes_monitor());
        assert!(!WindowLayout::LeftHalf.changes_monitor());
    }

    /// Moving between monitors keeps the window's share of the screen, not its
    /// pixel size: a window that filled half a 4K display must not arrive on a
    /// 1080p one larger than the screen.
    #[test]
    fn carrying_a_window_to_a_smaller_monitor_keeps_its_share() {
        let from = Rect::new(0, 0, 3840, 2160);
        let to = Rect::new(3840, 0, 1920, 1080);

        let carried = carry_between(Rect::new(0, 0, 1920, 2160), from, to);

        assert_eq!(carried.width, 960);
        assert_eq!(carried.height, 1080);
        assert_eq!(carried.x, 3840);
    }

    #[test]
    fn a_carried_window_never_lands_off_the_destination() {
        let from = Rect::new(0, 0, 1920, 1080);
        let to = Rect::new(1920, 100, 1280, 720);

        // Hanging off the right of its own monitor to begin with.
        let carried = carry_between(Rect::new(1800, 1000, 400, 300), from, to);

        assert!(carried.x >= to.x);
        assert!(carried.y >= to.y);
        assert!(right_edge(carried) <= right_edge(to));
        assert!(bottom_edge(carried) <= bottom_edge(to));
    }

    /// A degenerate source (a monitor that has just been disconnected) must not
    /// divide by zero or produce a rectangle nobody can see.
    #[test]
    fn carrying_from_an_empty_work_area_falls_back_to_filling_the_destination() {
        let to = Rect::new(0, 0, 1280, 720);
        assert_eq!(
            carry_between(Rect::new(0, 0, 10, 10), Rect::default(), to),
            to
        );
    }
}
