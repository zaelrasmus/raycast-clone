//! A dropdown: one value from a short list (F6).
//!
//! Made for the quicklink form's "Open with", where free text meant knowing
//! how Windows names each browser. The list is what is installed, so the
//! choice is one the platform can actually honour.
//!
//! # Keys
//!
//! Its own key context, and closed it gets out of the way: Up, Down, Enter
//! and Escape fall through to the form, so they still move between fields,
//! save and cancel. Space opens it. Open, Up and Down move through the
//! options, Enter or Space takes one, and Escape closes the list without
//! leaving the form (I-14).

use gpui::prelude::*;
use gpui::{
    App, Context, Corner, FocusHandle, ScrollHandle, SharedString, Window, actions, anchored,
    deferred, div, point, px,
};
use lumen_core::layout;
use lumen_core::theme::Colors;

use crate::styling::{color, type_scale};

actions!(
    lumen_select,
    [
        /// Open the list, or take the highlighted option when it is open.
        ToggleSelect,
        /// Take the highlighted option, when the list is open.
        ConfirmSelect,
        /// Close the list without choosing.
        CloseSelect,
        /// Highlight the option above.
        SelectUp,
        /// Highlight the option below.
        SelectDown,
    ]
);

/// The dropdown's key context.
pub const CONTEXT: &str = "Select";

/// What the dropdown tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectEvent {
    /// A different option was chosen.
    Changed,
}

impl gpui::EventEmitter<SelectEvent> for Select {}

/// One option: the value it stands for, and what the list says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub value: Option<String>,
    pub label: String,
}

/// The browsers "Open with" offers, and which of them `current` is.
///
/// The default browser first, as `None`. Then the installed ones, in the
/// platform's order. A saved name matches an installed browser the way
/// opening the link does ([`choose_browser`]), so a file saying `brave` shows
/// Brave. One that matches nothing is kept, marked as missing, rather than
/// dropped: the file is the user's, and saving an unrelated field must not
/// quietly change which browser a link opens in.
///
/// [`choose_browser`]: lumen_core::providers::quicklinks::choose_browser
pub fn browser_choices(current: Option<&str>, installed: &[String]) -> (Vec<Choice>, usize) {
    let mut choices = vec![Choice {
        value: None,
        label: "Default browser".into(),
    }];
    choices.extend(installed.iter().map(|name| Choice {
        value: Some(name.clone()),
        label: name.clone(),
    }));

    let current = current.map(str::trim).filter(|value| !value.is_empty());
    let Some(current) = current else {
        return (choices, 0);
    };

    let found = lumen_core::providers::quicklinks::matching_browser(current, installed);
    match found.and_then(|name| {
        choices
            .iter()
            .position(|c| c.value.as_deref() == Some(name))
    }) {
        Some(index) => (choices, index),
        None => {
            // Not "missing" before the list has arrived to say so.
            let label = if installed.is_empty() {
                current.to_string()
            } else {
                format!("{current} (not installed)")
            };
            choices.push(Choice {
                value: Some(current.to_string()),
                label,
            });
            let index = choices.len() - 1;
            (choices, index)
        }
    }
}

/// The apps "Open with" offers for a file or folder link, and which of them
/// `current` is (F16).
///
/// The platform's list, default first, each stored as [`APP_PREFIX`] and its
/// id; nothing stored selects the default. A stored app that is not listed is
/// kept, marked, as [`browser_choices`] keeps a browser. Until the list
/// arrives, or when it cannot be read, the one choice is the default app.
///
/// [`APP_PREFIX`]: lumen_core::providers::quicklinks::APP_PREFIX
pub fn app_choices(
    current: Option<&str>,
    apps: &[lumen_platform::AppChoice],
) -> (Vec<Choice>, usize) {
    use lumen_core::providers::quicklinks::APP_PREFIX;

    let mut choices: Vec<Choice> = apps
        .iter()
        .map(|app| Choice {
            value: Some(format!("{APP_PREFIX}{}", app.id)),
            label: app.name.clone(),
        })
        .collect();
    if choices.is_empty() {
        choices.push(Choice {
            value: None,
            label: "Default app".into(),
        });
    }

    // Only an app: a browser's name, from when the link was a URL, means
    // nothing to a file.
    let current = current
        .map(str::trim)
        .filter(|value| value.starts_with(APP_PREFIX));
    let Some(current) = current else {
        return (choices, 0);
    };
    if let Some(index) = choices
        .iter()
        .position(|choice| choice.value.as_deref() == Some(current))
    {
        return (choices, index);
    }
    let id = &current[APP_PREFIX.len()..];
    let label = if apps.is_empty() {
        id.to_string()
    } else {
        format!("{id} (not found)")
    };
    choices.push(Choice {
        value: Some(current.to_string()),
        label,
    });
    let index = choices.len() - 1;
    (choices, index)
}

