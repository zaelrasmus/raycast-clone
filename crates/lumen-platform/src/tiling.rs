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

    /// Full work-area height, keeping the window's horizontal position and
    /// width.
    MaximizeHeight,
    /// Full work-area width, keeping the window's vertical position and
    /// height.
    MaximizeWidth,
    /// Nine tenths of the work area, centred: nearly maximised, with the
    /// desktop still showing round the edge.
    AlmostMaximize,
    /// Three fifths of the work area, centred, but never larger than a
    /// comfortable window: 1025 by 900 logical pixels at most.
    ReasonableSize,
    /// Centred at its current size. [`WindowLayout::Center`] resizes; this
    /// only moves.
    CenterKeepSize,

    FirstFourth,
    SecondFourth,
    ThirdFourth,
    LastFourth,

    TopLeftSixth,
    TopCenterSixth,
    TopRightSixth,
    BottomLeftSixth,
    BottomCenterSixth,
    BottomRightSixth,

    /// Against that edge of the work area at the current size, keeping the
    /// other axis where it is.
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,

    /// Move to the adjacent monitor, keeping the window's proportions.
    NextMonitor,
    PreviousMonitor,

    /// Minimise, from any state: maximised and snapped included, in one
    /// command where the system's own key takes two from maximised.
    Minimize,
    /// Minimise every ordinary window on the target window's monitor, or on
    /// the cursor's when nothing is in front. The other monitors are left
    /// alone.
    MinimizeDisplay,
    /// Put the window back where it was before Lumen last arranged or
    /// minimised it; with nothing to go back to, the window Lumen minimised
    /// most recently. See [`crate::window_commands::History`].
    Restore,
    /// Focus the application's next window, or its previous one, among its
    /// switchable windows in z-order. See [`crate::window_commands::cycle`].
    FocusNextAppWindow,
    FocusPreviousAppWindow,
}

/// What a [`WindowLayout`] does to the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutKind {
    /// Puts it somewhere on its monitor: [`WindowLayout::place`] answers.
    Place,
    /// Carries it to another monitor.
    Monitor,
    /// Minimises, restores or focuses: a window command, not a region.
    Command,
}

/// Space left round the tiles, in physical pixels (F17, 7.58).
///
/// Physical because the work area is: [`WindowSettings::gaps`] scales the
/// configured logical figures by the monitor's scale factor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Gaps {
    /// Between two adjacent tiles.
    pub window: i32,
    /// Between a tile and the edge of the work area.
    pub screen: i32,
}

/// What repeating a half does to the window already there (F17, 7.58).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Cycling {
    /// A half is always a half.
    Off,
    /// ½, then ⅔, then ⅓, then ½ again.
    #[default]
    Sizes,
    /// On to the facing half of the next monitor that way.
    Displays,
}

/// The window management settings, as the platform holds them (F17).
///
/// Logical pixels, as the user wrote them; [`WindowSettings::gaps`] gives
/// them for one monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowSettings {
    pub gap: u32,
    pub screen_gap: u32,
    pub cycling: Cycling,
}

impl WindowSettings {
    /// The gaps in physical pixels on a monitor whose scale factor is `scale`.
    pub fn gaps(&self, scale: f32) -> Gaps {
        let physical = |logical: u32| (logical as f32 * scale).round() as i32;
        Gaps {
            window: physical(self.gap),
            screen: physical(self.screen_gap),
        }
    }
}

/// How much of the work area a repeated half takes along its axis (7.58).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Share {
    #[default]
    Half,
    TwoThirds,
    OneThird,
}

impl Share {
    /// The next size in the cycle: ½, ⅔, ⅓, and round again.
    pub fn next(self) -> Self {
        match self {
            Self::Half => Self::TwoThirds,
            Self::TwoThirds => Self::OneThird,
            Self::OneThird => Self::Half,
        }
    }

    fn of(self) -> Frac {
        match self {
            Self::Half => Frac(1, 2),
            Self::TwoThirds => Frac(2, 3),
            Self::OneThird => Frac(1, 3),
        }
    }
}

/// A fraction of the work area along one axis: numerator, denominator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frac(pub i32, pub i32);

impl Frac {
    const ZERO: Frac = Frac(0, 1);
    const ONE: Frac = Frac(1, 1);

    fn complement(self) -> Self {
        Frac(self.1 - self.0, self.1)
    }

    /// Where this falls on an axis that starts at `origin` and is `size`
    /// long. Integer division once per edge, so neighbours share the pixel.
    fn at(self, origin: i32, size: i32) -> i32 {
        origin + size * self.0 / self.1
    }

    /// Whether this is an edge of the work area rather than between tiles.
    fn outer(self) -> bool {
        self.0 == 0 || self.0 == self.1
    }
}

