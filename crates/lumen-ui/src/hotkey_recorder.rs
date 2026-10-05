//! The hotkey recorder (F17): the one place in the UI that reads keys.
//!
//! Every other shortcut in Lumen is a GPUI action bound in the keymap
//! (I-14). Recording a hotkey cannot be: the keys pressed *are* the data, any
//! combination at all, so the recorder listens to the raw key-down and turns
//! it into a [`Chord`] in [`capture`] -- here, and nowhere else (7.56).
//!
//! What still is an action: the keys that steer the recorder. Escape cancels,
//! Enter saves and Backspace clears, bound in the keymap in [`CONTEXT`]
//! like any other key. That context also unbinds the settings window's own
//! chords (Ctrl+S, Ctrl+Tab), so they are recorded rather than run: GPUI runs
//! a binding before any key listener sees the key.
//!
//! The launcher's and the timers' hotkeys stay registered while recording --
//! the launcher must stay summonable -- so pressing one toggles the launcher
//! and never arrives here. The page refuses either by name
//! ([`lumen_core::shortcuts::reserved_by`]).

use gpui::prelude::*;
use gpui::{
    Context, Div, FocusHandle, KeyDownEvent, ModifiersChangedEvent, SharedString, Window, actions,
    div, px,
};
use lumen_core::layout::settings as metrics;
use lumen_core::theme::Colors;
use lumen_platform::{Chord, Modifiers, canonical_key};

use crate::styling::color;

actions!(
    lumen_recorder,
    [
        /// Stop recording and keep the hotkey as it was.
        CancelRecording,
        /// Save the chord recorded.
        SaveRecording,
        /// Remove the command's hotkey.
        ClearRecording,
    ]
);

/// The recorder's key context: deeper than the settings window's, so its
/// bindings, and its unbinding of the window's, win while it has focus.
pub const CONTEXT: &str = "HotkeyRecorder";

/// Why a key alone is not a hotkey.
pub const NEEDS_MODIFIER: &str = "Add Ctrl, Alt, Shift or Win; only F13 to F24 stand alone";
/// Why a key cannot be part of one.
pub const UNSUPPORTED_KEY: &str = "That key cannot be a hotkey";

/// What the keys pressed so far amount to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capture {
    /// Modifiers held, and no key yet: drawn as they are, not yet a chord.
    Held(Modifiers),
    /// A hotkey.
    Chord(Chord),
    /// A key that makes no hotkey with what is held, and why.
    Invalid {
        held: Modifiers,
        key: String,
        why: &'static str,
    },
}

impl Default for Capture {
    fn default() -> Self {
        Self::Held(Modifiers::default())
    }
}

impl Capture {
    /// One cap per modifier, then the key, in the order a chord is written.
    pub fn caps(&self) -> Vec<String> {
        let (held, key) = match self {
            Capture::Held(held) => (held, None),
            Capture::Chord(chord) => (&chord.modifiers, Some(chord.key.as_str())),
            Capture::Invalid { held, key, .. } => (held, Some(key.as_str())),
        };
        let mut caps: Vec<String> = [
            (held.control, "Ctrl"),
            (held.alt, "Alt"),
            (held.shift, "Shift"),
            (held.super_key, "Win"),
        ]
        .into_iter()
        .filter(|(down, _)| *down)
        .map(|(_, name)| name.to_string())
        .collect();
        caps.extend(key.map(str::to_string));
        caps
    }
}

/// The modifiers GPUI reports, as a chord's. The function key is not one.
pub fn held(modifiers: &gpui::Modifiers) -> Modifiers {
    Modifiers {
        control: modifiers.control,
        alt: modifiers.alt,
        shift: modifiers.shift,
        super_key: modifiers.platform,
    }
}