/// What a quicklink's "Icon" offers, and which of them `current` is.
///
/// The site's icon is the absence of a value, as in the file. A value this
/// build does not know is offered as itself, so saving the form keeps it.
pub fn icon_choices(current: Option<&str>) -> (Vec<Choice>, usize) {
    use lumen_core::providers::quicklinks::ICON_DEFAULT;

    let mut choices = vec![
        Choice {
            value: None,
            label: "The website's icon".into(),
        },
        Choice {
            value: Some(ICON_DEFAULT.into()),
            label: "Lumen's link icon".into(),
        },
    ];
    match current.map(str::trim).filter(|value| !value.is_empty()) {
        None => (choices, 0),
        Some(ICON_DEFAULT) => (choices, 1),
        Some(other) => {
            choices.push(Choice {
                value: Some(other.to_string()),
                label: other.to_string(),
            });
            (choices, 2)
        }
    }
}

pub struct Select {
    focus: FocusHandle,
    choices: Vec<Choice>,
    selected: usize,
    /// The highlighted option while the list is open.
    open: Option<usize>,
    /// Whether the list opens above the field rather than below it.
    upward: bool,
    /// Drawn as a settings row's control (F15): its value and a chevron, no
    /// box, at the row's right edge, the list hanging from that edge.
    compact: bool,
    scroll: ScrollHandle,
    colors: Colors,
}

impl Select {
    pub fn new(choices: Vec<Choice>, selected: usize, colors: Colors, cx: &mut App) -> Self {
        Self {
            focus: cx.focus_handle(),
            selected: selected.min(choices.len().saturating_sub(1)),
            choices,
            open: None,
            upward: false,
            compact: false,
            scroll: ScrollHandle::new(),
            colors,
        }
    }

    /// Open the list above the field. For a host whose window ends not far
    /// below it, as the launcher's does: GPUI moves a list that does not fit
    /// on one side to the other only if it fits there entirely, and otherwise
    /// lays it over the field it belongs to.
    pub fn open_upward(&mut self, upward: bool) {
        self.upward = upward;
    }

    /// Draw as a settings row's control: no box, at the right edge (F15).
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    pub fn value(&self) -> Option<&str> {
        self.choices
            .get(self.selected)
            .and_then(|choice| choice.value.as_deref())
    }

    /// What the list says for the chosen option.
    pub fn label(&self) -> Option<&str> {
        self.choices
            .get(self.selected)
            .map(|choice| choice.label.as_str())
    }

    /// Replace the options, keeping the selection where it was.
    pub fn set_choices(&mut self, choices: Vec<Choice>, selected: usize, cx: &mut Context<Self>) {
        self.selected = selected.min(choices.len().saturating_sub(1));
        self.choices = choices;
        self.open = None;
        cx.notify();
    }

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        self.colors = colors;
        cx.notify();
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        self.open = None;
        if index < self.choices.len() && index != self.selected {
            self.selected = index;
            cx.emit(SelectEvent::Changed);
        }
        cx.notify();
    }

    fn toggle(&mut self, _: &ToggleSelect, _: &mut Window, cx: &mut Context<Self>) {
        match self.open {
            Some(index) => self.choose(index, cx),
            None => {
                self.open = Some(self.selected);
                self.scroll.scroll_to_item(self.selected);
                cx.notify();
            }
        }
    }

    fn confirm(&mut self, _: &ConfirmSelect, _: &mut Window, cx: &mut Context<Self>) {
        match self.open {
            Some(index) => self.choose(index, cx),
            // Closed, Enter is the form's: save.
            None => cx.propagate(),
        }
    }

    fn close(&mut self, _: &CloseSelect, _: &mut Window, cx: &mut Context<Self>) {
        if self.open.take().is_some() {
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(index) = self.open else {
            // Closed, Up and Down are the form's: the next field.
            cx.propagate();
            return;
        };
        let last = self.choices.len().saturating_sub(1) as isize;
        let next = (index as isize + delta).clamp(0, last) as usize;
        self.open = Some(next);
        self.scroll.scroll_to_item(next);
        cx.notify();
    }

    fn up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, cx);
    }

    fn down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.step(1, cx);
    }
}