/// A tiling layout as a region of the work area, each edge a fraction of it.
///
/// The one description gaps are applied to: an edge at 0 or 1 is on the
/// work area's edge and is inset by the screen gap; an edge anywhere else is
/// shared with a neighbour and is inset by half the window gap, so two
/// neighbours end up exactly one window gap apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tile {
    pub left: Frac,
    pub top: Frac,
    pub right: Frac,
    pub bottom: Frac,
}

impl Tile {
    const fn new(left: Frac, top: Frac, right: Frac, bottom: Frac) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// This tile on `work_area`, with `gaps` round it.
    pub fn rect(self, work_area: Rect, gaps: Gaps) -> Rect {
        let Rect {
            x,
            y,
            width,
            height,
        } = work_area;
        // The leading side of an interior gap gets the odd pixel, so the two
        // halves always add up to the whole gap.
        let start = |edge: Frac, origin: i32, size: i32| {
            let at = edge.at(origin, size);
            match edge.outer() {
                true => at + gaps.screen,
                false => at + gaps.window - gaps.window / 2,
            }
        };
        let end = |edge: Frac, origin: i32, size: i32| {
            let at = edge.at(origin, size);
            match edge.outer() {
                true => at - gaps.screen,
                false => at - gaps.window / 2,
            }
        };

        let left = start(self.left, x, width);
        let top = start(self.top, y, height);
        let right = end(self.right, x, width);
        let bottom = end(self.bottom, y, height);
        Rect::new(left, top, (right - left).max(1), (bottom - top).max(1))
    }
}

/// `area` with `inset` taken off every side, never smaller than a pixel.
fn inset(area: Rect, inset: i32) -> Rect {
    let inset = inset.clamp(0, (area.width.min(area.height) - 1).max(0) / 2);
    Rect::new(
        area.x + inset,
        area.y + inset,
        area.width - 2 * inset,
        area.height - 2 * inset,
    )
}

/// How much of the work area the centred layout takes.
///
/// Wide enough to be a working window, narrow enough that the point of the
/// layout -- not filling a 34-inch monitor with one document -- survives.
const CENTER_WIDTH: f32 = 0.6;
const CENTER_HEIGHT: f32 = 0.85;

/// How much of the work area "Almost Maximise" takes, on both axes.
const ALMOST_MAXIMIZE: f32 = 0.9;

/// How much of the work area "Reasonable Size" takes, before the cap.
const REASONABLE_SHARE: f32 = 0.6;

