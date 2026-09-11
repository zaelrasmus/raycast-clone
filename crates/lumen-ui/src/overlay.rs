//! The context-aware shortcut overlay (F3).
//!
//! Hold the configured key and the sheet for whatever is in front appears;
//! release it and the sheet goes away. It is a *passive* surface: it never
//! activates, never takes focus, is click-through, and is not in the keyboard
//! path at all, so it cannot consume the keystroke that follows it. Those
//! properties are enforced by the window's extended styles in the platform
//! layer, not here.
//!
//! # The animation
//!
//! F3 spends more words on the animation than on the feature, and names the
//! failure precisely: "The animation must be interruptible and reversible from
//! its current value. On a quick release, restarting from opacity one produces a
//! visible jump. This is the most common failure and the reason many overlays
//! feel cheap."
//!
//! That is the whole reason [`Fade`] exists rather than a bare
//! `with_animation(0.0 -> 1.0)`. A GPUI animation runs from a fixed start to a
//! fixed end; asking it to reverse means starting a *new* animation, and the new
//! one has to begin at whatever opacity the old one had reached. So [`Fade`]
//! tracks the value analytically, and on every retarget it reads the current
//! value, makes that the new starting point, and shortens the duration in
//! proportion to the distance left to travel -- which keeps the apparent speed
//! constant whether the fade runs to completion or is caught a third of the way
//! in.

use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{Animation, AnimationExt as _, Context, SharedString, Styled, Window, div, px};
use lumen_core::layout::motion;
use lumen_core::layout::overlay as metrics;
use lumen_core::sheets::{Group, Sheet, Shortcut};
use lumen_core::theme::{Colors, Theme};

use crate::styling::color;

/// A fade that can be reversed from wherever it currently is.
struct Fade {
    from: f32,
    to: f32,
    started: Instant,
    duration: Duration,
    /// Bumped on every retarget. Used as part of the animation element's id, so
    /// GPUI restarts the animation, and to let a pending completion task notice
    /// that it has been superseded.
    generation: usize,
}

/// GPUI's animation is driven by a delta this same curve is applied to, so the
/// analytic value and the rendered value cannot disagree. Using two different
/// easings -- one to compute the interrupt point, another to render -- is a
/// subtle way to reintroduce the very jump this design removes.
fn ease_out(t: f32) -> f32 {
    let inverted = 1.0 - t.clamp(0.0, 1.0);
    1.0 - inverted * inverted * inverted
}

/// Never zero: `Animation::new(Duration::ZERO)` has no sensible progress.
const MIN_DURATION: Duration = Duration::from_millis(1);

impl Fade {
    fn hidden() -> Self {
        Self {
            from: 0.0,
            to: 0.0,
            started: Instant::now(),
            duration: MIN_DURATION,
            generation: 0,
        }
    }

    /// The opacity right now.
    fn value(&self) -> f32 {
        let elapsed = self.started.elapsed().as_secs_f32();
        let total = self.duration.as_secs_f32();
        if total <= 0.0 {
            return self.to;
        }
        let progress = (elapsed / total).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * ease_out(progress)
    }

    /// Head for `target`, starting from wherever the fade has actually reached.
    fn retarget(&mut self, target: f32, base: Duration) {
        let current = self.value();
        let distance = (target - current).abs();

        self.from = current;
        self.to = target;
        // Scaled by the distance left: catching a fade at 20% and reversing it
        // should take a fifth of the time, not the whole duration. Without this
        // a quick tap leaves the overlay lingering on the way out.
        self.duration = base.mul_f32(distance).max(MIN_DURATION);
        self.started = Instant::now();
        self.generation = self.generation.wrapping_add(1);
    }

    fn is_hiding(&self) -> bool {
        self.to == 0.0
    }
}

/// What the overlay asks the window to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayEvent {
    /// The fade to zero has finished, so the window can actually be hidden.
    ///
    /// Emitted rather than hiding directly because window visibility is the
    /// platform's business (I-4), and because hiding at the *start* of the fade
    /// would mean there was no fade.
    FadedOut,
}