impl Render for Select {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors.clone();
        let focused = self.focus.is_focused(window);
        let label = self
            .choices
            .get(self.selected)
            .map(|choice| choice.label.clone())
            .unwrap_or_default();

        let trigger = div()
            .id("select-trigger")
            .h(px(layout::settings::FIELD_HEIGHT))
            .px(px(layout::settings::FIELD_PADDING))
            .flex()
            .items_center()
            .rounded(px(layout::settings::FIELD_RADIUS))
            .border_1()
            .map(|trigger| {
                if self.compact {
                    // No box: the value and its chevron, lit when focused.
                    trigger
                        .gap(px(layout::settings::FIELD_PADDING))
                        .border_color(if focused {
                            color(colors.accent)
                        } else {
                            gpui::transparent_black()
                        })
                        .when(focused || self.open.is_some(), |trigger| {
                            trigger.bg(color(colors.selection_background))
                        })
                } else {
                    trigger
                        .justify_between()
                        .border_color(color(if focused {
                            colors.accent
                        } else {
                            colors.border_subtle
                        }))
                        .bg(color(colors.surface_elevated))
                }
            })
            .text_size(type_scale::subtitle())
            .text_color(color(colors.text_primary))
            .cursor_pointer()
            .on_click(cx.listener(|select, event: &gpui::ClickEvent, window, cx| {
                // The mouse only; the keys are the keymap's. See the note on
                // the quicklink form's checkbox.
                if event.is_keyboard() {
                    return;
                }
                window.focus(&select.focus);
                select.toggle(&ToggleSelect, window, cx);
            }))
            .child(div().truncate().child(SharedString::from(label)))
            .child(
                div()
                    .flex_none()
                    .text_color(color(colors.text_muted))
                    .child("\u{25BE}"),
            );

        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::toggle))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::close))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .when(!self.compact, |select| select.w_full())
            .relative()
            .child(trigger)
            .when_some(self.open, |select, highlighted| {
                select.child(self.render_menu(highlighted, &colors, cx))
            })
    }
}

