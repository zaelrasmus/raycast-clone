//! Global keyboard gestures.
//!
//! This is the *outside the window* half of AD-15. Everything inside the window
//! is GPUI's action and keymap system, and neither layer intrudes on the other.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// A modifier key, distinguished by side.
///
/// The side matters: the hold gesture of AD-16 is a *specific* physical key, and
/// treating left and right alt as one key makes the gesture fire when the user
/// meant the other one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modifier {
    LeftAlt,
    RightAlt,
    LeftControl,
    RightControl,
    LeftShift,
    RightShift,
    LeftSuper,
    RightSuper,
}

/// The gesture that summons the launcher.
///
/// Modelled as modifiers plus one non-modifier key, which is what every system
/// accelerator API accepts. The *hold* gesture is deliberately not expressed
/// here: it is not an accelerator (AD-16) and lives in [`GestureEvent::Hold`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotkey {
    /// Modifiers that must be down, side-insensitive for invocation.
    pub modifiers: Modifiers,
    /// The non-modifier key, as a virtual-key name (`"Space"`, `"K"`).
    pub key: String,
    /// The lone modifier whose press-and-hold raises the shortcut overlay
    /// (F3). `None` disables the gesture.
    ///
    /// Never `LeftSuper`/`RightSuper` by default: releasing Super opens the
    /// Start menu, and suppressing that is the one case that would force a
    /// low-level hook (section 7).
    pub hold: Option<Modifier>,
}

impl Default for Hotkey {
    fn default() -> Self {
        Self {
            modifiers: Modifiers {
                alt: true,
                ..Default::default()
            },
            key: "Space".to_string(),
            // Right control, and the choice is constrained from three sides.
            //
            // Not Super: releasing it opens the Start menu, and suppressing that
            // is the single case section 7 says would force a low-level hook,
            // with the timeout and antivirus hazards that brings.
            //
            // Not right alt: on many keyboard layouts it is AltGr and produces
            // characters, so holding it would type into whatever has focus.
            //
            // Not a left-hand modifier: those are held constantly as part of
            // ordinary chords, and a sheet that appears during Alt+Tab or on the
            // way to Ctrl+C is a sheet the user turns off within a day.
            hold: Some(Modifier::RightControl),
        }
    }
}

/// Which modifiers must be held, side-insensitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Modifiers {
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub control: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub super_key: bool,
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = &self.modifiers;
        for (active, name) in [
            (m.control, "Ctrl"),
            (m.alt, "Alt"),
            (m.shift, "Shift"),
            (m.super_key, "Win"),
        ] {
            if active {
                write!(f, "{name}+")?;
            }
        }
        write!(f, "{}", self.key)
    }
}

/// What the global input source reports.
///
/// Both variants derive from one event stream (section 7: Raw Input as the
/// single source of truth), because two coexisting event loops disagree
/// eventually and the disagreement is invisible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureEvent {
    /// The launcher hotkey fired. Toggles visibility.
    Invoke,
    /// The hold gesture crossed its activation threshold, or ended.
    Hold(HoldPhase),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldPhase {
    /// The key has been down past the activation threshold. Show the overlay.
    Began,
    /// The key came back up. Hide it.
    Ended,
}

/// Where global gesture events are delivered.
///
/// I-15: the closure behind this sink runs on the system's input thread. It must
/// not allocate beyond the send, log, take a lock or touch UI state. It pushes
/// and returns.
#[derive(Clone)]
pub struct GestureSink(Arc<dyn Fn(GestureEvent) + Send + Sync>);

impl GestureSink {
    pub fn new(f: impl Fn(GestureEvent) + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    #[inline]
    pub fn emit(&self, event: GestureEvent) {
        (self.0)(event)
    }
}

impl fmt::Debug for GestureSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GestureSink")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_displays_in_conventional_order() {
        let hotkey = Hotkey {
            modifiers: Modifiers {
                control: true,
                alt: true,
                ..Default::default()
            },
            key: "Space".into(),
            hold: None,
        };
        assert_eq!(hotkey.to_string(), "Ctrl+Alt+Space");
    }

    #[test]
    fn default_hotkey_does_not_use_super() {
        // Releasing Super opens the Start menu; defaulting to it would force a
        // low-level hook on every install (section 7).
        let hotkey = Hotkey::default();
        assert!(!hotkey.modifiers.super_key);
        assert_eq!(hotkey.to_string(), "Alt+Space");
    }
}
