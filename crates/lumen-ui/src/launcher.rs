//! The launcher view.
//!
//! # Keyboard handling (AD-15, I-14)
//!
//! Every command is a GPUI action bound from a keymap. There are no hand-written
//! key comparisons here with exactly one deliberate exception: inserting typed
//! characters, in [`Launcher::on_key_down`]. Text entry is not a command and
//! cannot be expressed as one -- there is no action named "the letter k" -- so
//! the character is read from the keystroke directly. Everything that *is* a
//! command goes through `on_action`.
//!
//! # Layout stability (I-10)
//!
//! Every row is exactly `layout::ROW_HEIGHT` tall whether or not it has a
//! subtitle, its icon square exists before its icon does, and the kind label on
//! the right is drawn from a fixed vocabulary. Nothing in a row can change its
//! own size in response to its content, which is what makes results replace each
//! other without the list moving underneath the cursor.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    Animation, AnimationExt as _, App, Context, Entity, FocusHandle, Focusable, KeyDownEvent,
    ScrollStrategy, SharedString, Styled, Task, UniformListScrollHandle, Window, actions, div,
    ease_out_quint, img, uniform_list,
};
use lumen_core::theme::Appearance;
use lumen_core::{
    Action, Candidate, Config, Frecency, Item, Paths, Providers, Search, Theme, layout,
};
use lumen_platform::Platform;

use crate::icons::IconCache;
use crate::styling::{color, metrics, type_scale};

actions!(
    lumen,
    [
        /// Move the selection down one row.
        SelectNext,
        /// Move the selection up one row.
        SelectPrevious,
        /// Jump to the first result.
        SelectFirst,
        /// Jump to the last result.
        SelectLast,
        /// Run the selected row's primary action.
        Activate,
        /// Hide the launcher, restoring focus to where it came from.
        Dismiss,
        /// Delete the character before the cursor.
        DeleteBackward,
        /// Delete the word before the cursor.
        DeleteWordBackward,
        /// Clear the whole query.
        ClearQuery,
        /// Insert the clipboard's text at the cursor.
        Paste,
        /// Show or hide the action panel for the selected row.
        ToggleActionPanel,
    ]
);

/// One result, held as an index into the index rather than a copy of it.
///
/// Re-ranking happens on every keystroke; cloning several hundred items each
/// time would spend the per-frame budget on allocation alone.
struct Hit {
    source: Source,
    /// Character offsets into the title that matched, for highlighting.
    highlights: Vec<u32>,
    score: u32,
}

/// Which list a hit came from.
///
/// Two lists rather than one because they have opposite lifetimes: the
/// application index is built once and reused for thousands of queries, while
/// provider results are rebuilt for every keystroke. Concatenating them per
/// keystroke would mean cloning several hundred prepared haystacks to add three
/// rows, which is exactly the kind of work the per-frame budget cannot absorb.
#[derive(Clone, Copy)]
enum Source {
    /// An index into the application index.
    Indexed(usize),
    /// An index into this keystroke's provider answers.
    Answer(usize),
}

/// How many results are kept behind the visible window.
///
/// Bounded because nobody scrolls to the four hundredth fuzzy match, and an
/// unbounded list makes the sort the most expensive thing in the frame.
const MAX_RESULTS: usize = 64;

pub struct Launcher {
    focus_handle: FocusHandle,
    platform: Arc<dyn Platform>,
    paths: Paths,
    config: Config,
    theme: Theme,
    icons: IconCache,

    /// The searchable index. Empty until the first enumeration completes, which
    /// is why the root list is allowed to be empty rather than treated as a bug.
    candidates: Arc<Vec<Candidate>>,
    /// This keystroke's provider answers, rebuilt on every rerank.
    answers: Vec<Item>,
    providers: Providers,
    search: Search,
    frecency: Frecency,

    query: String,
    /// Byte offset of the caret within `query`.
    cursor: usize,
    hits: Vec<Hit>,
    selected: usize,
    scroll: UniformListScrollHandle,

    /// Which action is highlighted in the action panel, when it is open.
    ///
    /// `None` means the panel is closed. Holding the selection here rather than
    /// a separate `bool` plus index makes the two impossible to disagree.
    action_panel: Option<usize>,