/// The largest "Reasonable Size" gets, in logical pixels (Raycast's figure).
///
/// Logical, so that the window is the same size to the eye on a 4K monitor at
/// 150% as on a 1440p one at 100%. Multiplied by the monitor's scale factor
/// before it is compared with anything, since the work area is physical.
const REASONABLE_MAX: (f32, f32) = (1025.0, 900.0);

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
        WindowLayout::MaximizeHeight,
        WindowLayout::MaximizeWidth,
        WindowLayout::AlmostMaximize,
        WindowLayout::ReasonableSize,
        WindowLayout::CenterKeepSize,
        WindowLayout::FirstFourth,
        WindowLayout::SecondFourth,
        WindowLayout::ThirdFourth,
        WindowLayout::LastFourth,
        WindowLayout::TopLeftSixth,
        WindowLayout::TopCenterSixth,
        WindowLayout::TopRightSixth,
        WindowLayout::BottomLeftSixth,
        WindowLayout::BottomCenterSixth,
        WindowLayout::BottomRightSixth,
        WindowLayout::MoveUp,
        WindowLayout::MoveDown,
        WindowLayout::MoveLeft,
        WindowLayout::MoveRight,
        WindowLayout::NextMonitor,
        WindowLayout::PreviousMonitor,
        WindowLayout::Minimize,
        WindowLayout::MinimizeDisplay,
        WindowLayout::Restore,
        WindowLayout::FocusNextAppWindow,
        WindowLayout::FocusPreviousAppWindow,
    ];

    /// What this layout does to the window.
    pub fn kind(self) -> LayoutKind {
        match self {
            Self::NextMonitor | Self::PreviousMonitor => LayoutKind::Monitor,
            Self::Minimize
            | Self::MinimizeDisplay
            | Self::Restore
            | Self::FocusNextAppWindow
            | Self::FocusPreviousAppWindow => LayoutKind::Command,
            _ => LayoutKind::Place,
        }
    }

    /// Whether this layout moves the window to another monitor rather than
    /// placing it within the one it is already on.
    pub fn changes_monitor(self) -> bool {
        self.kind() == LayoutKind::Monitor
    }

    /// Whether there is nothing to do without a target window.
    ///
    /// Two commands choose their own windows: *Minimise on This Display*
    /// falls back to the monitor under the cursor, and *Restore* to the
    /// window Lumen minimised last -- which is never the one in front.
    pub fn needs_window(self) -> bool {
        !matches!(self, Self::MinimizeDisplay | Self::Restore)
    }

    /// Where this layout puts a window that is now at `current`, on a monitor
    /// whose work area is `work_area` and whose scale factor is `scale`.
    ///
    /// Every rectangle is in physical pixels, and `current` is the window's
    /// visible rectangle, not its outer one. Most layouts ignore `current` and
    /// `scale`; the ones that keep part of the window's geometry (the
    /// maximised axes, centring without resizing, the moves) read it, and
    /// "Reasonable Size" needs the scale for its cap. The window's size is
    /// clamped to the work area and its position brought inside it, so the
    /// result never hangs off the monitor even when the window already did.
    ///
    /// `None` for the monitor moves, which are not a region of the monitor the
    /// window is currently on and so have nothing to compute here, and for the
    /// window commands ([`LayoutKind::Command`]), which are not a region at
    /// all.
    ///
    /// [`WindowLayout::Maximize`] does return the whole work area even though
    /// the platform is expected to genuinely maximise instead: the rectangle is
    /// the right answer for a compositor that has no such concept, and a
    /// fallback for one that does when maximising is refused. With a screen
    /// gap it is the work area inset by it, and the platform places that
    /// rather than maximising: a maximised window cannot have a margin (7.58).
    ///
    /// The tiles ([`WindowLayout::tile`]) get `gaps` round them. Every other
    /// placement is worked out in the work area inset by the screen gap.
    pub fn place(self, work_area: Rect, current: Rect, scale: f32, gaps: Gaps) -> Option<Rect> {
        self.place_at(Share::Half, work_area, current, scale, gaps)
    }

    /// [`WindowLayout::place`], with a half resized to `share` along its axis
    /// when it is cycled (7.58). Any other layout ignores `share`.
    pub fn place_at(
        self,
        share: Share,
        work_area: Rect,
        current: Rect,
        scale: f32,
        gaps: Gaps,
    ) -> Option<Rect> {
        if let Some(tile) = self.tile_at(share) {
            return Some(tile.rect(work_area, gaps));
        }

        let work_area = inset(work_area, gaps.screen);
        let Rect {
            x,
            y,
            width,
            height,
        } = work_area;
        let (left, right) = (x, x + width);
        let (top, bottom) = (y, y + height);

        // The window's own size, no larger than the work area, and where it
        // can sit on each axis without hanging off.
        let w = current.width.clamp(1, width.max(1));
        let h = current.height.clamp(1, height.max(1));
        let keep_x = current.x.clamp(left, (right - w).max(left));
        let keep_y = current.y.clamp(top, (bottom - h).max(top));

        let centred = |w: i32, h: i32| Rect::new(x + (width - w) / 2, y + (height - h) / 2, w, h);
        let share = |size: i32, fraction: f32| (size as f32 * fraction).round() as i32;

        let rect = match self {
            Self::Center => centred(share(width, CENTER_WIDTH), share(height, CENTER_HEIGHT)),
            Self::MaximizeHeight => Rect::new(keep_x, top, w, height),
            Self::MaximizeWidth => Rect::new(left, keep_y, width, h),
            Self::AlmostMaximize => centred(
                share(width, ALMOST_MAXIMIZE),
                share(height, ALMOST_MAXIMIZE),
            ),
            Self::ReasonableSize => {
                let cap_w = (REASONABLE_MAX.0 * scale).round() as i32;
                let cap_h = (REASONABLE_MAX.1 * scale).round() as i32;
                centred(
                    share(width, REASONABLE_SHARE).min(cap_w).max(1),
                    share(height, REASONABLE_SHARE).min(cap_h).max(1),
                )
            }
            Self::CenterKeepSize => centred(w, h),

            Self::MoveUp => Rect::new(keep_x, top, w, h),
            Self::MoveDown => Rect::new(keep_x, bottom - h, w, h),
            Self::MoveLeft => Rect::new(left, keep_y, w, h),
            Self::MoveRight => Rect::new(right - w, keep_y, w, h),

            _ => return None,
        };

        Some(rect)
    }

    /// This layout as a tile, when it is one: the maximise, the halves,
    /// quarters, thirds, two-thirds, fourths and sixths.
    ///
    /// Edges rather than widths, each worked out once from the work area.
    /// Computing each column as `width / 3` instead leaves a one- or
    /// two-pixel seam between the thirds on most screen widths, which is
    /// exactly the kind of detail that makes a tiling feature look home-made.
    pub fn tile(self) -> Option<Tile> {
        self.tile_at(Share::Half)
    }

    fn tile_at(self, share: Share) -> Option<Tile> {
        let (zero, one, half) = (Frac::ZERO, Frac::ONE, Frac(1, 2));
        let (third, two_thirds) = (Frac(1, 3), Frac(2, 3));
        let (fourth, three_fourths) = (Frac(1, 4), Frac(3, 4));
        let (near, far) = (share.of(), share.of().complement());
        let tile = Tile::new;

        Some(match self {
            Self::Maximize => tile(zero, zero, one, one),

            Self::LeftHalf => tile(zero, zero, near, one),
            Self::RightHalf => tile(far, zero, one, one),
            Self::TopHalf => tile(zero, zero, one, near),
            Self::BottomHalf => tile(zero, far, one, one),

            Self::TopLeftQuarter => tile(zero, zero, half, half),
            Self::TopRightQuarter => tile(half, zero, one, half),
            Self::BottomLeftQuarter => tile(zero, half, half, one),
            Self::BottomRightQuarter => tile(half, half, one, one),

            Self::LeftThird => tile(zero, zero, third, one),
            Self::MiddleThird => tile(third, zero, two_thirds, one),
            Self::RightThird => tile(two_thirds, zero, one, one),
            Self::LeftTwoThirds => tile(zero, zero, two_thirds, one),
            Self::RightTwoThirds => tile(third, zero, one, one),

            Self::FirstFourth => tile(zero, zero, fourth, one),
            Self::SecondFourth => tile(fourth, zero, half, one),
            Self::ThirdFourth => tile(half, zero, three_fourths, one),
            Self::LastFourth => tile(three_fourths, zero, one, one),

            // Three columns on the thirds' edges, two rows on the halves'.
            Self::TopLeftSixth => tile(zero, zero, third, half),
            Self::TopCenterSixth => tile(third, zero, two_thirds, half),
            Self::TopRightSixth => tile(two_thirds, zero, one, half),
            Self::BottomLeftSixth => tile(zero, half, third, one),
            Self::BottomCenterSixth => tile(third, half, two_thirds, one),
            Self::BottomRightSixth => tile(two_thirds, half, one, one),

            _ => return None,
        })
    }

    /// Whether this is one of the four halves, which cycle (7.58).
    pub fn is_half(self) -> bool {
        matches!(
            self,
            Self::LeftHalf | Self::RightHalf | Self::TopHalf | Self::BottomHalf
        )
    }

    /// The half facing this one: left for right, top for bottom.
    pub fn opposite_half(self) -> Option<Self> {
        Some(match self {
            Self::LeftHalf => Self::RightHalf,
            Self::RightHalf => Self::LeftHalf,
            Self::TopHalf => Self::BottomHalf,
            Self::BottomHalf => Self::TopHalf,
            _ => return None,
        })
    }
}