impl gpui::EventEmitter<OverlayEvent> for Overlay {}

pub struct Overlay {
    theme: Theme,
    sheet: Sheet,
    fade: Fade,
}

impl Overlay {
    pub fn new(theme: Theme, sheet: Sheet) -> Self {
        Self {
            theme,
            sheet,
            fade: Fade::hidden(),
        }
    }

    /// Show `sheet`, fading in from the current opacity.
    ///
    /// The caller shows the window first, while this is still at zero opacity:
    /// "The animation starts at zero opacity: if the first frame arrives late it
    /// is invisible, and the hitch hides itself" (F3).
    pub fn present(&mut self, sheet: Sheet, cx: &mut Context<Self>) {
        self.sheet = sheet;
        self.fade.retarget(1.0, motion::OVERLAY_ENTER);
        cx.notify();
    }

    /// Fade out, then ask for the window to be hidden.
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.fade.retarget(0.0, motion::OVERLAY_EXIT);
        let generation = self.fade.generation;
        let duration = self.fade.duration;
        cx.notify();

        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(duration).await;

            this.update(cx, |this, cx| {
                // If the user pressed again during the fade, a newer transition
                // owns the overlay and this completion is stale. Hiding the
                // window here would blank a sheet that is on its way back in.
                if this.fade.generation == generation && this.fade.is_hiding() {
                    cx.emit(OverlayEvent::FadedOut);
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn apply_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    /// The sheet currently displayed, for sizing the window.
    pub fn sheet(&self) -> &Sheet {
        &self.sheet
    }
}

impl Render for Overlay {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &self.theme.colors;
        let columns = self.sheet.columns(metrics::COLUMNS);

        let mut panel = div()
            .size_full()
            .flex()
            .flex_col()
            .p(px(metrics::PADDING))
            .rounded(px(metrics::RADIUS))
            .border_1()
            .border_color(color(colors.border_subtle))
            .bg(color(colors.surface_background))
            .text_color(color(colors.text_primary));

        if let Some(family) = &self.theme.font_family {
            panel = panel.font_family(SharedString::from(family.clone()));
        }

        let panel = panel
            .child(
                div()
                    .h(px(metrics::HEADER_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(metrics::text::HEADING))
                            .child(self.sheet.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(metrics::text::GROUP))
                            .text_color(color(colors.text_muted))
                            .child("Shortcuts"),
                    ),
            )
            .child(div().flex().flex_1().gap(px(metrics::COLUMN_GAP)).children(
                columns.into_iter().map(|column| {
                    div().flex().flex_col().flex_1().children(
                        column
                            .into_iter()
                            .map(|index| render_group(&self.sheet.groups[index], colors)),
                    )
                }),
            ));

        let (from, to) = (self.fade.from, self.fade.to);
        panel.with_animation(
            ("overlay-fade", self.fade.generation),
            Animation::new(self.fade.duration).with_easing(ease_out),
            move |panel, delta| panel.opacity(from + (to - from) * delta),
        )
    }
}

fn render_group(group: &Group, colors: &Colors) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .mb(px(metrics::GROUP_GAP))
        .child(
            div()
                .h(px(metrics::GROUP_TITLE_HEIGHT))
                .flex()
                .items_center()
                .text_size(px(metrics::text::GROUP))
                .text_color(color(colors.accent))
                .child(group.title.to_uppercase()),
        )
        .children(
            group
                .shortcuts
                .iter()
                .map(|shortcut| render_shortcut(shortcut, colors)),
        )
}

fn render_shortcut(shortcut: &Shortcut, colors: &Colors) -> impl IntoElement {
    div()
        .h(px(metrics::ROW_HEIGHT))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(metrics::COLUMN_GAP))
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .text_size(px(metrics::text::DESCRIPTION))
                .text_color(color(colors.text_primary))
                .child(shortcut.description.clone()),
        )
        .child(div().flex().flex_none().gap(px(4.0)).children(
            keycaps(&shortcut.keys).into_iter().map(|cap| {
                div()
                    .px(px(5.0))
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(color(colors.border_subtle))
                    .bg(color(colors.surface_elevated))
                    .text_size(px(metrics::text::KEYS))
                    .text_color(color(colors.text_muted))
                    .child(SharedString::from(cap))
            }),
        ))
}