    /// How many times the window has been shown.
    ///
    /// Used as part of the appearance animation's element id, so that showing
    /// the launcher again replays the fade rather than resuming a finished one.
    appearances: usize,
    /// Kept alive so the theme watch keeps firing; never read directly.
    _theme_reload: Option<Task<()>>,
}

impl Launcher {
    pub fn new(
        platform: Arc<dyn Platform>,
        paths: Paths,
        config: Config,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Self {
        let frecency = Frecency::load(&paths.frecency_file());
        let icons = IconCache::new(platform.clone(), layout::ICON_RESOLUTION);

        let mut launcher = Self {
            focus_handle: cx.focus_handle(),
            platform,
            paths,
            config,
            theme,
            icons,
            candidates: Arc::new(Vec::new()),
            answers: Vec::new(),
            providers: Providers::builtin(),
            search: Search::new(),
            frecency,
            query: String::new(),
            cursor: 0,
            hits: Vec::new(),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            action_panel: None,
            appearances: 0,
            _theme_reload: None,
        };

        launcher.start_indexing(cx);
        launcher
    }

    /// Rebuild the index: applications, scripts and snippets.
    ///
    /// Public so the daemon can ask for it after the user edits a script or a
    /// snippet, rather than making them restart to see the change.
    pub fn reindex(&mut self, cx: &mut Context<Self>) {
        self.start_indexing(cx);
    }

    /// Build the application index off the UI thread.
    ///
    /// I-3: root search never waits on this. Until it lands the list is simply
    /// empty, and the user can already type.
    fn start_indexing(&mut self, cx: &mut Context<Self>) {
        let platform = self.platform.clone();
        let scripts_dir = self.paths.scripts_dir();
        let snippets_file = self.paths.snippets_file();

        cx.spawn(async move |this, cx| {
            let entries = cx
                .background_executor()
                .spawn(async move { platform.enumerate_applications() })
                .await;

            let entries = match entries {
                Ok(entries) => entries,
                Err(error) => {
                    log::error!("could not index applications: {error:#}");
                    return;
                }
            };

            log::info!("indexed {} applications", entries.len());

            // Script commands are read here, on the background task, because
            // scanning a folder and parsing headers is disk work -- and because
            // AD-08 requires the index to be built without running anything.
            let scripts = cx
                .background_executor()
                .spawn({
                    let dir = scripts_dir.clone();
                    async move { lumen_core::providers::scripts::scan(&dir) }
                })
                .await;

            if !scripts.is_empty() {
                log::info!("indexed {} script commands", scripts.len());
            }

            let snippets = cx
                .background_executor()
                .spawn({
                    let file = snippets_file.clone();
                    async move { lumen_core::providers::snippets::load(&file) }
                })
                .await;

            if !snippets.is_empty() {
                log::info!("indexed {} snippets", snippets.len());
            }

            // The provider catalogue joins the same index as the applications,
            // so Lumen's own commands are found by typing part of their name
            // exactly like anything else -- one notion of "matches", not two.
            let mut providers = Providers::builtin();
            providers.register(Box::new(lumen_core::providers::ScriptProvider::new(
                scripts,
            )));
            providers.register(Box::new(lumen_core::providers::SnippetProvider::new(
                snippets,
            )));
            let catalogue = providers.catalogue();

            // Preparing the haystacks is the expensive half, so it stays on the
            // background thread too: handing the UI raw entries would just move
            // the cost into the first keystroke.
            let candidates = cx
                .background_executor()
                .spawn(async move {
                    entries
                        .into_iter()
                        .map(Candidate::from_app)
                        .chain(catalogue.into_iter().map(Candidate::new))
                        .collect::<Vec<_>>()
                })
                .await;

            this.update(cx, |this, cx| {
                this.candidates = Arc::new(candidates);
                this.rerank(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Called when the window is shown, to reset to a clean search.
    ///
    /// The launcher always opens empty. Preserving the previous query means the
    /// user has to clear it before typing, every single time.
    pub fn on_shown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.clear();
        self.cursor = 0;
        self.selected = 0;
        self.appearances = self.appearances.wrapping_add(1);

        // Point GPUI's focus tree at this view. Without it GPUI has nothing to
        // resolve keystrokes against and the keymap of AD-15 never fires, even
        // once the operating system is delivering input to this window.
        //
        // Deliberately *not* `window.activate_window()`. Taking the foreground
        // is the platform's job, and it happens in `show_window` right after
        // this. GPUI additionally defers a placement of its own for a window
        // created hidden, and that placement lands after our `SetWindowPos` and
        // drags the window back to the rectangle it was created with -- which
        // is how the launcher ends up in the top-left corner instead of on the
        // monitor holding the cursor.
        window.focus(&self.focus_handle);

        self.rerank(cx);
        cx.notify();
    }

    pub fn apply_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    pub fn appearance(&self) -> Appearance {
        self.theme.appearance
    }

    /// How many rows the window should currently be sized for.
    pub fn visible_rows(&self) -> usize {
        self.hits.len().min(self.config.window.visible_rows)
    }

    fn rerank(&mut self, cx: &mut Context<Self>) {
        let previous_rows = self.visible_rows();

        // Answers run first and synchronously. Providers are contractually
        // instant (see `lumen_core::providers`), so this stays inside the frame
        // budget and I-3 is not at risk: nothing here waits on anything.
        self.answers = self.providers.answers(&self.query);

        let candidates = self.candidates.clone();
        let mut hits: Vec<Hit> = self
            .search
            .rank(&self.query, &candidates, &self.frecency, MAX_RESULTS)
            .into_iter()
            .map(|m| Hit {
                source: Source::Indexed(m.index),
                highlights: m.highlights,
                score: m.score,
            })
            .collect();

        // Sorted before the answers are prepended, so the answers keep the top
        // regardless of how well anything else scored.
        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| self.title_of(a).cmp(self.title_of(b)))
        });

        // An answer *is* what the user asked for -- it was computed from their
        // exact query -- so it goes above every fuzzy guess rather than
        // competing with them on a score it has no meaningful way to earn.
        let answers = (0..self.answers.len()).map(|index| Hit {
            source: Source::Answer(index),
            highlights: Vec::new(),
            score: u32::MAX,
        });
        hits.splice(0..0, answers);

        hits.truncate(MAX_RESULTS);
        self.hits = hits;

        // The selection always returns to the top on a new query. Trying to keep
        // it on "the same item" across a re-rank is the kind of cleverness that
        // makes a launcher feel unpredictable.
        self.selected = 0;

        // A panel describing the previous row must not survive into the next
        // one, or Enter runs an action the user can no longer see.
        self.action_panel = None;

        // Only on an actual change: emitting on every keystroke would ask the
        // platform to resize the window to the size it already has.
        let rows = self.visible_rows();
        if rows != previous_rows {
            cx.emit(LauncherEvent::RowsChanged(rows));
        }
    }

    fn item(&self, hit: &Hit) -> &Item {
        match hit.source {
            Source::Indexed(index) => &self.candidates[index].item,
            Source::Answer(index) => &self.answers[index],
        }
    }

    fn title_of(&self, hit: &Hit) -> &str {
        &self.item(hit).title
    }

    fn selected_item(&self) -> Option<&Item> {
        self.hits.get(self.selected).map(|hit| self.item(hit))
    }

    // -- Commands ----------------------------------------------------------

    /// The actions available for the selected row.
    fn available_actions(&self) -> Vec<Action> {
        self.selected_item()
            .map(|item| item.actions.clone())
            .unwrap_or_default()
    }

    fn toggle_action_panel(
        &mut self,
        _: &ToggleActionPanel,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.action_panel = match self.action_panel {
            Some(_) => None,
            // Opens on the *second* action, not the first: the first is what
            // Enter already does, so landing on it would make the panel's own
            // Enter a no-op and waste the gesture.
            None if self.available_actions().len() > 1 => Some(1),
            None if !self.available_actions().is_empty() => Some(0),
            None => None,
        };
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        // While the panel is open the arrow keys move within it. Two lists on
        // screen and one set of arrow keys: whichever is in front wins.
        if let Some(selected) = self.action_panel {
            let count = self.available_actions().len();
            if count > 0 {
                self.action_panel = Some((selected + 1) % count);
                cx.notify();
            }
            return;
        }

        if self.hits.is_empty() {
            return;
        }
        // Wrapping, both ways. Pressing down at the bottom of a short list and
        // having nothing happen is the kind of dead end that makes a keyboard
        // interface feel unfinished.
        self.selected = (self.selected + 1) % self.hits.len();
        self.reveal_selection(cx);
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(selected) = self.action_panel {
            let count = self.available_actions().len();
            if count > 0 {
                self.action_panel = Some(selected.checked_sub(1).unwrap_or(count - 1));
                cx.notify();
            }
            return;
        }

        if self.hits.is_empty() {
            return;
        }
        self.selected = self.selected.checked_sub(1).unwrap_or(self.hits.len() - 1);
        self.reveal_selection(cx);
    }

    fn select_first(&mut self, _: &SelectFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.selected = 0;
        self.reveal_selection(cx);
    }

    fn select_last(&mut self, _: &SelectLast, _: &mut Window, cx: &mut Context<Self>) {
        self.selected = self.hits.len().saturating_sub(1);
        self.reveal_selection(cx);
    }

    fn reveal_selection(&mut self, cx: &mut Context<Self>) {
        // Moving the row selection closes the panel: it belonged to the row the
        // user has just moved off.
        self.action_panel = None;
        // F2 acceptance criterion: the list jumps directly to the selection.
        // There is no smooth scrolling, because smooth scrolling in a list being
        // driven by arrow keys is motion the user did not ask for.
        self.scroll
            .scroll_to_item(self.selected, ScrollStrategy::Top);
        cx.notify();
    }

    fn activate(&mut self, _: &Activate, _: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.selected_item() else {
            return;
        };

        // Whichever action the panel is pointing at, or the primary when it is
        // closed. Falling back to the primary rather than doing nothing means a
        // panel left open on a row whose actions have changed still does the
        // obvious thing.
        let action = match self.action_panel {
            Some(index) => item.actions.get(index).cloned().unwrap_or(item.primary()),
            None => item.primary(),
        };
        let id = item.id.clone();
        self.action_panel = None;

        // Record and persist before launching. A launch that hands the
        // foreground to another application immediately is a launch whose
        // bookkeeping is easy to lose.
        self.frecency.record(&id);
        if let Err(error) = self.frecency.save(&self.paths.frecency_file()) {
            log::warn!("could not persist usage statistics: {error:#}");
        }

        cx.emit(LauncherEvent::Dismissed);

        // Lumen's own commands open windows or stop the process, which is the
        // shell's business and not the list's (I-4 lives out there). They go
        // back as an event; everything else the platform can do directly.
        if let Action::Builtin(command) = action {
            cx.emit(LauncherEvent::Builtin(command));
            return;
        }

        let platform = self.platform.clone();
        cx.background_spawn(async move {
            let outcome = match &action {
                Action::Launch(target) => platform.launch(target),
                Action::CopyToClipboard(text) => platform.set_clipboard_text(text),
                Action::RunScript(run) => run_script(platform.as_ref(), run),
                // Handled above, before this task was spawned.
                Action::Builtin(_) => Ok(()),
            };
            if let Err(error) = outcome {
                log::error!("could not run {action:?}: {error:#}");
            }
        })
        .detach();
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(LauncherEvent::Dismissed);
    }

    fn delete_backward(&mut self, _: &DeleteBackward, _: &mut Window, cx: &mut Context<Self>) {
        let Some(previous) = self.previous_boundary() else {
            return;
        };
        self.query.replace_range(previous..self.cursor, "");
        self.cursor = previous;
        self.rerank(cx);
        cx.notify();
    }

    fn delete_word_backward(
        &mut self,
        _: &DeleteWordBackward,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start = word_start_before(&self.query, self.cursor);
        self.query.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.rerank(cx);
        cx.notify();
    }

    fn clear_query(&mut self, _: &ClearQuery, _: &mut Window, cx: &mut Context<Self>) {
        // Escape unwinds one layer at a time: the panel, then the query, then
        // the launcher. One key, and it always means "back" -- never "back two
        // steps because the first one happened to be empty".
        if self.action_panel.take().is_some() {
            cx.notify();
            return;
        }

        // Escape on a non-empty query clears it; on an empty one it dismisses.
        if self.query.is_empty() {
            cx.emit(LauncherEvent::Dismissed);
            return;
        }
        self.query.clear();
        self.cursor = 0;
        self.rerank(cx);
        cx.notify();
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        match self.platform.clipboard_text() {
            Ok(Some(text)) => {
                // A pasted newline would silently become part of the query and
                // match nothing, with no visible reason why.
                let text = text.replace(['\r', '\n'], " ");
                self.insert(&text, cx);
            }
            Ok(None) => {}
            Err(error) => log::warn!("could not read the clipboard: {error:#}"),
        }
    }

    /// The one place raw keystrokes are inspected, and only for text.
    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(character) = event.keystroke.key_char.as_ref() else {
            return;
        };
        // Anything with a modifier is a command, and commands are actions. A
        // control chord that produced a character must not be typed into the
        // field.
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        if character.chars().any(|c| c.is_control()) {
            return;
        }

        self.insert(character, cx);
    }

    fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        self.query.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.rerank(cx);
        cx.notify();
    }

    fn previous_boundary(&self) -> Option<usize> {
        previous_char_boundary(&self.query, self.cursor)
    }
}

/// The byte offset of the character before `cursor`.
///
/// Character, not byte: backspacing through an accented letter or an emoji must
/// remove the whole thing rather than half of its encoding, which is the
/// difference between deleting a character and corrupting the string.
fn previous_char_boundary(text: &str, cursor: usize) -> Option<usize> {
    if cursor == 0 {
        return None;
    }
    text[..cursor]
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
}

/// The byte offset where the word before `cursor` starts.
///
/// Trailing whitespace is skipped first, so deleting just after a space removes
/// the word rather than only the space -- pressing the shortcut twice to get rid
/// of one word is the thing that makes it feel broken.
fn word_start_before(text: &str, cursor: usize) -> usize {
    let trimmed = text[..cursor].trim_end();
    trimmed
        .rfind(char::is_whitespace)
        .map(|index| index + 1)
        .unwrap_or(0)
}

/// What the launcher asks the window to do.
///
/// An event rather than a direct call because showing and hiding is the
/// platform's business (I-4), and the view must not reach across that boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherEvent {
    Dismissed,
    /// One of Lumen's own commands was run.
    Builtin(lumen_core::Builtin),
    /// The number of visible rows changed, so the window should be resized to
    /// hug its content.
    RowsChanged(usize),
}

