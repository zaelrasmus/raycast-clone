//! The Appearance page of the settings window (F15).
//!
//! The palette, the style, how the empty root list is arranged, and which of
//! its headers are drawn. Each choice is written as it is made -- there is
//! nothing to save -- because the daemon applies `config.json` as it changes,
//! and seeing the launcher change is the point of choosing a look.
//!
//! The arrangements offered by name are the four F15 settled on. Anything
//! else written by hand is shown as *Custom* and left alone until another is
//! chosen; its headers can still be turned on and off here.

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, FocusHandle, SharedString, Window, div, px};
use lumen_core::appearance::{self, Appearance, Arrangement};
use lumen_core::layout::settings as metrics;
use lumen_core::root::{RESULTS, RootConfig};
use lumen_core::theme::Colors;

use crate::quicklink_form::{NextField, PreviousField, TOGGLE_CONTEXT, ToggleFallback};
use crate::select::{Choice, Select, SelectEvent};
use crate::settings_ui as ui;
use crate::styling::color;
use crate::translate_page::FORM_CONTEXT;

/// The value of the *Custom* arrangement choice, which writes nothing.
const CUSTOM: &str = "custom";

/// One header's checkbox: a section's, by its index, or the typed list's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Header {
    Section(usize),
    Results,
}

pub struct AppearancePage {
    path: PathBuf,
    themes_dir: PathBuf,
    styles_dir: PathBuf,
    colors: Colors,
    /// Why the file could not be read. Nothing is written until it is fixed.
    problem: Option<String>,
    /// The keys as the file last read had them.
    current: Option<Appearance>,
    theme: Entity<Select>,
    style: Entity<Select>,
    arrangement: Entity<Select>,
    /// What the folders held at the last read, to tell when the dropdowns
    /// need their choices again.
    themes: Vec<(String, String)>,
    styles: Vec<(String, String)>,
    /// One per header, in the order they are drawn.
    headers: Vec<(Header, FocusHandle)>,
    error: Option<String>,
    status: Option<SharedString>,
}

