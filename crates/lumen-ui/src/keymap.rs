//! In-window key bindings (AD-15, I-14).
//!
//! Every binding is declared here and resolved by GPUI against the focus tree.
//! Nothing in the UI compares keys by hand.
//!
//! This is the *inside the window* half of AD-15. Global invocation is the other
//! half, lives behind `trait Platform`, and the two never overlap: GPUI resolves
//! keystrokes against the focus tree, which requires the window to already hold
//! focus, so it cannot cover summoning the window in the first place.

use gpui::{App, KeyBinding};

use crate::launcher::{
    Activate, ClearQuery, DeleteBackward, DeleteWordBackward, Dismiss, Paste, SelectFirst,
    SelectLast, SelectNext, SelectPrevious,
};

/// The key context these bindings apply in, matching `Launcher`'s.
const CONTEXT: &str = "Launcher";

/// Install the default bindings.
///
/// F2 promised a keymap *file*; this is the vocabulary that file will bind
/// against. Loading user overrides is a small addition on top of
/// `cx.bind_keys`, and deliberately not invented before there is a second set of
/// bindings to reconcile it with.
pub fn install(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, Some(CONTEXT)),
        KeyBinding::new("up", SelectPrevious, Some(CONTEXT)),
        // The Emacs-style pair as well: the hands are already on the home row
        // while typing a query, and reaching for the arrow keys breaks the flow
        // the launcher exists to preserve.
        KeyBinding::new("ctrl-n", SelectNext, Some(CONTEXT)),
        KeyBinding::new("ctrl-p", SelectPrevious, Some(CONTEXT)),
        KeyBinding::new("home", SelectFirst, Some(CONTEXT)),
        KeyBinding::new("end", SelectLast, Some(CONTEXT)),
        KeyBinding::new("pagedown", SelectLast, Some(CONTEXT)),
        KeyBinding::new("pageup", SelectFirst, Some(CONTEXT)),
        KeyBinding::new("enter", Activate, Some(CONTEXT)),
        // Escape clears a query and dismisses an empty one, so it always means
        // "back" rather than sometimes meaning "quit".
        KeyBinding::new("escape", ClearQuery, Some(CONTEXT)),
        KeyBinding::new("backspace", DeleteBackward, Some(CONTEXT)),
        KeyBinding::new("ctrl-backspace", DeleteWordBackward, Some(CONTEXT)),
        KeyBinding::new("ctrl-w", DeleteWordBackward, Some(CONTEXT)),
        KeyBinding::new("ctrl-v", Paste, Some(CONTEXT)),
        KeyBinding::new("ctrl-u", ClearQuery, Some(CONTEXT)),
    ]);

    // Bound without a context so it works even if focus has somehow left the
    // launcher's subtree. A launcher that cannot be dismissed is a launcher that
    // has to be killed from Task Manager.
    cx.bind_keys([KeyBinding::new("ctrl-escape", Dismiss, None)]);
}