impl gpui::EventEmitter<LauncherEvent> for Launcher {}

impl Focusable for Launcher {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Launcher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &self.theme.colors;

        // The surface must stay translucent for the system backdrop to be
        // visible at all -- the "indispensable condition" of section 7. When the
        // backdrop is off, the same token is simply opaque.
        let background = if self.config.backdrop.wants_translucent_surface() {
            colors.surface_background
        } else {
            colors.surface_background.with_alpha(255)
        };

        let mut root = div()
            .key_context("Launcher")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_first))
            .on_action(cx.listener(Self::select_last))
            .on_action(cx.listener(Self::activate))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::delete_backward))
            .on_action(cx.listener(Self::delete_word_backward))
            .on_action(cx.listener(Self::clear_query))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::toggle_action_panel))
            .on_key_down(cx.listener(Self::on_key_down))
            .size_full()
            .flex()
            .flex_col()
            .rounded(metrics::window_radius())
            .border_1()
            .border_color(color(colors.border_subtle))
            .bg(color(background))
            .text_color(color(colors.text_primary));

        if let Some(family) = &self.theme.font_family {
            root = root.font_family(SharedString::from(family.clone()));
        }

        // The panel replaces the list when it is open: one region, one height,
        // and no window resize to swap between them.
        let body = match self.render_action_panel(cx) {
            Some(panel) => panel,
            None => self.render_results(cx).into_any_element(),
        };

        root.child(self.render_search_field(window))
            .child(body)
            .child(self.render_footer())
            // F2: a fade over roughly 120 ms. Nothing more.
            //
            // Driven by GPUI's animation element rather than by measuring
            // elapsed time inside `render` and asking for another frame: a
            // repaint requested from within a render pass is not guaranteed to
            // arrive, and when it does not, the window is left stranded at
            // whatever opacity the first frame happened to compute.
            //
            // The element id carries `appearances`, so each show restarts the
            // animation instead of replaying a finished one.
            .with_animation(
                ("appear", self.appearances),
                Animation::new(layout::motion::APPEAR).with_easing(ease_out_quint()),
                |root, delta| root.opacity(delta),
            )
    }
}