/// A key-down, as a hotkey or the reason it is not one.
///
/// Valid is what [`Chord`]'s parser accepts -- one rule, not two: a key with
/// at least one modifier, or F13 to F24 alone.
pub fn capture(keystroke: &gpui::Keystroke) -> Capture {
    let mut modifiers = held(&keystroke.modifiers);
    let pressed = keystroke.key.as_str();
    if let Some(own) = modifier_key(pressed) {
        own(&mut modifiers);
        return Capture::Held(modifiers);
    }
    // gpui reports Shift with a punctuation key as the shifted character,
    // Shift taken off; a chord names the key.
    let key = match unshifted(pressed) {
        Some(key) => {
            modifiers.shift = true;
            Some(key)
        }
        None => canonical_key(pressed),
    };
    let Some(key) = key else {
        return Capture::Invalid {
            held: modifiers,
            key: pressed.to_uppercase(),
            why: UNSUPPORTED_KEY,
        };
    };
    let chord = Chord {
        modifiers,
        key: key.to_string(),
    };
    match chord.to_string().parse::<Chord>() {
        Ok(chord) => Capture::Chord(chord),
        Err(_) => Capture::Invalid {
            held: modifiers,
            key: key.to_string(),
            why: NEEDS_MODIFIER,
        },
    }
}

/// What a modifier key's own key-down holds, for a platform that reports
/// one as a key rather than only as a change of modifiers.
fn modifier_key(key: &str) -> Option<fn(&mut Modifiers)> {
    Some(match key {
        "control" | "ctrl" => |m: &mut Modifiers| m.control = true,
        "alt" => |m: &mut Modifiers| m.alt = true,
        "shift" => |m: &mut Modifiers| m.shift = true,
        "platform" | "super" | "win" | "cmd" => |m: &mut Modifiers| m.super_key = true,
        "function" | "fn" => |_: &mut Modifiers| {},
        _ => return None,
    })
}

/// The key under a shifted character, on the US layout the key names
/// describe.
fn unshifted(character: &str) -> Option<&'static str> {
    Some(match character {
        "~" => "Backtick",
        "_" => "Minus",
        "+" => "Equals",
        "{" => "LeftBracket",
        "}" => "RightBracket",
        "|" => "Backslash",
        ":" => "Semicolon",
        "\"" => "Quote",
        "<" => "Comma",
        ">" => "Period",
        "?" => "Slash",
        _ => return None,
    })
}

/// What the recorder tells the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecorderEvent {
    /// The keys held or pressed changed.
    Changed(Capture),
    Save,
    Cancel,
    Clear,
}

impl gpui::EventEmitter<RecorderEvent> for HotkeyRecorder {}

/// The field while it records: what is held, as caps, live.
pub struct HotkeyRecorder {
    focus_handle: FocusHandle,
    capture: Capture,
    colors: Colors,
}

impl HotkeyRecorder {
    pub fn new(colors: Colors, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            capture: Capture::default(),
            colors,
        }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        self.colors = colors;
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if event.is_held {
            return;
        }
        self.capture = capture(&event.keystroke);
        cx.emit(RecorderEvent::Changed(self.capture.clone()));
        cx.notify();
    }

    /// Held modifiers show as they go down. Once a chord is recorded,
    /// letting go of them keeps it; a refusal stays until the next key.
    fn modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now = held(&event.modifiers);
        let keep = match &self.capture {
            Capture::Chord(_) => true,
            Capture::Invalid { .. } => now == Modifiers::default(),
            Capture::Held(_) => false,
        };
        if keep {
            return;
        }
        self.capture = Capture::Held(now);
        cx.emit(RecorderEvent::Changed(self.capture.clone()));
        cx.notify();
    }

    fn cancel(&mut self, _: &CancelRecording, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(RecorderEvent::Cancel);
    }

    fn save(&mut self, _: &SaveRecording, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(RecorderEvent::Save);
    }

    fn clear(&mut self, _: &ClearRecording, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(RecorderEvent::Clear);
    }
}

impl Render for HotkeyRecorder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let caps = self.capture.caps();
        let invalid = matches!(self.capture, Capture::Invalid { .. });
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::clear))
            .on_key_down(cx.listener(Self::key_down))
            .on_modifiers_changed(cx.listener(Self::modifiers_changed))
            .size_full()
            .flex()
            .items_center()
            .when(caps.is_empty(), |field| {
                field
                    .text_size(px(metrics::text::LABEL))
                    .text_color(color(self.colors.text_muted))
                    .child("Press a combination\u{2026}")
            })
            .when(!caps.is_empty(), |field| {
                field.child(keycaps(&caps, invalid, &self.colors))
            })
    }
}