/// Split a chord into the individual caps to draw.
///
/// `keys` is free text written by the sheet's author -- it describes another
/// application's shortcut, which Lumen has no way to parse or validate. So this
/// splits on `+` and otherwise leaves the text exactly as written: an author who
/// writes "Hold Right Ctrl" gets one cap saying that, rather than an error.
fn keycaps(keys: &str) -> Vec<String> {
    let caps: Vec<String> = keys
        .split('+')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .map(|part| part.to_string())
        .collect();

    if caps.is_empty() {
        vec![keys.trim().to_string()]
    } else {
        caps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_chord_into_caps() {
        assert_eq!(keycaps("Ctrl+Shift+P"), vec!["Ctrl", "Shift", "P"]);
    }

    #[test]
    fn leaves_prose_written_by_the_author_alone() {
        assert_eq!(keycaps("Hold Right Ctrl"), vec!["Hold Right Ctrl"]);
    }

    #[test]
    fn tolerates_spacing_and_stray_separators() {
        assert_eq!(keycaps("Ctrl + Shift + P"), vec!["Ctrl", "Shift", "P"]);
        assert_eq!(keycaps("Ctrl++"), vec!["Ctrl"]);
    }

    #[test]
    fn the_easing_spans_the_whole_range() {
        assert!((ease_out(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((ease_out(1.0) - 1.0).abs() < f32::EPSILON);
        // Ease-*out*: most of the distance is covered early.
        assert!(ease_out(0.5) > 0.5);
    }

    #[test]
    fn a_fresh_fade_starts_invisible() {
        // F3: the animation starts at zero opacity, so a late first frame is
        // invisible rather than a flash.
        assert_eq!(Fade::hidden().value(), 0.0);
    }

    #[test]
    fn retargeting_resumes_from_the_current_value_not_from_the_end() {
        let mut fade = Fade::hidden();
        fade.retarget(1.0, Duration::from_millis(150));

        // Catch it part-way in.
        std::thread::sleep(Duration::from_millis(40));
        let caught = fade.value();
        assert!(caught > 0.0 && caught < 1.0, "caught at {caught}");

        fade.retarget(0.0, Duration::from_millis(90));

        // The reverse begins exactly where the forward fade had reached. This
        // is the assertion that encodes "no visible jump".
        assert!(
            (fade.from - caught).abs() < 0.02,
            "{} vs {caught}",
            fade.from
        );
        assert_eq!(fade.to, 0.0);
    }

    #[test]
    fn a_reversal_is_shortened_in_proportion_to_the_distance_left() {
        let mut fade = Fade::hidden();
        // Interrupted immediately, so barely any opacity was gained.
        fade.retarget(1.0, Duration::from_millis(150));
        fade.retarget(0.0, Duration::from_millis(90));

        // Almost nothing to undo, so it must not take the full exit duration.
        assert!(
            fade.duration < Duration::from_millis(20),
            "a near-instant reversal took {:?}",
            fade.duration
        );
    }

    #[test]
    fn each_retarget_supersedes_the_previous_one() {
        let mut fade = Fade::hidden();
        let first = fade.generation;
        fade.retarget(1.0, Duration::from_millis(150));
        assert_ne!(fade.generation, first);

        let second = fade.generation;
        fade.retarget(0.0, Duration::from_millis(90));
        assert_ne!(fade.generation, second);
    }

    #[test]
    fn the_duration_never_reaches_zero() {
        let mut fade = Fade::hidden();
        // Retargeting to the value it already holds leaves zero distance.
        fade.retarget(0.0, Duration::from_millis(90));
        assert!(fade.duration >= MIN_DURATION);
    }
}