impl Launcher {
    fn render_search_field(&self, window: &Window) -> impl IntoElement {
        let colors = &self.theme.colors;
        let focused = self.focus_handle.is_focused(window);

        let (before, after) = self.query.split_at(self.cursor);

        div()
            .flex()
            .items_center()
            .h(metrics::search_height())
            .px(metrics::gutter())
            .border_b_1()
            .border_color(color(colors.border_subtle))
            .text_size(type_scale::query())
            .child(if self.query.is_empty() {
                div()
                    .text_color(color(colors.text_placeholder))
                    .child("Search for apps and commands...")
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .items_center()
                    .child(before.to_string())
                    // A one-pixel caret drawn as an element rather than shaped
                    // into the text. Full text shaping with an input handler is
                    // what brings IME support, and it is deliberately deferred:
                    // see the note in `lib.rs`.
                    .child(
                        div()
                            .w(gpui::px(1.0))
                            .h(type_scale::query())
                            .bg(color(if focused {
                                colors.accent
                            } else {
                                colors.text_placeholder
                            })),
                    )
                    .child(after.to_string())
                    .into_any_element()
            })
    }

    /// The height of the region the list occupies.
    ///
    /// Shared with the action panel, which takes that region over, so the two
    /// cannot disagree and the window never has to resize to swap between them.
    fn results_height(&self) -> gpui::Pixels {
        metrics::row_height() * self.visible_rows() as f32 + metrics::list_padding() * 2.0
    }