/// The monitor next to `current` on the side a half points to: left of it
/// for [`WindowLayout::LeftHalf`], above it for [`WindowLayout::TopHalf`].
///
/// Among `monitors` (full bounds), one entirely on that side, the nearest,
/// and of those the one most in line with `current`. `None` when there is
/// none, or `toward` is not a half.
pub fn monitor_toward(monitors: &[Rect], current: Rect, toward: WindowLayout) -> Option<usize> {
    let right = |r: &Rect| r.x + r.width;
    let bottom = |r: &Rect| r.y + r.height;
    // How far beyond `current` a monitor is, when it is on that side, and how
    // far out of line with it.
    let distance = |m: &Rect| -> Option<(i32, i32)> {
        let across = (m.center_y() - current.center_y()).abs();
        let down = (m.center_x() - current.center_x()).abs();
        match toward {
            WindowLayout::LeftHalf => {
                (right(m) <= current.x).then(|| (current.x - right(m), across))
            }
            WindowLayout::RightHalf => {
                (m.x >= right(&current)).then(|| (m.x - right(&current), across))
            }
            WindowLayout::TopHalf => {
                (bottom(m) <= current.y).then(|| (current.y - bottom(m), down))
            }
            WindowLayout::BottomHalf => {
                (m.y >= bottom(&current)).then(|| (m.y - bottom(&current), down))
            }
            _ => None,
        }
    };
    monitors
        .iter()
        .enumerate()
        .filter_map(|(index, m)| distance(m).map(|d| (d, index)))
        .min()
        .map(|(_, index)| index)
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

    /// An ordinary window, inside the awkward work area.
    fn window() -> Rect {
        Rect::new(300, 200, 800, 600)
    }

    fn at(layout: WindowLayout, area: Rect, current: Rect) -> Rect {
        layout.place(area, current, 1.0, NONE).unwrap()
    }

    fn right_edge(rect: Rect) -> i32 {
        rect.x + rect.width
    }

    fn bottom_edge(rect: Rect) -> i32 {
        rect.y + rect.height
    }

    const NONE: Gaps = Gaps {
        window: 0,
        screen: 0,
    };

    fn tiles() -> Vec<(WindowLayout, Tile)> {
        WindowLayout::ALL
            .iter()
            .filter_map(|&layout| layout.tile().map(|tile| (layout, tile)))
            .collect()
    }

    /// Every pair of tiles that share an edge, with the gap between them in
    /// pixels: horizontal neighbours, then vertical ones.
    fn neighbour_gaps(area: Rect, gaps: Gaps) -> Vec<i32> {
        let all = tiles();
        let mut found = Vec::new();
        for &(_, a) in &all {
            for &(_, b) in &all {
                let (ra, rb) = (a.rect(area, gaps), b.rect(area, gaps));
                let rows_meet = a.top == b.top && a.bottom == b.bottom;
                let columns_meet = a.left == b.left && a.right == b.right;
                let same = |p: Frac, q: Frac| p.0 * q.1 == q.0 * p.1;
                if rows_meet && same(a.right, b.left) {
                    found.push(rb.x - right_edge(ra));
                }
                if columns_meet && same(a.bottom, b.top) {
                    found.push(rb.y - bottom_edge(ra));
                }
            }
        }
        found
    }

    #[test]
    fn every_tile_is_a_region_of_the_unit_square() {
        let untiled = [
            WindowLayout::Center,
            WindowLayout::MaximizeHeight,
            WindowLayout::MaximizeWidth,
            WindowLayout::AlmostMaximize,
            WindowLayout::ReasonableSize,
            WindowLayout::CenterKeepSize,
            WindowLayout::MoveUp,
            WindowLayout::MoveDown,
            WindowLayout::MoveLeft,
            WindowLayout::MoveRight,
        ];
        for &layout in WindowLayout::ALL {
            let tiled = layout.kind() == LayoutKind::Place && !untiled.contains(&layout);
            assert_eq!(layout.tile().is_some(), tiled, "{layout:?}");
            let Some(tile) = layout.tile() else { continue };
            let value = |f: Frac| f.0 as f32 / f.1 as f32;
            for edge in [tile.left, tile.top, tile.right, tile.bottom] {
                assert!((0.0..=1.0).contains(&value(edge)), "{layout:?}");
            }
            assert!(value(tile.left) < value(tile.right), "{layout:?}");
            assert!(value(tile.top) < value(tile.bottom), "{layout:?}");
        }
    }

    #[test]
    fn without_gaps_every_pair_of_neighbouring_tiles_abuts() {
        let gaps = neighbour_gaps(awkward(), NONE);
        assert!(gaps.len() > 20);
        assert!(gaps.iter().all(|&gap| gap == 0), "{gaps:?}");
    }

    /// Odd on purpose: the odd pixel must go to one side, not be lost.
    #[test]
    fn with_a_window_gap_every_pair_of_neighbours_is_exactly_that_far_apart() {
        let gaps = neighbour_gaps(
            awkward(),
            Gaps {
                window: 7,
                screen: 5,
            },
        );
        assert!(gaps.iter().all(|&gap| gap == 7), "{gaps:?}");
    }

    #[test]
    fn the_screen_gap_is_left_at_every_outer_edge() {
        let area = awkward();
        let gaps = Gaps {
            window: 8,
            screen: 12,
        };
        for (layout, tile) in tiles() {
            let rect = tile.rect(area, gaps);
            if tile.left.outer() {
                assert_eq!(rect.x, area.x + 12, "{layout:?}");
            }
            if tile.top.outer() {
                assert_eq!(rect.y, area.y + 12, "{layout:?}");
            }
            if tile.right.outer() {
                assert_eq!(right_edge(rect), right_edge(area) - 12, "{layout:?}");
            }
            if tile.bottom.outer() {
                assert_eq!(bottom_edge(rect), bottom_edge(area) - 12, "{layout:?}");
            }
        }
    }

    #[test]
    fn the_gaps_are_logical_pixels_scaled_to_the_monitor() {
        let settings = WindowSettings {
            gap: 8,
            screen_gap: 4,
            cycling: Cycling::Sizes,
        };
        assert_eq!(
            settings.gaps(1.5),
            Gaps {
                window: 12,
                screen: 6
            }
        );
        assert_eq!(
            settings.gaps(1.0),
            Gaps {
                window: 8,
                screen: 4
            }
        );
    }

    /// A maximised window cannot have a margin: with a screen gap, Maximise
    /// is the work area inset by it, which the platform places (7.58).
    #[test]
    fn maximise_with_a_screen_gap_is_the_inset_work_area() {
        let area = Rect::new(0, 0, 1920, 1040);
        let gaps = Gaps {
            window: 16,
            screen: 10,
        };
        assert_eq!(
            WindowLayout::Maximize.place(area, window(), 1.0, gaps),
            Some(Rect::new(10, 10, 1900, 1020))
        );
    }

    #[test]
    fn the_untiled_layouts_stay_inside_the_screen_gap() {
        let area = awkward();
        let inner = inset(area, 24);
        let gaps = Gaps {
            window: 0,
            screen: 24,
        };
        let huge = Rect::new(-500, 900, 4000, 3000);
        for &layout in WindowLayout::ALL {
            for current in [window(), huge] {
                let Some(rect) = layout.place(area, current, 1.0, gaps) else {
                    continue;
                };
                assert!(rect.x >= inner.x && rect.y >= inner.y, "{layout:?}");
                assert!(right_edge(rect) <= right_edge(inner), "{layout:?}");
                assert!(bottom_edge(rect) <= bottom_edge(inner), "{layout:?}");
            }
        }
    }

    #[test]
    fn a_cycled_half_takes_its_share_from_its_own_edge() {
        let area = Rect::new(0, 0, 1800, 900);
        let at = |layout: WindowLayout, share| {
            layout.place_at(share, area, window(), 1.0, NONE).unwrap()
        };
        assert_eq!(
            at(WindowLayout::LeftHalf, Share::TwoThirds),
            Rect::new(0, 0, 1200, 900)
        );
        assert_eq!(
            at(WindowLayout::LeftHalf, Share::OneThird),
            Rect::new(0, 0, 600, 900)
        );
        assert_eq!(
            at(WindowLayout::RightHalf, Share::TwoThirds),
            Rect::new(600, 0, 1200, 900)
        );
        assert_eq!(
            at(WindowLayout::TopHalf, Share::OneThird),
            Rect::new(0, 0, 1800, 300)
        );
        assert_eq!(
            at(WindowLayout::BottomHalf, Share::TwoThirds),
            Rect::new(0, 300, 1800, 600)
        );
        // Anything else ignores the share.
        assert_eq!(
            at(WindowLayout::LeftThird, Share::TwoThirds),
            at(WindowLayout::LeftThird, Share::Half)
        );
    }

    #[test]
    fn the_monitor_toward_a_half_is_the_nearest_on_that_side() {
        let middle = Rect::new(0, 0, 1920, 1080);
        let monitors = [
            Rect::new(-3840, 0, 1920, 1080),
            Rect::new(-1920, 200, 1920, 1080),
            middle,
            Rect::new(0, -1440, 2560, 1440),
        ];
        let toward = |layout| monitor_toward(&monitors, middle, layout);
        assert_eq!(toward(WindowLayout::LeftHalf), Some(1));
        assert_eq!(toward(WindowLayout::TopHalf), Some(3));
        assert_eq!(toward(WindowLayout::RightHalf), None);
        assert_eq!(toward(WindowLayout::BottomHalf), None);
        assert_eq!(toward(WindowLayout::LeftThird), None);
    }

    #[test]
    fn every_layout_that_tiles_stays_inside_the_work_area() {
        let area = awkward();
        for layout in WindowLayout::ALL {
            let Some(rect) = layout.place(area, window(), 1.0, NONE) else {
                assert_ne!(layout.kind(), LayoutKind::Place);
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
        let left = at(WindowLayout::LeftHalf, area, window());
        let right = at(WindowLayout::RightHalf, area, window());

        assert_eq!(right_edge(left), right.x);
        assert_eq!(left.x, area.x);
        assert_eq!(right_edge(right), right_edge(area));
    }

    #[test]
    fn the_thirds_meet_with_no_seam_and_cover_the_width() {
        let area = awkward();
        let one = at(WindowLayout::LeftThird, area, window());
        let two = at(WindowLayout::MiddleThird, area, window());
        let three = at(WindowLayout::RightThird, area, window());

        assert_eq!(right_edge(one), two.x);
        assert_eq!(right_edge(two), three.x);
        assert_eq!(one.x, area.x);
        assert_eq!(right_edge(three), right_edge(area));
    }

    /// Two thirds plus the remaining third is the whole width, from either end.
    #[test]
    fn the_two_thirds_layouts_complement_the_thirds() {
        let area = awkward();

        let left_two = at(WindowLayout::LeftTwoThirds, area, window());
        let right_one = at(WindowLayout::RightThird, area, window());
        assert_eq!(right_edge(left_two), right_one.x);

        let right_two = at(WindowLayout::RightTwoThirds, area, window());
        let left_one = at(WindowLayout::LeftThird, area, window());
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
        .map(|layout| layout.place(area, window(), 1.0, NONE).unwrap());

        let covered: i64 = quarters
            .iter()
            .map(|rect| rect.width as i64 * rect.height as i64)
            .sum();
        assert_eq!(covered, area.width as i64 * area.height as i64);
    }

    #[test]
    fn maximize_is_the_whole_work_area() {
        let area = awkward();
        assert_eq!(
            WindowLayout::Maximize.place(area, window(), 1.0, NONE),
            Some(area)
        );
    }

    #[test]
    fn center_is_smaller_than_the_screen_and_actually_centred() {
        let area = Rect::new(0, 0, 2000, 1000);
        let rect = at(WindowLayout::Center, area, window());

        assert!(rect.width < area.width && rect.height < area.height);
        assert_eq!(rect.x + rect.width / 2, area.center_x());
        assert_eq!(rect.y + rect.height / 2, area.center_y());
    }

    /// Minimise, restore and focus are window commands, not regions: none has
    /// a place, and each says what it is instead.
    #[test]
    fn the_window_commands_have_no_place() {
        let commands = [
            WindowLayout::Minimize,
            WindowLayout::MinimizeDisplay,
            WindowLayout::Restore,
            WindowLayout::FocusNextAppWindow,
            WindowLayout::FocusPreviousAppWindow,
        ];
        for layout in commands {
            assert_eq!(
                layout.place(awkward(), window(), 1.0, NONE),
                None,
                "{layout:?}"
            );
            assert_eq!(layout.kind(), LayoutKind::Command, "{layout:?}");
            assert!(!layout.changes_monitor(), "{layout:?}");
            assert!(WindowLayout::ALL.contains(&layout), "{layout:?}");
        }
        assert_eq!(WindowLayout::LeftHalf.kind(), LayoutKind::Place);
        assert_eq!(WindowLayout::NextMonitor.kind(), LayoutKind::Monitor);
    }

    /// A layout has a place exactly when it is a placement.
    #[test]
    fn place_answers_exactly_for_placements() {
        for layout in WindowLayout::ALL {
            assert_eq!(
                layout.place(awkward(), window(), 1.0, NONE).is_some(),
                layout.kind() == LayoutKind::Place,
                "{layout:?}"
            );
        }
    }

    /// Only a command that can choose its own windows runs with nothing in
    /// front: the desktop's display is minimised, the last minimised window
    /// restored.
    #[test]
    fn only_minimise_display_and_restore_run_without_a_window() {
        for layout in WindowLayout::ALL {
            let without = matches!(
                layout,
                WindowLayout::MinimizeDisplay | WindowLayout::Restore
            );
            assert_eq!(layout.needs_window(), !without, "{layout:?}");
        }
    }

    #[test]
    fn the_monitor_moves_have_no_tile() {
        assert_eq!(
            WindowLayout::NextMonitor.place(awkward(), window(), 1.0, NONE),
            None
        );
        assert_eq!(
            WindowLayout::PreviousMonitor.place(awkward(), window(), 1.0, NONE),
            None
        );
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

    #[test]
    fn every_layout_stays_inside_even_for_an_oversized_window_hanging_off() {
        let area = awkward();
        let huge = Rect::new(-500, 900, 4000, 3000);
        for layout in WindowLayout::ALL {
            let Some(rect) = layout.place(area, huge, 2.0, NONE) else {
                assert_ne!(layout.kind(), LayoutKind::Place);
                continue;
            };
            assert!(rect.x >= area.x && rect.y >= area.y, "{layout:?}");
            assert!(right_edge(rect) <= right_edge(area), "{layout:?}");
            assert!(bottom_edge(rect) <= bottom_edge(area), "{layout:?}");
            assert!(rect.width > 0 && rect.height > 0, "{layout:?}");
        }
    }

    #[test]
    fn the_fourths_meet_with_no_seam_and_cover_the_width() {
        let area = awkward();
        let fourths = [
            WindowLayout::FirstFourth,
            WindowLayout::SecondFourth,
            WindowLayout::ThirdFourth,
            WindowLayout::LastFourth,
        ]
        .map(|layout| at(layout, area, window()));

        assert_eq!(fourths[0].x, area.x);
        for pair in fourths.windows(2) {
            assert_eq!(right_edge(pair[0]), pair[1].x);
        }
        assert_eq!(right_edge(fourths[3]), right_edge(area));
        for fourth in fourths {
            assert_eq!((fourth.y, fourth.height), (area.y, area.height));
            assert!((fourth.width - area.width / 4).abs() <= 1);
        }
    }

    #[test]
    fn the_sixths_tile_the_work_area_exactly() {
        let area = awkward();
        let top = [
            WindowLayout::TopLeftSixth,
            WindowLayout::TopCenterSixth,
            WindowLayout::TopRightSixth,
        ]
        .map(|layout| at(layout, area, window()));
        let bottom = [
            WindowLayout::BottomLeftSixth,
            WindowLayout::BottomCenterSixth,
            WindowLayout::BottomRightSixth,
        ]
        .map(|layout| at(layout, area, window()));

        for row in [top, bottom] {
            assert_eq!(row[0].x, area.x);
            assert_eq!(right_edge(row[0]), row[1].x);
            assert_eq!(right_edge(row[1]), row[2].x);
            assert_eq!(right_edge(row[2]), right_edge(area));
        }
        for (upper, lower) in top.iter().zip(&bottom) {
            assert_eq!(upper.y, area.y);
            assert_eq!(bottom_edge(*upper), lower.y);
            assert_eq!(bottom_edge(*lower), bottom_edge(area));
            assert_eq!((upper.x, upper.width), (lower.x, lower.width));
        }

        // The columns are the thirds' columns.
        let middle = at(WindowLayout::MiddleThird, area, window());
        assert_eq!((top[1].x, top[1].width), (middle.x, middle.width));
    }

    #[test]
    fn maximise_height_keeps_x_and_width() {
        let area = awkward();
        let rect = at(WindowLayout::MaximizeHeight, area, window());
        assert_eq!(rect, Rect::new(300, area.y, 800, area.height));
    }

    #[test]
    fn maximise_width_keeps_y_and_height() {
        let area = awkward();
        let rect = at(WindowLayout::MaximizeWidth, area, window());
        assert_eq!(rect, Rect::new(area.x, 200, area.width, 600));
    }

    #[test]
    fn almost_maximise_is_nine_tenths_centred() {
        let area = Rect::new(-1920, 40, 1920, 1040);
        let rect = at(WindowLayout::AlmostMaximize, area, window());
        assert_eq!(rect, Rect::new(-1920 + 96, 40 + 52, 1728, 936));
    }

    #[test]
    fn reasonable_size_is_three_fifths_on_a_small_screen() {
        let area = Rect::new(0, 0, 1366, 728);
        let rect = at(WindowLayout::ReasonableSize, area, window());
        assert_eq!(rect, Rect::new(273, 145, 820, 437));
    }

    #[test]
    fn reasonable_size_is_capped_in_logical_pixels() {
        let area = Rect::new(2560, 0, 3840, 2100);

        let rect = WindowLayout::ReasonableSize
            .place(area, window(), 1.0, NONE)
            .unwrap();
        assert_eq!(rect, Rect::new(2560 + 1407, 600, 1025, 900));

        // The same cap at 150% is half as many physical pixels again.
        let rect = WindowLayout::ReasonableSize
            .place(area, window(), 1.5, NONE)
            .unwrap();
        assert_eq!(rect, Rect::new(2560 + 1151, 420, 1538, 1260));
    }

    #[test]
    fn centre_without_resizing_keeps_the_size() {
        let area = Rect::new(0, 0, 2000, 1000);
        let rect = at(WindowLayout::CenterKeepSize, area, window());
        assert_eq!(rect, Rect::new(600, 200, 800, 600));
    }

    #[test]
    fn centre_without_resizing_clamps_an_oversized_window() {
        let area = awkward();
        let rect = at(
            WindowLayout::CenterKeepSize,
            area,
            Rect::new(0, 0, 2500, 600),
        );
        assert_eq!((rect.x, rect.width), (area.x, area.width));
        assert_eq!(rect.height, 600);
        assert_eq!(rect.y, area.y + (area.height - 600) / 2);
    }

    #[test]
    fn the_moves_go_to_an_edge_and_keep_the_size() {
        let area = awkward();
        let w = window();

        assert_eq!(
            at(WindowLayout::MoveLeft, area, w),
            Rect::new(area.x, w.y, w.width, w.height)
        );
        assert_eq!(
            at(WindowLayout::MoveRight, area, w),
            Rect::new(right_edge(area) - w.width, w.y, w.width, w.height)
        );
        assert_eq!(
            at(WindowLayout::MoveUp, area, w),
            Rect::new(w.x, area.y, w.width, w.height)
        );
        assert_eq!(
            at(WindowLayout::MoveDown, area, w),
            Rect::new(w.x, bottom_edge(area) - w.height, w.width, w.height)
        );
    }

    /// On a second monitor the work area does not start at zero, and a window
    /// on it does not either.
    #[test]
    fn current_rect_layouts_work_off_a_non_zero_origin() {
        let area = Rect::new(1920, -200, 1280, 984);
        let w = Rect::new(2000, -100, 500, 400);

        assert_eq!(
            at(WindowLayout::MoveRight, area, w),
            Rect::new(1920 + 1280 - 500, -100, 500, 400)
        );
        assert_eq!(
            at(WindowLayout::MaximizeHeight, area, w),
            Rect::new(2000, -200, 500, 984)
        );
        assert_eq!(
            at(WindowLayout::CenterKeepSize, area, w),
            Rect::new(1920 + 390, -200 + 292, 500, 400)
        );
    }

    /// A window that hangs off the work area keeps its size but is brought
    /// inside, rather than moved to an edge it cannot reach.
    #[test]
    fn a_window_hanging_off_is_brought_inside() {
        let area = Rect::new(0, 0, 1920, 1080);
        let off = Rect::new(1700, 900, 400, 300);

        assert_eq!(
            at(WindowLayout::MaximizeHeight, area, off),
            Rect::new(1520, 0, 400, 1080)
        );
        assert_eq!(
            at(WindowLayout::MoveUp, area, off),
            Rect::new(1520, 0, 400, 300)
        );
    }
}