impl AppearancePage {
    pub fn new(
        path: PathBuf,
        themes_dir: PathBuf,
        styles_dir: PathBuf,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> Self {
        let empty =
            |cx: &mut Context<Self>| cx.new(|cx| Select::new(Vec::new(), 0, colors.clone(), cx));
        let (theme, style, arrangement) = (empty(cx), empty(cx), empty(cx));
        Self {
            path,
            themes_dir,
            styles_dir,
            colors,
            problem: None,
            current: None,
            theme,
            style,
            arrangement,
            themes: Vec::new(),
            styles: Vec::new(),
            headers: Vec::new(),
            error: None,
            status: None,
        }
    }

    /// Re-read the file and the two folders, off the UI thread.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        let themes_dir = self.themes_dir.clone();
        let styles_dir = self.styles_dir.clone();
        cx.spawn(async move |this, cx| {
            let (read, themes, styles) = cx
                .background_executor()
                .spawn(async move {
                    (
                        appearance::read(&path),
                        appearance::installed(&themes_dir),
                        appearance::installed(&styles_dir),
                    )
                })
                .await;
            this.update(cx, |page, cx| {
                match read {
                    Ok(current) => {
                        page.problem = None;
                        page.rebuild(current, themes, styles, cx);
                    }
                    Err(error) => {
                        log::warn!("the appearance page cannot read the file: {error:#}");
                        page.problem = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn rebuild(
        &mut self,
        current: Appearance,
        themes: Vec<(String, String)>,
        styles: Vec<(String, String)>,
        cx: &mut Context<Self>,
    ) {
        // A dropdown is built again only when what it shows is not what the
        // file says, or its choices changed: built again, it is a new
        // element, and the focus a keyboard choice left on it would be lost.
        if themes != self.themes || self.theme.read(cx).value() != Some(current.theme.as_str()) {
            self.theme = named_select(&themes, &current.theme, &self.colors, cx);
            cx.subscribe(&self.theme, |page, select, SelectEvent::Changed, cx| {
                if let Some(name) = select.read(cx).value().map(str::to_string) {
                    let done = format!("Using the {name} palette");
                    page.write(done, move |path| appearance::set_theme(path, &name), cx);
                }
            })
            .detach();
        }
        if styles != self.styles || self.style.read(cx).value() != Some(current.style.as_str()) {
            self.style = named_select(&styles, &current.style, &self.colors, cx);
            cx.subscribe(&self.style, |page, select, SelectEvent::Changed, cx| {
                if let Some(name) = select.read(cx).value().map(str::to_string) {
                    let done = format!("Using the {name} style");
                    page.write(done, move |path| appearance::set_style(path, &name), cx);
                }
            })
            .detach();
        }
        let shown = Arrangement::of(&current.root).map_or(CUSTOM, Arrangement::label);
        if self.arrangement.read(cx).value() != Some(shown) {
            self.arrangement = arrangement_select(&current.root, &self.colors, cx);
            cx.subscribe(
                &self.arrangement,
                |page, select, SelectEvent::Changed, cx| {
                    let chosen = select.read(cx).value().and_then(|value| {
                        Arrangement::ALL
                            .into_iter()
                            .find(|arrangement| arrangement.label() == value)
                    });
                    if let (Some(arrangement), Some(current)) = (chosen, &page.current) {
                        let root = arrangement.apply(&current.root);
                        page.write_root(root, "Arranged".into(), cx);
                    }
                },
            )
            .detach();
        }
        self.themes = themes;
        self.styles = styles;

        // Keep each header's focus where the header survives, so a click
        // that rewrites the file does not throw the focus away.
        let old = std::mem::take(&mut self.headers);
        let focus_for = |header: Header, cx: &mut Context<Self>| {
            old.iter()
                .find(|(each, _)| *each == header)
                .map_or_else(|| cx.focus_handle(), |(_, focus)| focus.clone())
        };
        let mut headers: Vec<(Header, FocusHandle)> = (0..current.root.sections.len())
            .map(|index| {
                let header = Header::Section(index);
                (header, focus_for(header, cx))
            })
            .collect();
        headers.push((Header::Results, focus_for(Header::Results, cx)));
        self.headers = headers;
        self.current = Some(current);
    }

    /// Write one change through `change`, off the UI thread, then read the
    /// file back so the page shows what it now says.
    fn write(
        &mut self,
        done: String,
        change: impl FnOnce(&std::path::Path) -> anyhow::Result<()> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.problem.is_some() {
            return;
        }
        let path = self.path.clone();
        self.error = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { change(&path) })
                .await;
            this.update(cx, |page, cx| {
                match result {
                    Ok(()) => page.status = Some(done.into()),
                    Err(error) => {
                        log::warn!("could not save the appearance: {error:#}");
                        page.error = Some(format!("{error:#}"));
                    }
                }
                page.reload(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn write_root(&mut self, root: RootConfig, done: String, cx: &mut Context<Self>) {
        self.write(done, move |path| appearance::set_root(path, &root), cx);
    }

    fn flip(&mut self, header: Header, cx: &mut Context<Self>) {
        let Some(current) = &self.current else {
            return;
        };
        let mut root = current.root.clone();
        let (title, on) = match header {
            Header::Section(index) => {
                let Some(section) = root.sections.get_mut(index) else {
                    return;
                };
                section.header = !section.header;
                (section.title.clone(), section.header)
            }
            Header::Results => {
                root.results_header = !root.results_header;
                (RESULTS.to_string(), root.results_header)
            }
        };
        let done = format!("{title} header {}", if on { "shown" } else { "hidden" });
        self.write_root(root, done, cx);
    }

    fn header_on(&self, header: Header) -> bool {
        let Some(current) = &self.current else {
            return false;
        };
        match header {
            Header::Section(index) => current
                .root
                .sections
                .get(index)
                .is_some_and(|section| section.header),
            Header::Results => current.root.results_header,
        }
    }

    fn header_title(&self, header: Header) -> String {
        match header {
            Header::Section(index) => self
                .current
                .as_ref()
                .and_then(|current| current.root.sections.get(index))
                .map(|section| section.title.clone())
                .unwrap_or_default(),
            Header::Results => format!("{RESULTS}, while typing"),
        }
    }

    // -- Focus ----------------------------------------------------------------

    /// Everything Tab visits, in order.
    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        let mut stops = vec![
            self.theme.read(cx).focus_handle().clone(),
            self.style.read(cx).focus_handle().clone(),
            self.arrangement.read(cx).focus_handle().clone(),
        ];
        stops.extend(self.headers.iter().map(|(_, focus)| focus.clone()));
        stops
    }

    pub fn focus(&self, window: &mut Window, cx: &App) {
        window.focus(self.theme.read(cx).focus_handle());
    }

    fn step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let stops = self.stops(cx);
        let at = stops
            .iter()
            .position(|focus| focus.is_focused(window))
            .unwrap_or(0);
        let next = if forward {
            (at + 1) % stops.len()
        } else {
            (at + stops.len() - 1) % stops.len()
        };
        window.focus(&stops[next]);
        cx.notify();
    }

    fn next_field(&mut self, _: &NextField, window: &mut Window, cx: &mut Context<Self>) {
        self.step(true, window, cx);
    }

    fn previous_field(&mut self, _: &PreviousField, window: &mut Window, cx: &mut Context<Self>) {
        self.step(false, window, cx);
    }

    fn toggle(&mut self, _: &ToggleFallback, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(header) = self
            .headers
            .iter()
            .find(|(_, focus)| focus.is_focused(window))
            .map(|(header, _)| *header)
        {
            self.flip(header, cx);
        }
    }

    // -- What the host asks ---------------------------------------------------

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        for select in [&self.theme, &self.style, &self.arrangement] {
            select.update(cx, |select, cx| select.set_colors(colors.clone(), cx));
        }
        self.colors = colors;
        cx.notify();
    }

    /// What was applied, or why it could not be; the window's status line
    /// shows it.
    pub fn status(&self) -> Option<SharedString> {
        self.error
            .clone()
            .map(SharedString::from)
            .or_else(|| self.status.clone())
    }
}

impl Render for AppearancePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if window.focused(cx).is_none() {
            self.focus(window, cx);
        }
        let colors = self.colors.clone();

        if let Some(problem) = &self.problem {
            return ui::page("appearance")
                .child(
                    div()
                        .text_size(px(metrics::text::HINT))
                        .text_color(color(colors.text_error))
                        .child(format!(
                            "{problem}

Fix config.json, and this page comes back by itself."
                        )),
                )
                .into_any_element();
        }

        let entity = cx.entity();
        let headers: Vec<AnyElement> = self
            .headers
            .clone()
            .into_iter()
            .enumerate()
            .map(|(index, (header, focus))| {
                let entity = entity.clone();
                let (title, description) = match header {
                    Header::Section(_) => (format!("{} header", self.header_title(header)), None),
                    Header::Results => (
                        format!("{RESULTS} header"),
                        Some("Over the list while you type".into()),
                    ),
                };
                ui::toggle_row(
                    ("header", index),
                    title,
                    description,
                    self.header_on(header),
                    &focus,
                    TOGGLE_CONTEXT,
                    window,
                    &colors,
                    move |_window, cx| entity.update(cx, |page, cx| page.flip(header, cx)),
                )
            })
            .collect();

        ui::page("appearance")
            .key_context(FORM_CONTEXT)
            .on_action(cx.listener(Self::next_field))
            .on_action(cx.listener(Self::previous_field))
            .on_action(cx.listener(Self::toggle))
            .child(ui::section_title("Look", &colors))
            .child(ui::card(
                [
                    ui::row(
                        "Palette",
                        Some("Colours and the typeface, from the themes folder".into()),
                        self.theme.clone(),
                        &colors,
                    ),
                    ui::row(
                        "Style",
                        Some("Sizes, spacing and layout, from the styles folder".into()),
                        self.style.clone(),
                        &colors,
                    ),
                ],
                &colors,
            ))
            .child(ui::section_title("Root list", &colors))
            .child(ui::card(
                [ui::row(
                    "Arrangement",
                    Some("What the list shows before you type".into()),
                    self.arrangement.clone(),
                    &colors,
                )],
                &colors,
            ))
            .child(ui::card(headers, &colors))
            .into_any_element()
    }
}

/// A dropdown of installed files, by the name written in each, with
/// `current` chosen. A name that is not installed is shown as itself, so the
/// page does not claim a different choice than the file makes.
fn named_select(
    installed: &[(String, String)],
    current: &str,
    colors: &Colors,
    cx: &mut App,
) -> Entity<Select> {
    let mut choices: Vec<Choice> = installed
        .iter()
        .map(|(stem, label)| Choice {
            value: Some(stem.clone()),
            label: label.clone(),
        })
        .collect();
    if !installed.iter().any(|(stem, _)| stem == current) {
        choices.push(Choice {
            value: None,
            label: format!("{current} (not installed)"),
        });
    }
    let selected = choices
        .iter()
        .position(|choice| choice.value.as_deref() == Some(current))
        .unwrap_or(choices.len().saturating_sub(1));
    cx.new(|cx| Select::new(choices, selected, colors.clone(), cx).compact())
}

fn arrangement_select(root: &RootConfig, colors: &Colors, cx: &mut App) -> Entity<Select> {
    let mut choices: Vec<Choice> = Arrangement::ALL
        .iter()
        .map(|arrangement| Choice {
            value: Some(arrangement.label().into()),
            label: arrangement.label().into(),
        })
        .collect();
    let selected = match Arrangement::of(root) {
        Some(arrangement) => Arrangement::ALL
            .iter()
            .position(|each| *each == arrangement)
            .unwrap_or(0),
        None => {
            choices.push(Choice {
                value: Some(CUSTOM.into()),
                label: "Custom, as config.json says".into(),
            });
            choices.len() - 1
        }
    };
    cx.new(|cx| Select::new(choices, selected, colors.clone(), cx).compact())
}