    fn render_results(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.hits.is_empty() {
            return div().h(gpui::px(0.0)).into_any_element();
        }

        let count = self.hits.len();
        let entity = cx.entity();

        div()
            .py(metrics::list_padding())
            .h(self.results_height())
            .child(
                uniform_list(
                    "results",
                    count,
                    cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                        range
                            .map(|index| this.render_row(index, &entity, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(self.scroll.clone())
                .h_full(),
            )
            .into_any_element()
    }

    fn render_row(&self, index: usize, entity: &Entity<Self>, cx: &mut App) -> gpui::AnyElement {
        let colors = &self.theme.colors;
        let hit = &self.hits[index];
        let item = self.item(hit);
        let selected = index == self.selected;

        let icon = item
            .icon
            .as_ref()
            .and_then(|source| self.icons.get(source, cx));

        let entity = entity.clone();

        div()
            .id(index)
            .h(metrics::row_height())
            .mx(metrics::row_inset())
            .px(metrics::gutter() - metrics::row_inset())
            .flex()
            .items_center()
            .gap(metrics::icon_gap())
            .rounded(metrics::row_radius())
            .when(selected, |row| {
                row.bg(color(colors.selection_background))
                    .border_1()
                    .border_color(color(colors.selection_border))
            })
            // Not selected rows still reserve the border, or selecting a row
            // would shift its text by a pixel (I-10).
            .when(!selected, |row| {
                row.border_1().border_color(gpui::transparent_black())
            })
            .on_click(move |_event, window, cx| {
                entity.update(cx, |this, cx| {
                    this.selected = index;
                    this.activate(&Activate, window, cx);
                });
            })
            .child(
                // The icon square exists whether or not the icon has loaded, so
                // a late icon never moves the title (I-10).
                div()
                    .size(metrics::icon_size())
                    .flex_none()
                    .when_some(icon, |slot, image| slot.child(img(image).size_full())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_hidden()
                    .child(
                        div()
                            .text_size(type_scale::title())
                            .text_color(color(colors.text_primary))
                            .child(highlighted(&item.title, &hit.highlights, colors)),
                    )
                    .when_some(item.subtitle.as_ref(), |slot, subtitle| {
                        slot.child(
                            div()
                                .text_size(type_scale::subtitle())
                                .text_color(color(colors.text_muted))
                                .child(subtitle.clone()),
                        )
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(type_scale::footer())
                    .text_color(color(colors.text_muted))
                    .child(item.kind_label()),
            )
            .into_any_element()
    }

    fn render_footer(&self) -> impl IntoElement {
        let colors = &self.theme.colors;
        let verb = self
            .selected_item()
            .map(|item| item.primary().verb())
            .unwrap_or("Open");

        div()
            .flex()
            .items_center()
            .justify_between()
            .h(metrics::footer_height())
            .px(metrics::gutter())
            .border_t_1()
            .border_color(color(colors.border_subtle))
            .text_size(type_scale::footer())
            .text_color(color(colors.text_muted))
            .child(div().child(format!("{} results", self.hits.len())))
            .child(
                div()
                    .flex()
                    .gap(metrics::icon_gap())
                    .child(format!("{verb}  ⏎"))
                    // Advertised only when there is a second action to reach.
                    // A hint for a panel that would open empty teaches the user
                    // that the hint lies.
                    .when(self.available_actions().len() > 1, |footer| {
                        footer.child("Actions  Ctrl+K")
                    })
                    .child("Dismiss  Esc"),
            )
    }

    /// The action panel, when it is open.
    ///
    /// Drawn inside the launcher's own window rather than given one of its own:
    /// unlike the shortcut overlay it is an ordinary part of the list's
    /// interface, it wants the focus the launcher already has, and it would
    /// gain nothing from being a separate surface but the cost of being one.
    fn render_action_panel(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        use lumen_core::layout::action_panel as panel;

        let selected = self.action_panel?;
        let actions = self.available_actions();
        if actions.is_empty() {
            return None;
        }

        let colors = &self.theme.colors;
        let entity = cx.entity();

        // The panel takes the list's place rather than floating over it.
        //
        // An overlay was the first design and it does not survive contact with
        // the layout: the window is sized exactly to its content, so there is
        // no spare room for a floating element, and an absolutely positioned
        // one with only `right` and `bottom` set gives taffy no way to derive a
        // height -- it lays out at zero and is never seen. Replacing the list
        // reuses the one region whose height is already known to be right, and
        // it is arguably the better interaction anyway: the panel is a mode,
        // Escape leaves it, and nothing is hidden behind it.
        Some(
            div()
                .h(self.results_height())
                .p(gpui::px(panel::PADDING))
                .flex()
                .flex_col()
                .bg(color(colors.surface_elevated))
                .children(actions.into_iter().enumerate().map(|(index, action)| {
                    let entity = entity.clone();
                    div()
                        .id(("action", index))
                        .h(gpui::px(panel::ROW_HEIGHT))
                        .px(metrics::gutter())
                        .flex()
                        .items_center()
                        .justify_between()
                        .rounded(gpui::px(panel::ROW_RADIUS))
                        .text_size(type_scale::title())
                        .when(index == selected, |row| {
                            row.bg(color(colors.selection_background))
                                .text_color(color(colors.text_primary))
                        })
                        .when(index != selected, |row| {
                            row.text_color(color(colors.text_muted))
                        })
                        .cursor_pointer()
                        .on_click(move |_event, window, cx| {
                            entity.update(cx, |this, cx| {
                                this.action_panel = Some(index);
                                this.activate(&Activate, window, cx);
                            });
                        })
                        .child(action.verb())
                        // The first action is what Enter already does; saying so
                        // is what stops the panel looking like a list of things
                        // Enter cannot reach.
                        .when(index == 0, |row| {
                            row.child(
                                div()
                                    .text_size(type_scale::footer())
                                    .text_color(color(colors.text_muted))
                                    .child("⏎"),
                            )
                        })
                }))
                .into_any_element(),
        )
    }
}

/// Render a title with its matched characters emphasised.
///
/// Highlighting is what makes a fuzzy match legible: without it, a result that
/// matched on three scattered letters looks like it has no reason to be there.
fn highlighted(
    title: &str,
    highlights: &[u32],
    colors: &lumen_core::theme::Colors,
) -> gpui::AnyElement {
    if highlights.is_empty() {
        return div().child(title.to_string()).into_any_element();
    }

    // Runs rather than one element per character: a title of thirty characters
    // would otherwise be thirty elements, on every row, on every frame.
    let mut runs: Vec<(String, bool)> = Vec::new();
    for (index, character) in title.chars().enumerate() {
        let matched = highlights.binary_search(&(index as u32)).is_ok();
        match runs.last_mut() {
            Some((text, run_matched)) if *run_matched == matched => text.push(character),
            _ => runs.push((character.to_string(), matched)),
        }
    }

    div()
        .flex()
        .children(runs.into_iter().map(|(text, matched)| {
            let mut run = div().child(text);
            if matched {
                run = run.text_color(color(colors.match_highlight));
            }
            run
        }))
        .into_any_element()
}

/// Run a script command and deal with its output.
///
/// Off the UI thread, and after the launcher has been dismissed: a script may
/// take as long as it likes and nothing waits for it. That is also why `Copy`
/// is the only mode that does anything with the output -- by the time a script
/// finishes there is no list left to show it in, and the clipboard is where the
/// user wanted it anyway.
fn run_script(
    platform: &dyn Platform,
    run: &lumen_core::providers::ScriptRun,
) -> anyhow::Result<()> {
    use lumen_core::providers::ScriptMode;

    let output = platform.run_script(&run.path)?;

    match run.mode {
        ScriptMode::Silent => Ok(()),
        ScriptMode::Copy if output.is_empty() => {
            // Clearing the clipboard because a script printed nothing would
            // destroy whatever the user had copied earlier, which is a far
            // worse outcome than doing nothing.
            log::warn!("{} printed nothing to copy", run.path.display());
            Ok(())
        }
        ScriptMode::Copy => platform.set_clipboard_text(&output),
    }
}
#[cfg(test)]
mod tests {
    use super::{previous_char_boundary, word_start_before};

    use lumen_core::item::{Builtin, Kind};
    use lumen_core::{Action, Item};

    fn item_with(actions: Vec<Action>) -> Item {
        Item {
            id: "x".into(),
            title: "X".into(),
            subtitle: None,
            icon: None,
            keywords: Vec::new(),
            actions,
            kind: Kind::Application,
        }
    }

    /// The panel opens on the *second* action, because the first is what Enter
    /// already does.
    #[test]
    fn opening_the_panel_skips_the_primary_action() {
        let item = item_with(vec![
            Action::Builtin(Builtin::OpenSettings),
            Action::CopyToClipboard("x".into()),
        ]);

        let opened = opening_selection(item.actions.len());
        assert_eq!(opened, Some(1));
    }

    /// With only one action there is nothing the panel adds, so it lands on it
    /// rather than on nothing.
    #[test]
    fn a_single_action_still_opens_somewhere_valid() {
        assert_eq!(opening_selection(1), Some(0));
    }

    #[test]
    fn an_item_with_no_actions_does_not_open_a_panel() {
        assert_eq!(opening_selection(0), None);
    }

    /// Mirrors `toggle_action_panel`'s choice of starting index, kept beside it
    /// so the rule is checkable without standing up a GPUI app.
    fn opening_selection(action_count: usize) -> Option<usize> {
        match action_count {
            0 => None,
            1 => Some(0),
            _ => Some(1),
        }
    }

    #[test]
    fn an_application_offers_a_second_action_for_the_panel_to_show() {
        use lumen_platform::{AppEntry, LaunchTarget};

        let item = Item::from_app(AppEntry {
            id: "app:x".into(),
            name: "Firefox".into(),
            detail: None,
            target: LaunchTarget::ShellExecute("x".into()),
            icon: None,
            keywords: Vec::new(),
        });

        assert!(item.actions.len() > 1, "the panel would open empty");
        assert_eq!(item.primary().verb(), "Open");
        assert_eq!(item.actions[1], Action::CopyToClipboard("Firefox".into()));
    }

    #[test]
    fn backspace_removes_a_whole_character_not_one_byte() {
        let query = "café";
        let start = previous_char_boundary(query, query.len()).unwrap();
        assert_eq!(&query[..start], "caf");
    }

    #[test]
    fn backspace_at_the_start_has_nothing_to_remove() {
        assert_eq!(previous_char_boundary("anything", 0), None);
    }

    #[test]
    fn backspace_handles_characters_outside_the_basic_plane() {
        let query = "ok 🚀";
        let start = previous_char_boundary(query, query.len()).unwrap();
        assert_eq!(&query[..start], "ok ");
    }

    #[test]
    fn deleting_a_word_removes_the_word() {
        assert_eq!(word_start_before("visual studio code", 18), 14);
    }

    /// The case that separates a usable shortcut from an irritating one: just
    /// after a space, one press must remove the word before it, not the space.
    #[test]
    fn deleting_a_word_skips_trailing_whitespace_first() {
        assert_eq!(word_start_before("visual studio ", 14), 7);
    }

    #[test]
    fn deleting_the_only_word_clears_the_query() {
        assert_eq!(word_start_before("visual", 6), 0);
        assert_eq!(word_start_before("", 0), 0);
    }
}