impl Select {
    fn render_menu(
        &self,
        highlighted: usize,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let rows = self.choices.iter().enumerate().map(|(index, choice)| {
            let lit = index == highlighted;
            div()
                .id(SharedString::from(format!("select-option-{index}")))
                .h(px(layout::select::ROW_HEIGHT))
                .flex_none()
                .px(px(layout::settings::FIELD_PADDING))
                .flex()
                .items_center()
                .rounded(px(layout::select::ROW_RADIUS))
                .text_size(type_scale::subtitle())
                .text_color(color(if lit {
                    colors.text_primary
                } else {
                    colors.text_muted
                }))
                .when(lit, |row| row.bg(color(colors.selection_background)))
                .cursor_pointer()
                .on_click(cx.listener(move |select, _, _, cx| select.choose(index, cx)))
                .child(
                    div()
                        .truncate()
                        .child(SharedString::from(choice.label.clone())),
                )
        });

        // Drawn after everything else, so the form's later rows do not paint
        // over it. Anchored at the field's top-left corner: from there the
        // list hangs below the field, or stands on top of it.
        let (corner, offset) = match (self.upward, self.compact) {
            (true, false) => (Corner::BottomLeft, -layout::select::GAP),
            (true, true) => (Corner::BottomRight, -layout::select::GAP),
            (false, compact) => (
                if compact {
                    Corner::TopRight
                } else {
                    Corner::TopLeft
                },
                layout::settings::FIELD_HEIGHT + layout::select::GAP,
            ),
        };
        let menu = deferred(
            anchored()
                .anchor(corner)
                .offset(point(px(0.0), px(offset)))
                .child(
                    div()
                        .id("select-menu")
                        .occlude()
                        .track_scroll(&self.scroll)
                        .overflow_y_scroll()
                        .max_h(px(layout::select::MAX_HEIGHT))
                        .w(px(layout::select::MENU_WIDTH))
                        .p(px(layout::select::PADDING))
                        .flex()
                        .flex_col()
                        .rounded(px(layout::settings::FIELD_RADIUS))
                        .border_1()
                        .border_color(color(colors.border_strong))
                        .bg(color(colors.surface_elevated))
                        .shadow_md()
                        .on_mouse_down_out(cx.listener(|select, _, _, cx| {
                            select.open = None;
                            cx.notify();
                        }))
                        .children(rows),
                ),
        )
        .with_priority(1);
        // Compact, the list hangs from the trigger's right edge: anchored
        // from a point laid out there, since an anchor's origin is its own.
        div()
            .absolute()
            .top_0()
            .when(self.compact, |anchor| anchor.right_0())
            .when(!self.compact, |anchor| anchor.left_0())
            .child(menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_platform::AppChoice;

    fn installed() -> Vec<String> {
        vec!["Zen".into(), "Brave".into(), "Helium".into()]
    }

    #[test]
    fn nothing_saved_is_the_default_browser() {
        let (choices, selected) = browser_choices(None, &installed());
        assert_eq!(selected, 0);
        assert_eq!(choices[0].value, None);
        assert_eq!(choices.len(), 4);

        let (_, selected) = browser_choices(Some("  "), &installed());
        assert_eq!(selected, 0);
    }

    /// Written by hand in lower case, it is still the installed Brave.
    #[test]
    fn a_saved_name_selects_the_browser_it_opens_in() {
        let (choices, selected) = browser_choices(Some("brave"), &installed());
        assert_eq!(choices[selected].value.as_deref(), Some("Brave"));
        assert_eq!(choices.len(), 4);
    }

    /// Uninstalled, or a typo: kept and shown, never silently replaced.
    #[test]
    fn a_saved_name_that_matches_nothing_is_kept_and_marked() {
        let (choices, selected) = browser_choices(Some("Opera"), &installed());
        assert_eq!(choices.len(), 5);
        assert_eq!(choices[selected].value.as_deref(), Some("Opera"));
        assert_eq!(choices[selected].label, "Opera (not installed)");
    }

    #[test]
    fn the_icon_choice_reads_the_file_and_keeps_what_it_does_not_know() {
        assert_eq!(icon_choices(None).1, 0);
        assert_eq!(icon_choices(Some("default")).1, 1);

        let (choices, selected) = icon_choices(Some("emoji:rocket"));
        assert_eq!(choices[selected].value.as_deref(), Some("emoji:rocket"));
    }

    fn apps() -> Vec<AppChoice> {
        [
            ("C:\\Code.exe", "Visual Studio Code"),
            ("Notepad", "Notepad"),
        ]
        .map(|(id, name)| AppChoice {
            id: id.into(),
            name: name.into(),
        })
        .to_vec()
    }

    /// A new link to a file opens with the system's default app, listed
    /// first, and every app is stored as `app:` and its id.
    #[test]
    fn a_new_path_link_selects_the_default_app() {
        let (choices, selected) = app_choices(None, &apps());
        assert_eq!(selected, 0);
        assert_eq!(
            choices,
            [
                Choice {
                    value: Some("app:C:\\Code.exe".into()),
                    label: "Visual Studio Code".into(),
                },
                Choice {
                    value: Some("app:Notepad".into()),
                    label: "Notepad".into(),
                },
            ]
        );
        // A browser's name, left from when the link was a URL, is not an app.
        assert_eq!(app_choices(Some("Brave"), &apps()), (choices, 0));
    }

    #[test]
    fn a_stored_app_is_selected_when_editing() {
        let (choices, selected) = app_choices(Some("app:Notepad"), &apps());
        assert_eq!(choices.len(), 2);
        assert_eq!(choices[selected].value.as_deref(), Some("app:Notepad"));
    }

    /// Uninstalled since: kept, so saving another field does not change the
    /// app the link opens with.
    #[test]
    fn a_stored_app_that_is_not_listed_is_kept() {
        let (choices, selected) = app_choices(Some("app:C:\\Old.exe"), &apps());
        assert_eq!(choices.len(), 3);
        assert_eq!(choices[selected].value.as_deref(), Some("app:C:\\Old.exe"));
        assert_eq!(choices[selected].label, "C:\\Old.exe (not found)");
    }

    /// Before the list arrives, or when it cannot be read: the default app,
    /// and a stored one still selected.
    #[test]
    fn with_no_apps_known_yet_the_default_or_the_stored_one_is_offered() {
        let (choices, selected) = app_choices(None, &[]);
        assert_eq!(selected, 0);
        assert_eq!(choices[0].value, None);
        assert_eq!(choices.len(), 1);

        let (choices, selected) = app_choices(Some("app:Notepad"), &[]);
        assert_eq!(choices[selected].value.as_deref(), Some("app:Notepad"));
        assert_eq!(choices[selected].label, "Notepad");
    }

    /// Before the browser list arrives, a saved one must still be selected.
    #[test]
    fn with_no_browsers_known_yet_the_saved_one_is_still_selected() {
        let (choices, selected) = browser_choices(Some("Brave"), &[]);
        assert_eq!(choices[selected].value.as_deref(), Some("Brave"));
        assert_eq!(choices[selected].label, "Brave");
    }
}
