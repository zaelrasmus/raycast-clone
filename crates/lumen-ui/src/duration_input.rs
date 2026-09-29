//! A duration as three numbers: hours, minutes, seconds (F8).
//!
//! The launcher reads `25m` and `1h30` because typing is what it is for. A
//! form is not: there, a person wants to see which box is the minutes. So a
//! duration in a form is three short fields with their units beside them,
//! and what is saved is written back in the launcher's grammar
//! (`timers::duration::written`), so the file stays readable by hand.
//!
//! An empty field is zero, and a field may overflow into the next unit --
//! `90` minutes is an hour and a half -- because that is what was meant.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{App, Context, Entity, FocusHandle, Focusable, Window, div, px};
use lumen_core::layout;
use lumen_core::theme::Colors;
use lumen_core::timers::duration::split;

use crate::styling::{color, type_scale};
use crate::text_field::{TextField, TextFieldEvent};

/// What the input tells its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationEvent {
    Changed,
}

impl gpui::EventEmitter<DurationEvent> for DurationInput {}

const UNITS: [&str; 3] = ["h", "min", "s"];

pub struct DurationInput {
    /// Hours, minutes, seconds, in that order.
    fields: [Entity<TextField>; 3],
    colors: Colors,
}

impl DurationInput {
    /// Showing `value`, or nothing. A zero part is left empty rather than
    /// written `0`, so `25 min` reads as one number, not three.
    pub fn new(value: Option<Duration>, colors: Colors, cx: &mut Context<Self>) -> Self {
        let (hours, minutes, seconds) = value.map_or((0, 0, 0), split);
        let fields = [hours, minutes, seconds].map(|part| {
            let text = if part == 0 {
                String::new()
            } else {
                part.to_string()
            };
            let field = cx.new(|cx| TextField::new(text, "0", colors.clone(), cx));
            cx.subscribe(&field, |_, _, event, cx| {
                if matches!(event, TextFieldEvent::Changed(_)) {
                    cx.emit(DurationEvent::Changed);
                }
            })
            .detach();
            field
        });
        Self { fields, colors }
    }

    /// The duration typed: `None` when it is zero, and why not when a field
    /// is not a whole number.
    pub fn value(&self, cx: &App) -> Result<Option<Duration>, String> {
        let texts = self
            .fields
            .each_ref()
            .map(|field| field.read(cx).text().trim().to_string());
        parse(&texts)
    }

    /// The three fields, for Tab.
    pub fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        self.fields
            .iter()
            .map(|field| field.focus_handle(cx))
            .collect()
    }

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        for field in &self.fields {
            field.update(cx, |field, cx| field.set_colors(colors.clone(), cx));
        }
        self.colors = colors;
        cx.notify();
    }
}

/// Three fields' text as a duration.
fn parse(texts: &[String; 3]) -> Result<Option<Duration>, String> {
    let mut total: u64 = 0;
    for (text, (unit, seconds)) in
        texts
            .iter()
            .zip([("hours", 3600), ("minutes", 60), ("seconds", 1)])
    {
        if text.is_empty() {
            continue;
        }
        let value: u64 = text
            .parse()
            .map_err(|_| format!("The {unit} are a whole number, not {text:?}"))?;
        total = total.saturating_add(value.saturating_mul(seconds));
    }
    Ok((total > 0).then(|| Duration::from_secs(total)))
}

impl Render for DurationInput {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &self.colors;
        div()
            .flex()
            .items_center()
            .gap(px(layout::form::DURATION_GAP))
            .children(self.fields.iter().zip(UNITS).map(|(field, unit)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(layout::form::DURATION_GAP))
                    .child(
                        div()
                            .w(px(layout::form::DURATION_FIELD_WIDTH))
                            .child(field.clone()),
                    )
                    .child(
                        div()
                            .text_size(type_scale::subtitle())
                            .text_color(color(colors.text_muted))
                            .child(unit),
                    )
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(h: &str, m: &str, s: &str) -> [String; 3] {
        [h.into(), m.into(), s.into()]
    }

    #[test]
    fn three_fields_make_one_duration() {
        assert_eq!(
            parse(&texts("1", "30", "")),
            Ok(Some(Duration::from_secs(5400)))
        );
        assert_eq!(
            parse(&texts("", "25", "")),
            Ok(Some(Duration::from_secs(1500)))
        );
        assert_eq!(
            parse(&texts("", "", "45")),
            Ok(Some(Duration::from_secs(45)))
        );
    }

    #[test]
    fn a_field_may_overflow_into_the_next_unit() {
        assert_eq!(
            parse(&texts("", "90", "")),
            Ok(Some(Duration::from_secs(5400)))
        );
    }

    #[test]
    fn nothing_is_no_duration_and_words_are_refused() {
        assert_eq!(parse(&texts("", "", "")), Ok(None));
        assert_eq!(parse(&texts("0", "0", "0")), Ok(None));
        let error = parse(&texts("", "ten", "")).unwrap_err();
        assert!(error.contains("minutes"), "{error}");
        assert!(parse(&texts("-1", "", "")).is_err());
    }
}