/// A chord as a row of key caps: the recorder's live, a row's saved.
pub fn keycaps(caps: &[String], invalid: bool, colors: &Colors) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(metrics::KEYCAP_GAP))
        .children(caps.iter().map(|cap| {
            div()
                .h(px(metrics::KEYCAP_HEIGHT))
                .px(px(metrics::KEYCAP_PADDING_X))
                .flex()
                .items_center()
                .rounded(px(metrics::KEYCAP_RADIUS))
                .border_1()
                .border_color(color(if invalid {
                    colors.text_error
                } else {
                    colors.border_strong
                }))
                .bg(color(colors.surface_background))
                .text_size(px(metrics::text::HINT))
                .text_color(color(colors.text_primary))
                .child(SharedString::from(cap.clone()))
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pressed(keys: &str) -> Capture {
        capture(&gpui::Keystroke::parse(keys).unwrap())
    }

    fn chord(text: &str) -> Chord {
        text.parse().unwrap()
    }

    #[test]
    fn a_key_with_modifiers_is_a_chord() {
        assert_eq!(pressed("ctrl-alt-k"), Capture::Chord(chord("Ctrl+Alt+K")));
        assert_eq!(
            pressed("ctrl-shift-left"),
            Capture::Chord(chord("Ctrl+Shift+Left"))
        );
        assert_eq!(pressed("win-shift-m"), Capture::Chord(chord("Shift+Win+M")));
        assert_eq!(pressed("alt-f4"), Capture::Chord(chord("Alt+F4")));
        assert_eq!(pressed("ctrl-`"), Capture::Chord(chord("Ctrl+Backtick")));
    }

    /// How Windows reports Shift with a punctuation key: the shifted
    /// character, Shift dropped.
    #[test]
    fn a_shifted_character_is_its_key_with_shift() {
        let keystroke = gpui::Keystroke {
            modifiers: gpui::Modifiers {
                control: true,
                ..gpui::Modifiers::default()
            },
            key: "~".into(),
            key_char: None,
        };
        assert_eq!(
            capture(&keystroke),
            Capture::Chord(chord("Ctrl+Shift+Backtick"))
        );
    }

    #[test]
    fn f13_to_f24_stand_alone() {
        assert_eq!(pressed("f13"), Capture::Chord(chord("F13")));
        assert_eq!(pressed("f24"), Capture::Chord(chord("F24")));
    }

    #[test]
    fn a_letter_alone_needs_a_modifier() {
        assert_eq!(
            pressed("k"),
            Capture::Invalid {
                held: Modifiers::default(),
                key: "K".into(),
                why: NEEDS_MODIFIER,
            }
        );
        assert!(matches!(pressed("f12"), Capture::Invalid { .. }));
    }

    #[test]
    fn modifiers_alone_are_not_yet_a_chord() {
        let control = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(pressed("ctrl-control"), Capture::Held(control));
        assert_eq!(pressed("shift"), Capture::Held(shift));
        assert_eq!(
            held(&gpui::Modifiers {
                control: true,
                alt: true,
                ..gpui::Modifiers::default()
            }),
            Modifiers {
                control: true,
                alt: true,
                ..Modifiers::default()
            }
        );
    }

    #[test]
    fn a_key_no_chord_can_name_is_refused() {
        assert!(matches!(
            pressed("ctrl-capslock"),
            Capture::Invalid {
                why: UNSUPPORTED_KEY,
                ..
            }
        ));
    }

    #[test]
    fn what_is_held_reads_as_caps() {
        assert_eq!(
            pressed("ctrl-alt-k").caps(),
            ["Ctrl", "Alt", "K"].map(String::from)
        );
        assert_eq!(
            Capture::Held(Modifiers {
                shift: true,
                super_key: true,
                ..Modifiers::default()
            })
            .caps(),
            ["Shift", "Win"].map(String::from)
        );
        assert_eq!(pressed("k").caps(), ["K"].map(String::from));
    }
}
