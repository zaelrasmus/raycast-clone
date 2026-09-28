//! The launcher view.
//!
//! # Keyboard handling (AD-15, I-14)
//!
//! Every command is a GPUI action bound from a keymap, and there are no
//! hand-written key comparisons here at all. The query is a
//! [`TextField`](crate::text_field::TextField), which receives typed text
//! through the system input handler and owns every editing command; the
//! launcher only hears that the text changed. Everything that is about the
//! *list* -- moving the selection, running a row, the action panel -- is bound
//! here, in the `Launcher` context that encloses the field.
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
    Animation, AnimationExt as _, App, Context, Entity, FocusHandle, Focusable, ScrollStrategy,
    SharedString, Styled, Subscription, Task, UniformListScrollHandle, Window, actions, div,
    ease_out_quint, img, svg, uniform_list,
};
use lumen_core::providers::QuicklinkProvider;
use lumen_core::root::{Aside, CLIPBOARD_ENTRIES, Hit};
use lumen_core::theme::Appearance;
use lumen_core::{
    Action, Candidate, Config, Frecency, Item, Paths, Providers, RootSearch, Theme, layout,
};
use lumen_platform::Platform;

use crate::icons::IconCache;
use crate::quicklink_form::{FormEvent, QuicklinkForm};
use crate::styling::{color, metrics, type_scale};
use crate::text_field::{TextField, TextFieldEvent};

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
        /// Clear the whole query.
        ClearQuery,
        /// Show or hide the action panel for the selected row.
        ToggleActionPanel,
        /// Type the selected quicklink's argument (F6).
        AskArgument,
    ]
);

/// What the search field says when it is empty and nothing else applies.
const ROOT_PLACEHOLDER: &str = "Search for apps and commands...";

/// The fallback for a view that did not name its own.
const FIND: &str = "Search...";

/// What the search field says while it holds a quicklink's argument.
const ARGUMENT_PLACEHOLDER: &str = "Type what to search for...";

/// A view's item, prepared for the host's matcher.
///
/// The same `Candidate` the root list uses, so that filtering a view is the
/// same matching as filtering applications -- one notion of "matches", not two.
fn candidate_for(item: &lumen_core::view::Item) -> Candidate {
    Candidate::new(Item {
        id: item.id.clone(),
        title: item.title.clone(),
        subtitle: item.subtitle.clone(),
        icon: None,
        keywords: item.keywords.clone(),
        actions: Vec::new(),
        kind: lumen_core::Kind::Command,
    })
}

/// One row of the action panel, whichever mode produced it.
struct PanelEntry {
    title: String,
    /// The key combination shown beside it, when there is one. Shown, never
    /// bound -- see `lumen_core::view::Action::shortcut`.
    shortcut: Option<String>,
}

/// What the launcher is currently showing.
///
/// A view takes over the list's region rather than opening a window of its own
/// (F4). That is what an extension's results are: the launcher, showing
/// somebody else's rows. Giving them a separate surface would mean a second
/// search field, a second focus owner and a second thing to dismiss.
enum Mode {
    Root,
    View(Box<ViewState>),
    /// Saving a new quicklink (F6). Like a view, it takes the list's place.
    Form(FormState),
}

/// The quicklink form, and the subscription that hears it finish.
struct FormState {
    form: Entity<QuicklinkForm>,
    _events: Subscription,
}

/// Where a view's events go.
///
/// A sink rather than a channel type, for the same reason
/// [`GestureSink`](lumen_platform::GestureSink) is one: the launcher must not
/// know what is on the other end. Today it is a subprocess; when the transport
/// is the component model it will be a call into a runtime, and nothing in the
/// view layer will notice.
#[derive(Clone)]
pub struct ViewSink(Arc<dyn Fn(lumen_core::view::Event) + Send + Sync>);

impl ViewSink {
    pub fn new(f: impl Fn(lumen_core::view::Event) + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    fn emit(&self, event: lumen_core::view::Event) {
        (self.0)(event)
    }
}

/// A view, and everything the launcher needs to draw and drive it.
struct ViewState {
    /// Where events go. `None` for the bundled example, which has nobody to
    /// tell.
    sink: Option<ViewSink>,
    /// True until the extension's first view arrives.
    ///
    /// A state rather than a wait: the launcher is already open, already
    /// focused and already accepting keystrokes. I-3 is about root search and
    /// this is not root search -- but the same discipline applies, because a
    /// launcher that freezes while a process starts is a launcher that feels
    /// broken whichever invariant technically covers it.
    loading: bool,
    view: lumen_core::view::View,
    /// The flattened rows after the current filter. Rebuilt when the query
    /// changes, not on every frame.
    rows: Vec<crate::view::Row>,
    /// GPUI's virtualised-list state. Replaced whenever `rows` changes length,
    /// because it caches measured heights by index.
    list: gpui::ListState,
    /// An index into the view's *unfiltered* flat item order, so that a row
    /// keeps its identity when the filter changes underneath it.
    selected: usize,
    /// The view's items prepared for matching, when the host is doing the
    /// filtering. Built once when the view opens.
    candidates: Arc<Vec<Candidate>>,
    /// The query the extension was last told about.
    ///
    /// Kept so that a re-rank triggered by something other than typing does not
    /// report a change that did not happen -- see `rerank`.
    sent_query: String,
}

pub struct Launcher {
    /// The query. Holds focus for the whole launcher: it is the one thing in
    /// the window that takes input, and everything else is reached through
    /// actions that bubble up from it.
    query_field: Entity<TextField>,
    /// Kept alive so edits keep arriving; never read.
    _query_changes: Subscription,
    platform: Arc<dyn Platform>,
    paths: Paths,
    config: Config,
    theme: Theme,
    icons: IconCache,

    /// What the root list is ranked from, and how (AD-03: the policy is in
    /// `core`, and this view only draws its hits).
    root: RootSearch,
    /// The clipboard history, once the daemon has opened it.
    ///
    /// Shared with the recorder rather than opened twice: one database, one
    /// writer, and a read that cannot see a half-written row.
    clipboard: Option<Arc<parking_lot::Mutex<lumen_core::clipboard::History>>>,

    /// A copy of the field's text, updated on every edit. Ranking reads it on
    /// every keystroke, and reaching into another entity for it each time
    /// would need an `App` everywhere a query is read.
    query: String,
    selected: usize,
    scroll: UniformListScrollHandle,

    /// The root list, or an extension's view (F4).
    mode: Mode,

    /// The quicklink whose argument the search field holds, if it does (F6).
    ///
    /// Beside `mode` rather than a variant of it, because it is still the root
    /// list -- the same rows, the same actions, the same panel -- narrowed to
    /// one link. A mode of its own would duplicate every branch that already
    /// says `Mode::Root`.
    argument: Option<lumen_core::providers::quicklinks::ArgumentRequest>,

    /// Which action is highlighted in the action panel, when it is open.
    ///
    /// `None` means the panel is closed. Holding the selection here rather than
    /// a separate `bool` plus index makes the two impossible to disagree.
    action_panel: Option<usize>,

    /// Whether the window has actually held focus since it was last shown.
    ///
    /// The guard that makes dismiss-on-focus-loss safe. Showing is two steps --
    /// the view resets, then the platform takes the foreground -- and the
    /// activation observer can report "not active" in between. Without this the
    /// launcher would dismiss itself on the way up, every time.
    focused_once: bool,
    /// Kept alive so the activation observer keeps firing; never read.
    _activation: Option<gpui::Subscription>,

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

        let query_field =
            cx.new(|cx| TextField::search(ROOT_PLACEHOLDER, theme.colors.clone(), cx));
        let query_changes = cx.subscribe(&query_field, |launcher, _, event, cx| match event {
            TextFieldEvent::Changed(text) => {
                launcher.query.clone_from(text);
                launcher.rerank(cx);
                cx.notify();
            }
            // Backspace in an empty argument takes the pill away, as it would
            // the character to its left.
            TextFieldEvent::DeletedPastStart => {
                if launcher.argument.is_some() {
                    launcher.leave_argument(cx);
                }
            }
        });

        let mut root = RootSearch::new(Providers::builtin(), frecency);
        root.set_visible_rows(config.window.visible_rows);

        let mut launcher = Self {
            query_field,
            _query_changes: query_changes,
            platform,
            paths,
            config,
            theme,
            icons,
            root,
            clipboard: None,
            query: String::new(),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            mode: Mode::Root,
            argument: None,
            focused_once: false,
            _activation: None,
            action_panel: None,
            appearances: 0,
            _theme_reload: None,
        };

        launcher.start_indexing(cx);
        launcher.refresh_files(cx);
        launcher
    }

    /// Give the launcher the clipboard history to read from.
    pub fn use_clipboard_history(
        &mut self,
        history: Arc<parking_lot::Mutex<lumen_core::clipboard::History>>,
        cx: &mut Context<Self>,
    ) {
        self.clipboard = Some(history);
        self.refresh_clipboard(cx);
    }

    /// Re-read the clipboard snapshot.
    ///
    /// Called by the daemon after it records an entry, rather than on a timer
    /// or when the window opens: the recorder is the only thing that knows the
    /// history changed, copying is rare, and doing the work then means the
    /// snapshot is already correct by the time anybody searches it.
    ///
    /// The read is a hundred short rows from an indexed table, but it is still
    /// disk I/O and matching runs inside the frame budget, so it happens on a
    /// background task exactly like the application index.
    pub fn refresh_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(history) = self.clipboard.clone() else {
            return;
        };

        cx.spawn(async move |this, cx| {
            let candidates = cx
                .background_executor()
                .spawn(async move {
                    let entries =
                        history
                            .lock()
                            .recent(CLIPBOARD_ENTRIES)
                            .unwrap_or_else(|error| {
                                log::warn!("could not read the clipboard history: {error:#}");
                                Vec::new()
                            });

                    lumen_core::providers::clipboard::items(entries)
                        .into_iter()
                        .map(Candidate::new)
                        .collect::<Vec<_>>()
                })
                .await;

            this.update(cx, |this, cx| {
                this.set_aside(Aside::Clipboard, candidates, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Re-read the files under the configured roots.
    ///
    /// Separate from `start_indexing` and started alongside it rather than
    /// inside it, because the two have nothing to do with each other and the
    /// file walk is much the slower of the two. Sequencing them would mean the
    /// application list -- the thing the launcher is mostly for -- waited on a
    /// walk of Documents.
    pub fn refresh_files(&mut self, cx: &mut Context<Self>) {
        let roots = self.config.files.roots();
        let limits = self.config.files.limits;

        if !self.config.files.enabled || roots.is_empty() {
            if self.config.files.enabled {
                log::info!("file search has no folders to look in");
            }
            // Emptied rather than left alone, so turning file search off in a
            // running daemon takes the files out of the results too.
            if !self.root.aside_is_empty(Aside::Files) {
                self.set_aside(Aside::Files, Vec::new(), cx);
            }
            return;
        }

        cx.spawn(async move |this, cx| {
            let started = std::time::Instant::now();
            let candidates = cx
                .background_executor()
                .spawn(async move {
                    lumen_core::providers::files::scan(&roots, limits)
                        .into_iter()
                        .map(Candidate::new)
                        .collect::<Vec<_>>()
                })
                .await;

            log::info!(
                "indexed {} files in {} ms",
                candidates.len(),
                started.elapsed().as_millis()
            );

            this.update(cx, |this, cx| {
                this.set_aside(Aside::Files, candidates, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Replace one aside and re-rank against it.
    ///
    /// An aside is a *root-list* index, so when a view is open this stores it
    /// and stops. Re-ranking a list nobody is looking at is wasted work, and --
    /// as the first extension proved -- re-ranking is not free of consequence:
    /// it is on the path that tells an extension about the query.
    fn set_aside(&mut self, aside: Aside, candidates: Vec<Candidate>, cx: &mut Context<Self>) {
        self.root.replace_aside(aside, candidates);

        if matches!(self.mode, Mode::View(_)) {
            return;
        }

        self.rerank(cx);
        cx.notify();
    }

    /// Rebuild the index: applications, scripts, snippets and clipboard history.
    ///
    /// Public so the daemon can ask for it after the user edits a script or a
    /// snippet, rather than making them restart to see the change.
    pub fn reindex(&mut self, cx: &mut Context<Self>) {
        self.start_indexing(cx);
        self.refresh_files(cx);
    }

    /// Rebuild the application index after something was installed or removed.
    ///
    /// The same work as the index built at startup -- scripts, snippets and
    /// extensions included, since they share the one index -- and like it,
    /// never waited on (I-3). The old list stays up until the new one lands.
    pub fn reindex_applications(&mut self, cx: &mut Context<Self>) {
        self.start_indexing(cx);
    }

    /// Re-read `quicklinks.json` after it was saved (F6).
    ///
    /// The whole index rather than the links alone: as catalogue entries they
    /// live in it, and one index built one way is simpler than a second list
    /// with its own ranking rules. It is off the UI thread and the old index
    /// stays up until the new one lands (I-3).
    pub fn reload_quicklinks(&mut self, cx: &mut Context<Self>) {
        self.start_indexing(cx);
    }

    /// Build the application index off the UI thread.
    ///
    /// I-3: root search never waits on this. Until it lands the list is simply
    /// empty, and the user can already type.
    fn start_indexing(&mut self, cx: &mut Context<Self>) {
        let platform = self.platform.clone();
        let paths = self.paths.clone();
        let current = self.root.index();

        cx.spawn(async move |this, cx| {
            // Built and compared on the background thread: enumeration and
            // preparing the haystacks are the expensive halves, and the answer
            // to a re-index is usually "nothing changed".
            let (rebuilt, quicklinks) = cx
                .background_executor()
                .spawn(async move {
                    // Read once, for both halves: the index holds the links
                    // as catalogue entries, and root search needs the links
                    // themselves for answers and fallbacks -- an alias can
                    // change without any catalogue entry changing.
                    let quicklinks =
                        lumen_core::providers::quicklinks::load(&paths.quicklinks_file());
                    let rebuilt =
                        lumen_core::root::build_index(platform.as_ref(), &paths, &quicklinks).map(
                            |rebuilt| {
                                let unchanged = lumen_core::root::same_index(&current, &rebuilt);
                                (!unchanged).then_some(rebuilt)
                            },
                        );
                    (rebuilt, QuicklinkProvider::new(quicklinks))
                })
                .await;

            let index = match rebuilt {
                Ok(index) => index,
                Err(error) => {
                    log::error!("could not index applications: {error:#}");
                    None
                }
            };

            this.update(cx, |this, cx| {
                let links_changed = this.root.replace_quicklinks(quicklinks);
                // Nothing to show means nothing to re-rank: that would put the
                // selection back on the first row under somebody who had just
                // moved it (see `same_index`).
                if index.is_none() && !links_changed {
                    return;
                }
                if let Some(index) = index {
                    this.root.replace_index(index);
                }
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
        self.clear_query_text(cx);
        self.selected = 0;
        self.appearances = self.appearances.wrapping_add(1);

        // A view does not survive the launcher being dismissed. Coming back to
        // somebody else's list a day later, with no memory of opening it, is
        // the kind of state that makes an interface feel haunted.
        self.mode = Mode::Root;
        self.argument = None;
        self.action_panel = None;

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
        self.query_field.read(cx).focus(window);

        // Registered on the first show rather than at construction, because it
        // needs a `Window` and the launcher is built before one exists. Once,
        // not per show: a subscription per invocation would dismiss the
        // launcher as many times as it had ever been opened.
        self.focused_once = false;
        if self._activation.is_none() {
            self._activation = Some(cx.observe_window_activation(
                window,
                |launcher, window, cx| {
                    launcher.on_activation_changed(window.is_window_active(), cx);
                },
            ));
        }

        self.rerank(cx);
        cx.notify();
    }

    /// The window gained or lost focus.
    ///
    /// Losing it means the user clicked something else, and a launcher that
    /// stays on top of whatever they clicked is a launcher they have to dismiss
    /// before they can use the thing they just chose. So it goes away -- which
    /// is also what every other launcher on every platform does, and therefore
    /// what people expect without being told.
    fn on_activation_changed(&mut self, active: bool, cx: &mut Context<Self>) {
        if active {
            self.focused_once = true;
            return;
        }

        // Not yet focused: this is the gap during the show sequence, not the
        // user clicking away.
        if !self.focused_once {
            return;
        }

        if !self.config.window.dismiss_on_focus_loss {
            return;
        }

        self.focused_once = false;
        cx.emit(LauncherEvent::Dismissed);
    }

    pub fn apply_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        let colors = theme.colors.clone();
        if let Mode::Form(state) = &self.mode {
            let colors = colors.clone();
            state
                .form
                .update(cx, |form, cx| form.set_colors(colors, cx));
        }
        self.query_field
            .update(cx, |field, cx| field.set_colors(colors, cx));
        self.theme = theme;
        cx.notify();
    }

    /// Take a configuration that was edited while the daemon runs.
    ///
    /// Row count and file-search folders are the launcher's to apply; where
    /// the window sits is the daemon's, which re-places it after this.
    pub fn apply_config(&mut self, config: Config, cx: &mut Context<Self>) {
        let previous_height = self.content_height();
        let files_changed = config.files != self.config.files;

        self.root.set_visible_rows(config.window.visible_rows);
        self.config = config;

        if files_changed {
            self.refresh_files(cx);
        }
        self.resize_if_needed(previous_height, cx);
        cx.notify();
    }

    pub fn appearance(&self) -> Appearance {
        self.theme.appearance
    }

    /// How many result rows the window should currently be sized for.
    pub fn visible_rows(&self) -> usize {
        self.hits().len().min(self.config.window.visible_rows)
    }

    /// How tall the window should be for whatever it is currently showing.
    ///
    /// Three kinds of content, three heights. The action panel replaces the
    /// list (7.11) and an action row is shorter than a result row; a view's
    /// rows are result-height but its section headers are not.
    pub fn content_height(&self) -> f32 {
        if self.action_panel.is_some() {
            return self.config.panel_height(self.panel_entries().len());
        }

        match &self.mode {
            Mode::Root => self.config.window_height(self.visible_rows()),
            Mode::Form(_) => {
                lumen_core::layout::SEARCH_HEIGHT
                    + lumen_core::layout::form::HEIGHT
                    + lumen_core::layout::FOOTER_HEIGHT
            }
            Mode::View(state) if state.loading => self.loading_height(),
            Mode::View(state) => {
                lumen_core::layout::SEARCH_HEIGHT
                    + lumen_core::layout::LIST_PADDING * 2.0
                    + self.view_rows_height(state)
                    + lumen_core::layout::FOOTER_HEIGHT
            }
        }
    }

    /// How much vertical space a view's rows take, rounded down to a whole row.
    ///
    /// Not `total.min(cap)`. A view's rows are not all the same height -- a
    /// section header is shorter than an item -- so capping the sum lands
    /// wherever it lands, which for the bundled example is two thirds of the
    /// way through the last row it shows. Accumulating whole rows until the
    /// next one would not fit is what keeps I-10's promise here: a partial row
    /// is never left clipped at the bottom edge.
    fn view_rows_height(&self, state: &ViewState) -> f32 {
        // A list with nothing in it still has something to say, and needs room
        // to say it. Without this the window is sized to zero rows and the
        // empty state spills over the footer.
        if state.rows.is_empty() && !state.loading {
            return lumen_core::layout::view::EMPTY_HEIGHT;
        }

        let cap = self.config.window.visible_rows as f32 * lumen_core::layout::ROW_HEIGHT;

        let mut used = 0.0;
        for row in &state.rows {
            let height = crate::view::row_height(row);
            if used + height > cap {
                break;
            }
            used += height;
        }

        // A single row taller than the whole cap is shown anyway: an empty
        // window is worse than one row that does not quite fit.
        if used == 0.0 {
            return state
                .rows
                .first()
                .map(crate::view::row_height)
                .unwrap_or_default();
        }

        used
    }

    /// Ask for a resize, but only when the height actually changed.
    ///
    /// Emitting on every keystroke would ask the platform to move the window to
    /// the rectangle it is already in, which is a `SetWindowPos` and a fresh
    /// swap-chain buffer for nothing.
    fn resize_if_needed(&mut self, previous: f32, cx: &mut Context<Self>) {
        let height = self.content_height();
        if height != previous {
            cx.emit(LauncherEvent::Resized(height));
        }
    }

    fn rerank(&mut self, cx: &mut Context<Self>) {
        // A form has no list to rank. Asides still land while it is open --
        // the clipboard is recorded whatever the launcher shows -- and the
        // root list is ranked when the form is left.
        if matches!(self.mode, Mode::Form(_)) {
            return;
        }

        let previous_height = self.content_height();

        // A view filters its own rows and has no use for the root index.
        if matches!(self.mode, Mode::View(_)) {
            self.refilter_view();
            self.action_panel = None;

            // Sent only when the extension asked to do its own filtering, and
            // only when the query has actually changed.
            //
            // The second condition is not belt and braces. Re-ranking happens
            // for reasons that have nothing to do with typing -- a background
            // index landing, for one -- and an extension told its query changed
            // re-renders. The first extension found this the hard way: copying
            // a character updated the clipboard history, which re-ranked, which
            // reported a query change that had not happened, which made the
            // extension redraw the list and wipe the "copied" tag it had just
            // put there. A spurious event is not a harmless one.
            let query = self.query.clone();
            if let Mode::View(state) = &mut self.mode
                && let lumen_core::view::View::List(list) = &state.view
                && list.filtering == lumen_core::view::Filtering::Extension
                && state.sent_query != query
            {
                state.sent_query = query.clone();
                self.tell_extension(lumen_core::view::Event::QueryChanged { query });
            }

            self.resize_if_needed(previous_height, cx);
            return;
        }

        // The ordering rules -- answers first, asides discounted and capped --
        // are `RootSearch`'s. Nothing in it waits on anything (I-3).
        match &self.argument {
            Some(argument) => {
                self.root.rank_argument(&argument.id, &self.query);
            }
            None => {
                self.root.rank(&self.query);
            }
        }

        // The selection always returns to the top on a new query. Trying to keep
        // it on "the same item" across a re-rank is the kind of cleverness that
        // makes a launcher feel unpredictable.
        self.selected = 0;

        // A panel describing the previous row must not survive into the next
        // one, or Enter runs an action the user can no longer see.
        self.action_panel = None;

        self.resize_if_needed(previous_height, cx);
    }

    // -- Extension views (F4) ----------------------------------------------

    /// Open a view in place of the root list.
    fn open_view(&mut self, source: &lumen_core::ViewSource, cx: &mut Context<Self>) {
        let previous_height = self.content_height();

        let view = match source {
            // `include_str!`, not a read: this is the file the plan's step 1
            // asks to render, it ships with the binary, and a demonstration
            // that can fail because a path moved demonstrates nothing.
            lumen_core::ViewSource::Example => {
                let source = include_str!("../../../assets/views/example.json");
                match lumen_core::view::from_json(source) {
                    Ok(view) => view,
                    Err(error) => {
                        log::error!("the example view is not valid: {error}");
                        return;
                    }
                }
            }
            // Starting a process is the daemon's business, not the list's, so
            // this leaves. The daemon calls `begin_view` straight back, which
            // is what puts the loading state on screen; the view itself arrives
            // whenever the extension gets round to sending one.
            lumen_core::ViewSource::Command { extension, command } => {
                cx.emit(LauncherEvent::RunExtension {
                    extension: extension.clone(),
                    command: command.clone(),
                });
                return;
            }
        };

        // Prepared once, here, rather than on each keystroke: filtering a view
        // is the same matcher the root list uses and it runs in the same frame.
        let candidates = match &view {
            lumen_core::view::View::List(list) => list.items().map(candidate_for).collect(),
            lumen_core::view::View::Detail(_) => Vec::new(),
        };

        let rows = match &view {
            lumen_core::view::View::List(list) => crate::view::rows(list),
            lumen_core::view::View::Detail(_) => Vec::new(),
        };

        self.clear_query_text(cx);
        self.action_panel = None;
        self.argument = None;
        self.mode = Mode::View(Box::new(ViewState {
            sink: None,
            loading: false,
            list: crate::view::list_state(rows.len()),
            selected: 0,
            rows,
            candidates: Arc::new(candidates),
            sent_query: String::new(),
            view,
        }));

        self.resize_if_needed(previous_height, cx);
        cx.notify();
    }

    /// Take over the list for an extension that is starting (F4, step 3).
    ///
    /// Called before the extension has said anything. The launcher shows that
    /// it is loading rather than freezing or staying on the root list: the user
    /// pressed Enter and something has to acknowledge it.
    /// How tall a view is while it is still starting.
    ///
    /// The same as an empty state's, so that a view which turns out to have
    /// nothing in it does not resize the window on arrival.
    pub fn loading_height(&self) -> f32 {
        lumen_core::layout::SEARCH_HEIGHT
            + lumen_core::layout::LIST_PADDING * 2.0
            + lumen_core::layout::view::EMPTY_HEIGHT
            + lumen_core::layout::FOOTER_HEIGHT
    }

    pub fn begin_view(&mut self, sink: ViewSink, cx: &mut Context<Self>) {
        let previous_height = self.content_height();

        self.clear_query_text(cx);
        self.action_panel = None;
        self.argument = None;
        self.mode = Mode::View(Box::new(ViewState {
            sink: Some(sink),
            loading: true,
            view: lumen_core::view::View::List(Default::default()),
            rows: Vec::new(),
            list: crate::view::list_state(0),
            selected: 0,
            candidates: Arc::new(Vec::new()),
            sent_query: String::new(),
        }));

        self.resize_if_needed(previous_height, cx);
        cx.notify();
    }

    /// Draw what the extension has sent.
    ///
    /// Replaces whatever was there. An extension re-renders as often as it
    /// likes and the last one wins, which is what makes the protocol
    /// declarative rather than a list of mutations to apply in order.
    pub fn show_view(&mut self, view: lumen_core::view::View, cx: &mut Context<Self>) {
        let previous_height = self.content_height();

        let Mode::View(state) = &mut self.mode else {
            // The user left before it arrived. Nothing to draw and nothing to
            // report: leaving is allowed.
            return;
        };

        state.candidates = Arc::new(match &view {
            lumen_core::view::View::List(list) => list.items().map(candidate_for).collect(),
            lumen_core::view::View::Detail(_) => Vec::new(),
        });
        state.view = view;
        state.loading = false;

        // Rebuilt through the filter rather than wholesale, so a re-render
        // arriving while the user is typing does not silently widen the list
        // back out underneath them.
        self.refilter_view();
        self.resize_if_needed(previous_height, cx);
        cx.notify();
    }

    /// The extension has gone. Return to the root list.
    pub fn end_view(&mut self, cx: &mut Context<Self>) {
        if matches!(self.mode, Mode::View(_)) {
            self.leave_view(cx);
        }
    }

    /// Tell the extension something, if there is one listening.
    fn tell_extension(&self, event: lumen_core::view::Event) {
        if let Mode::View(state) = &self.mode
            && let Some(sink) = &state.sink
        {
            sink.emit(event);
        }
    }

    /// Return to the root list.
    fn leave_view(&mut self, cx: &mut Context<Self>) {
        // Told before the mode changes, or there is nothing left to tell.
        self.tell_extension(lumen_core::view::Event::Dismissed);

        let previous_height = self.content_height();
        self.mode = Mode::Root;
        self.clear_query_text(cx);
        self.action_panel = None;
        self.rerank(cx);
        self.resize_if_needed(previous_height, cx);
        cx.notify();
    }

    /// Narrow the open view to the current query.
    ///
    /// Only under [`Filtering::Host`](lumen_core::view::Filtering::Host), which
    /// is the default and the one the frame budget can promise: the same
    /// matcher as the root list, in the same frame, waiting on nothing. Under
    /// `Extension` the view is left exactly as it was sent, because narrowing
    /// it here would fight whatever the extension is about to send back.
    ///
    /// The surviving items keep the order and the sections the view gave them
    /// rather than being re-sorted by score. The extension chose that order --
    /// and the sections are labelled, so reordering across them would put rows
    /// under headings that do not describe them.
    fn refilter_view(&mut self) {
        let query = self.query.trim().to_string();

        let Mode::View(state) = &mut self.mode else {
            return;
        };
        let lumen_core::view::View::List(list) = &state.view else {
            return;
        };

        let keep: Option<std::collections::HashSet<usize>> =
            if query.is_empty() || list.filtering == lumen_core::view::Filtering::Extension {
                None
            } else {
                let candidates = state.candidates.clone();
                Some(self.root.filter(&query, &candidates).into_iter().collect())
            };

        let Mode::View(state) = &mut self.mode else {
            return;
        };
        let lumen_core::view::View::List(list) = &state.view else {
            return;
        };

        let mut rows = Vec::new();
        let mut index = 0;
        for section in &list.sections {
            let first = rows.len();
            for _ in &section.items {
                if keep.as_ref().is_none_or(|keep| keep.contains(&index)) {
                    rows.push(crate::view::Row::Item(index));
                }
                index += 1;
            }
            // A heading with nothing under it is a heading about nothing.
            if rows.len() > first
                && let Some(title) = &section.title
            {
                rows.insert(first, crate::view::Row::Section(title.clone().into()));
            }
        }

        // The selection lands on the first surviving row. Keeping it on an item
        // the filter just removed would leave Enter pointing at something that
        // is no longer on screen.
        state.selected = rows
            .iter()
            .find_map(|row| match row {
                crate::view::Row::Item(index) => Some(*index),
                crate::view::Row::Section(_) => None,
            })
            .unwrap_or(0);

        state.list = crate::view::list_state(rows.len());
        state.rows = rows;
    }

    /// The items currently on screen in a view, in display order.
    fn view_items(&self) -> Vec<usize> {
        let Mode::View(state) = &self.mode else {
            return Vec::new();
        };
        state
            .rows
            .iter()
            .filter_map(|row| match row {
                crate::view::Row::Item(index) => Some(*index),
                crate::view::Row::Section(_) => None,
            })
            .collect()
    }

    /// Move the selection within a view by `delta` places.
    fn move_view_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let items = self.view_items();
        if items.is_empty() {
            return;
        }

        let Mode::View(state) = &mut self.mode else {
            return;
        };

        let current = items
            .iter()
            .position(|index| *index == state.selected)
            .unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(items.len() as isize) as usize;
        state.selected = items[next];

        // Told about the row, not the item: a header occupies a row too, and
        // scrolling to the item's index would stop short by one per section.
        if let Some(row) = state.rows.iter().position(
            |row| matches!(row, crate::view::Row::Item(index) if *index == state.selected),
        ) {
            state.list.scroll_to_reveal_item(row);
        }

        // Told about the move, so an extension that wants to follow the
        // selection -- a detail pane, a preview -- can.
        self.tell_selection();
        cx.notify();
    }

    /// Tell the extension which row is selected now.
    fn tell_selection(&self) {
        let Mode::View(state) = &self.mode else {
            return;
        };
        let lumen_core::view::View::List(list) = &state.view else {
            return;
        };
        let Some(item) = list.item(state.selected) else {
            return;
        };

        self.tell_extension(lumen_core::view::Event::SelectionChanged {
            item: item.id.clone(),
        });
    }

    fn hits(&self) -> &[Hit] {
        self.root.hits()
    }

    fn item(&self, hit: &Hit) -> &Item {
        self.root.item(hit)
    }

    fn selected_item(&self) -> Option<&Item> {
        self.hits().get(self.selected).map(|hit| self.item(hit))
    }

    // -- Commands ----------------------------------------------------------

    /// The actions available for the selected row.
    fn available_actions(&self) -> Vec<Action> {
        self.selected_item()
            .map(|item| item.actions.clone())
            .unwrap_or_default()
    }

    /// The action panel's rows, whichever mode is showing.
    ///
    /// One shape for both, because the panel is one piece of interface. A root
    /// row's actions are Lumen's own and name themselves with a verb; a view's
    /// are the extension's and name themselves with whatever it chose, plus the
    /// key combination it wants shown beside them.
    fn panel_entries(&self) -> Vec<PanelEntry> {
        match &self.mode {
            Mode::Root => self
                .available_actions()
                .into_iter()
                .map(|action| PanelEntry {
                    title: action.verb().to_string(),
                    shortcut: None,
                })
                .collect(),
            Mode::View(state) => state
                .view
                .actions(state.selected)
                .iter()
                .map(|action| PanelEntry {
                    title: action.title.clone(),
                    shortcut: action.shortcut.clone(),
                })
                .collect(),
            Mode::Form(_) => Vec::new(),
        }
    }

    fn toggle_action_panel(
        &mut self,
        _: &ToggleActionPanel,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous_height = self.content_height();

        self.action_panel = match self.action_panel {
            Some(_) => None,
            // Opens on the *second* action, not the first: the first is what
            // Enter already does, so landing on it would make the panel's own
            // Enter a no-op and waste the gesture.
            None if self.panel_entries().len() > 1 => Some(1),
            None if !self.panel_entries().is_empty() => Some(0),
            None => None,
        };

        self.resize_if_needed(previous_height, cx);
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        // While the panel is open the arrow keys move within it. Two lists on
        // screen and one set of arrow keys: whichever is in front wins.
        if let Some(selected) = self.action_panel {
            let count = self.panel_entries().len();
            if count > 0 {
                self.action_panel = Some((selected + 1) % count);
                cx.notify();
            }
            return;
        }

        if matches!(self.mode, Mode::View(_)) {
            self.move_view_selection(1, cx);
            return;
        }

        if self.hits().is_empty() {
            return;
        }
        // Wrapping, both ways. Pressing down at the bottom of a short list and
        // having nothing happen is the kind of dead end that makes a keyboard
        // interface feel unfinished.
        self.selected = (self.selected + 1) % self.hits().len();
        self.reveal_selection(cx);
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(selected) = self.action_panel {
            let count = self.panel_entries().len();
            if count > 0 {
                self.action_panel = Some(selected.checked_sub(1).unwrap_or(count - 1));
                cx.notify();
            }
            return;
        }

        if matches!(self.mode, Mode::View(_)) {
            self.move_view_selection(-1, cx);
            return;
        }

        if self.hits().is_empty() {
            return;
        }
        self.selected = self
            .selected
            .checked_sub(1)
            .unwrap_or(self.hits().len() - 1);
        self.reveal_selection(cx);
    }

    fn select_first(&mut self, _: &SelectFirst, _: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.mode, Mode::View(_)) {
            let first = self.view_items().first().copied();
            if let (Some(first), Mode::View(state)) = (first, &mut self.mode) {
                state.selected = first;
                state.list.scroll_to_reveal_item(0);
                cx.notify();
            }
            return;
        }

        self.selected = 0;
        self.reveal_selection(cx);
    }

    fn select_last(&mut self, _: &SelectLast, _: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.mode, Mode::View(_)) {
            let last = self.view_items().last().copied();
            if let (Some(last), Mode::View(state)) = (last, &mut self.mode) {
                state.selected = last;
                let rows = state.rows.len().saturating_sub(1);
                state.list.scroll_to_reveal_item(rows);
                cx.notify();
            }
            return;
        }

        self.selected = self.hits().len().saturating_sub(1);
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

    fn activate(&mut self, _: &Activate, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.mode, Mode::View(_)) {
            self.activate_in_view(cx);
            return;
        }

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

        // Asking for an argument has used nothing yet, so it is not recorded:
        // the link is recorded once, when it is actually opened.
        if let Action::AskArgument(request) = action {
            self.enter_argument(request, cx);
            return;
        }

        // Record and persist before launching. A launch that hands the
        // foreground to another application immediately is a launch whose
        // bookkeeping is easy to lose.
        // The write goes to a background thread: it is disk I/O on the path
        // between Enter and the launch, and nothing here needs to wait for it.
        let history = self.root.record_use(&id);
        let path = self.paths.frecency_file();
        cx.background_spawn(async move {
            if let Err(error) = history.save(&path) {
                log::warn!("could not persist usage statistics: {error:#}");
            }
        })
        .detach();

        // Arranging has to hide *and* keep the handle that hiding would
        // otherwise consume, so it replaces the dismissal rather than
        // following it. The daemon does both, in that order.
        if let Action::ArrangeWindow(layout) = action {
            cx.emit(LauncherEvent::ArrangeWindow(layout));
            return;
        }

        // A view replaces the list rather than doing something and going away,
        // so this neither dismisses nor reaches the platform.
        if let Action::OpenView(source) = action {
            self.open_view(&source, cx);
            return;
        }

        // So does the quicklink form.
        if action == Action::CreateQuicklink {
            self.open_form(window, cx);
            return;
        }

        cx.emit(LauncherEvent::Dismissed);

        // Lumen's own commands open windows or stop the process, which is the
        // shell's business and not the list's (I-4 lives out there). They go
        // back as an event; everything else the platform can do directly.
        if let Action::Builtin(command) = action {
            cx.emit(LauncherEvent::Builtin(command));
            return;
        }

        // A quicklink that names no browser opens in the preferred one, which
        // only the configuration knows.
        let action = match action {
            Action::OpenLink(open) => {
                Action::OpenLink(open.or_browser(self.config.preferred_browser.as_deref()))
            }
            action => action,
        };

        let platform = self.platform.clone();
        cx.background_spawn(async move {
            if let Err(error) = action.perform(platform.as_ref()) {
                log::error!("could not run {action:?}: {error:#}");
            }
        })
        .detach();
    }

    /// Tab: type a quicklink's argument.
    ///
    /// The selected row's, when it takes one -- looking through all of its
    /// actions, so the key means the same whichever one Enter runs. Otherwise
    /// the link whose alias or name the query is exactly: `yt` then Tab means
    /// YouTube even when usage has put something else first.
    fn ask_argument(&mut self, _: &AskArgument, _: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.mode, Mode::Root)
            || self.action_panel.is_some()
            || self.argument.is_some()
        {
            return;
        }
        let request = self
            .selected_item()
            .and_then(|item| {
                item.actions.iter().find_map(|action| match action {
                    Action::AskArgument(request) => Some(request.clone()),
                    _ => None,
                })
            })
            .or_else(|| self.root.named_link(&self.query));
        if let Some(request) = request {
            self.enter_argument(request, cx);
        }
    }

    /// Turn the search field into `request`'s argument.
    ///
    /// Starts empty: what was typed to *find* the link is not what the user
    /// wants to search for with it.
    fn enter_argument(
        &mut self,
        request: lumen_core::providers::quicklinks::ArgumentRequest,
        cx: &mut Context<Self>,
    ) {
        self.argument = Some(request);
        self.action_panel = None;
        self.clear_query_text(cx);
        self.rerank(cx);
        cx.notify();
    }

    /// Back to the root list from an argument.
    fn leave_argument(&mut self, cx: &mut Context<Self>) {
        self.argument = None;
        self.clear_query_text(cx);
        self.rerank(cx);
        cx.notify();
    }

    /// Show the form that saves a new quicklink, in place of the list.
    fn open_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let previous_height = self.content_height();

        let path = self.paths.quicklinks_file();
        let colors = self.theme.colors.clone();
        let form = cx.new(|cx| QuicklinkForm::new(path, None, colors, cx));
        let events = cx.subscribe_in(&form, window, |this, _, event, window, cx| {
            let saved = match event {
                FormEvent::Saved(name) => Some(name.clone()),
                FormEvent::Cancelled => None,
            };
            this.leave_form(saved, window, cx);
        });

        self.action_panel = None;
        self.argument = None;
        self.clear_query_text(cx);
        self.mode = Mode::Form(FormState {
            form: form.clone(),
            _events: events,
        });
        form.read(cx).focus(window, cx);

        // What the form can offer -- the installed browsers, and a link just
        // copied -- is a registry walk and a clipboard read, so it arrives a
        // moment later rather than holding the form back (I-3's discipline,
        // if not its letter).
        let platform = self.platform.clone();
        let form = form.downgrade();
        cx.spawn(async move |_, cx| {
            let (browsers, copied) = cx
                .background_executor()
                .spawn(async move {
                    let browsers = platform
                        .installed_browsers()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|browser| browser.name)
                        .collect::<Vec<_>>();
                    let copied = platform
                        .clipboard_text()
                        .ok()
                        .flatten()
                        .map(|text| text.trim().to_string())
                        .filter(|text| lumen_core::providers::quicklinks::looks_like_link(text));
                    (browsers, copied)
                })
                .await;

            form.update(cx, |form, cx| {
                form.set_browsers(browsers, cx);
                if let Some(link) = copied {
                    form.suggest_link(link, cx);
                }
            })
            .ok();
        })
        .detach();

        self.resize_if_needed(previous_height, cx);
        cx.notify();
    }

    /// Back to the root list from the form.
    ///
    /// After a save the query is the new link's name, so the list shows it as
    /// soon as the watcher has re-read the file: the confirmation is the link
    /// itself, not a message about it.
    fn leave_form(&mut self, saved: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.mode, Mode::Form(_)) {
            return;
        }
        let previous_height = self.content_height();

        self.mode = Mode::Root;
        match saved {
            Some(name) => {
                self.query_field
                    .update(cx, |field, cx| field.set_text(name.clone(), cx));
                self.query = name;
            }
            None => self.clear_query_text(cx),
        }
        self.query_field.read(cx).focus(window);
        self.rerank(cx);
        self.resize_if_needed(previous_height, cx);
        cx.notify();
    }

    /// Run the selected action inside a view.
    ///
    /// The launcher does not know what an extension's action means, so it
    /// does not carry one out: it tells the extension which action was
    /// invoked on which item, and the extension answers with a new view or a
    /// request of the host. The bundled example view has no process behind it,
    /// so there the event is only logged.
    fn activate_in_view(&mut self, cx: &mut Context<Self>) {
        let Mode::View(state) = &self.mode else {
            return;
        };

        let actions = state.view.actions(state.selected);
        let action = match self.action_panel {
            Some(index) => actions.get(index).or_else(|| actions.first()),
            None => actions.first(),
        };

        let Some(action) = action else {
            return;
        };

        let item = match &state.view {
            lumen_core::view::View::List(list) => {
                list.item(state.selected).map(|item| item.id.clone())
            }
            lumen_core::view::View::Detail(_) => None,
        };

        let event = lumen_core::view::Event::ActionInvoked {
            item,
            action: action.id.clone(),
        };

        match &state.sink {
            Some(_) => self.tell_extension(event),
            // The bundled example has nobody to tell. Logged rather than
            // silently dropped, because "did that do anything?" is the first
            // question anyone looking at step 1 asks.
            None => log::info!("the example view has nowhere to send {event:?}"),
        }

        self.action_panel = None;
        cx.notify();
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(LauncherEvent::Dismissed);
    }

    fn clear_query(&mut self, _: &ClearQuery, window: &mut Window, cx: &mut Context<Self>) {
        // The form handles Escape itself; this is for when focus has somehow
        // left its fields, and it must still mean "back", not "dismiss".
        if matches!(self.mode, Mode::Form(_)) {
            self.leave_form(None, window, cx);
            return;
        }

        // Escape unwinds one layer at a time: the panel, then the query, then
        // the launcher. One key, and it always means "back" -- never "back two
        // steps because the first one happened to be empty".
        if self.action_panel.take().is_some() {
            cx.notify();
            return;
        }

        // In a view the layers are one deeper: the panel, the view's query,
        // the view, and only then the launcher. Leaving the view on the same
        // key is what makes it feel like somewhere you went rather than
        // somewhere you are stuck.
        if matches!(self.mode, Mode::View(_)) {
            if self.query.is_empty() {
                self.leave_view(cx);
            } else {
                self.clear_query_text(cx);
                self.rerank(cx);
                cx.notify();
            }
            return;
        }

        // An argument is one more layer: its text, then the pill, then the
        // launcher.
        if self.argument.is_some() && self.query.is_empty() {
            self.leave_argument(cx);
            return;
        }

        // Escape on a non-empty query clears it; on an empty one it dismisses.
        if self.query.is_empty() {
            cx.emit(LauncherEvent::Dismissed);
            return;
        }
        self.clear_query_text(cx);
        self.rerank(cx);
        cx.notify();
    }

    /// Empty the query without treating it as an edit.
    ///
    /// The field does not report this back, so callers re-rank themselves when
    /// they need to -- most of them are changing mode at the same moment and
    /// re-rank once for both.
    fn clear_query_text(&mut self, cx: &mut Context<Self>) {
        self.query.clear();
        self.query_field
            .update(cx, |field, cx| field.set_text(String::new(), cx));
    }
}

/// What the launcher asks the window to do.
///
/// An event rather than a direct call because showing and hiding is the
/// platform's business (I-4), and the view must not reach across that boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum LauncherEvent {
    Dismissed,
    /// One of Lumen's own commands was run.
    Builtin(lumen_core::Builtin),
    /// An extension's command was chosen, and the daemon should start it.
    ///
    /// Carries the folder and the command rather than anything already
    /// running: at this point nothing is (AD-08), which is the whole reason
    /// the root list can offer it without waiting on anything (I-3).
    RunExtension {
        extension: std::path::PathBuf,
        command: String,
    },
    /// A layout was chosen for the window that held the foreground.
    ///
    /// Carries no window handle: the daemon is the only thing that has one,
    /// and it is the same handle it is about to restore focus to. This event
    /// also *replaces* `Dismissed` rather than following it, because hiding
    /// takes that handle and the arrangement still needs it.
    ArrangeWindow(lumen_platform::WindowLayout),
    /// The content changed size, so the window should be resized to hug it.
    ///
    /// A height rather than a row count, because the launcher has two kinds of
    /// content and they are not made of the same rows: a result row is tall
    /// enough for a title and a subtitle, an action row holds one line. When
    /// this carried a row count the action panel was drawn into a window still
    /// sized for eight results, and two actions sat above six rows of nothing.
    Resized(f32),
}

impl gpui::EventEmitter<LauncherEvent> for Launcher {}

impl Focusable for Launcher {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.query_field.focus_handle(cx)
    }
}

impl Render for Launcher {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            // No `track_focus` here. Focus lives in the query field, and a
            // focusable root would take it back on every click anywhere in
            // the window -- after which typing would go nowhere.
            .key_context("Launcher")
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_first))
            .on_action(cx.listener(Self::select_last))
            .on_action(cx.listener(Self::activate))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::clear_query))
            .on_action(cx.listener(Self::toggle_action_panel))
            .on_action(cx.listener(Self::ask_argument))
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

        // One region, three possible occupants, in order of precedence: the
        // action panel over whatever it was opened from, then an extension's
        // view, then the root list.
        let body = match (self.render_action_panel(cx), &self.mode) {
            (Some(panel), _) => panel,
            (None, Mode::Form(state)) => state.form.clone().into_any_element(),
            (None, Mode::View(state)) => self.render_view(state),
            (None, Mode::Root) => self.render_results(cx).into_any_element(),
        };

        root.child(self.render_search_field(cx))
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
    fn render_search_field(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &self.theme.colors;

        // A view says what its own search does. "Search for apps and
        // commands" over somebody's pull requests is the launcher talking
        // over an extension in its own window.
        // A form's fields take the input, so the search row only says what
        // the form is.
        if matches!(self.mode, Mode::Form(_)) {
            return div()
                .flex()
                .items_center()
                .h(metrics::search_height())
                .px(metrics::gutter())
                .border_b_1()
                .border_color(color(colors.border_subtle))
                .text_size(type_scale::query())
                .text_color(color(colors.text_primary))
                .child("Create Quicklink");
        }

        let placeholder = match &self.mode {
            Mode::View(state) => state.view.placeholder().unwrap_or(FIND),
            Mode::Form(_) => ROOT_PLACEHOLDER,
            Mode::Root if self.argument.is_some() => ARGUMENT_PLACEHOLDER,
            Mode::Root => ROOT_PLACEHOLDER,
        };
        self.query_field.update(cx, |field, _| {
            field.set_placeholder(placeholder.to_string())
        });

        div()
            .flex()
            .items_center()
            .h(metrics::search_height())
            .px(metrics::gutter())
            .border_b_1()
            .border_color(color(colors.border_subtle))
            .text_size(type_scale::query())
            // The link the field is filling in, where the text would start:
            // what you type reads as its continuation.
            .when_some(self.argument.as_ref(), |row, argument| {
                row.child(
                    div()
                        .flex_none()
                        .mr(metrics::argument_pill_gap())
                        .px(metrics::argument_pill_padding_x())
                        .py(metrics::argument_pill_padding_y())
                        .rounded(metrics::argument_pill_radius())
                        .bg(color(colors.selection_background))
                        .border_1()
                        .border_color(color(colors.selection_border))
                        .text_size(type_scale::title())
                        .text_color(color(colors.text_primary))
                        .child(argument.name.clone()),
                )
            })
            .child(self.query_field.clone())
    }

    /// The height of the region the list occupies.
    ///
    /// Shared with the action panel, which takes that region over, so the two
    /// cannot disagree and the window never has to resize to swap between them.
    fn results_height(&self) -> gpui::Pixels {
        metrics::row_height() * self.visible_rows() as f32 + metrics::list_padding() * 2.0
    }

    /// Draw the open view (F4, step 2).
    ///
    /// The launcher owns the search field and the footer; the renderer owns
    /// everything between them. That split is the one an extension will see:
    /// it supplies rows, and the chrome around them is never its to change.
    fn render_view(&self, state: &ViewState) -> gpui::AnyElement {
        let height = gpui::px(
            self.content_height()
                - lumen_core::layout::SEARCH_HEIGHT
                - lumen_core::layout::FOOTER_HEIGHT,
        );

        if state.loading {
            return crate::view::render_loading(&self.theme, height);
        }

        // The loading state and the empty state are given the same room, so
        // the window does not jump when the first render turns out to be an
        // empty list.
        let height = if state.rows.is_empty() {
            gpui::px(lumen_core::layout::view::EMPTY_HEIGHT)
        } else {
            height
        };

        match &state.view {
            lumen_core::view::View::List(list) => crate::view::render_list(
                list,
                &state.rows,
                &state.list,
                state.selected,
                &self.theme,
                height,
            ),
            lumen_core::view::View::Detail(detail) => {
                crate::view::render_detail(detail, &self.theme, height)
            }
        }
    }

    fn render_results(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.hits().is_empty() {
            return div().h(gpui::px(0.0)).into_any_element();
        }

        let count = self.hits().len();
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
        let hit = &self.hits()[index];
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
            // Full width, and this is load-bearing rather than tidy. Without it
            // the row is only as wide as its content, `flex_1` below has no
            // free space to claim, and the kind label ends up butted against
            // the subtitle -- so it sits at a different horizontal position on
            // every row, which is precisely the layout shift I-10 forbids.
            .w_full()
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
                // a late icon never moves the title (I-10). What fills it is a
                // three-way choice and the order matters: the real icon if it
                // has arrived, the bundled glyph for the row's kind if there is
                // no real icon to arrive, and nothing at all while extraction
                // is still in flight -- because swapping a glyph for an icon
                // one frame later is a flicker, and a row that is about to have
                // its own icon should simply wait for it.
                div()
                    .size(metrics::icon_size())
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .map(|slot| match icon {
                        Some(image) => slot.child(img(image).size_full()),
                        None if item.icon.is_none() => slot.child(
                            svg()
                                .path(crate::icons::glyph(item.kind))
                                .size(metrics::icon_size())
                                .text_color(color(colors.icon)),
                        ),
                        None => slot,
                    }),
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
                                // A file's subtitle is a path and can be longer
                                // than the window. Ending it in an ellipsis is
                                // the difference between a row that is too long
                                // and a row that is cut through the middle of a
                                // letter.
                                .truncate()
                                .child(subtitle.clone()),
                        )
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .pl(metrics::icon_gap())
                    .text_size(type_scale::footer())
                    .text_color(color(colors.text_muted))
                    .child(item.kind_label()),
            )
            .into_any_element()
    }

    fn render_footer(&self) -> impl IntoElement {
        let colors = &self.theme.colors;
        let entries = self.panel_entries();
        let verb = match &self.mode {
            Mode::Root => self
                .selected_item()
                .map(|item| item.primary().verb().to_string())
                .unwrap_or_else(|| "Open".to_string()),
            // The extension's own word for what Enter does. It is the first
            // thing a view gets to say in the launcher's own chrome, and
            // overriding it with "Open" would make every extension sound the
            // same.
            Mode::View(_) => entries
                .first()
                .map(|entry| entry.title.clone())
                .unwrap_or_else(|| "Open".to_string()),
            Mode::Form(_) => "Save".to_string(),
        };

        let count = match &self.mode {
            Mode::Root => self.hits().len(),
            Mode::View(_) => self.view_items().len(),
            Mode::Form(_) => 0,
        };
        let status = match (&self.mode, count) {
            (Mode::Form(_), _) => "Next field  Tab".to_string(),
            (_, 1) => "1 result".to_string(),
            (_, count) => format!("{count} results"),
        };

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
            .child(div().child(status))
            .child(
                div()
                    .flex()
                    .gap(metrics::icon_gap())
                    .child(format!("{verb}  ⏎"))
                    // Advertised only when there is a second action to reach.
                    // A hint for a panel that would open empty teaches the user
                    // that the hint lies.
                    .when(entries.len() > 1, |footer| footer.child("Actions  Ctrl+K"))
                    .child(match self.mode {
                        Mode::Root if self.argument.is_some() => "Back  Esc",
                        Mode::Root => "Dismiss  Esc",
                        Mode::View(_) | Mode::Form(_) => "Back  Esc",
                    }),
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
        let entries = self.panel_entries();
        if entries.is_empty() {
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
                .p(gpui::px(panel::PADDING))
                .flex()
                .flex_col()
                .bg(color(colors.surface_elevated))
                .children(entries.into_iter().enumerate().map(|(index, entry)| {
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
                        .child(entry.title)
                        // What the extension asked to have shown beside it, when
                        // it asked. Shown, never bound: see
                        // `lumen_core::view::Action::shortcut`.
                        .when_some(entry.shortcut, |row, shortcut| {
                            row.child(
                                div()
                                    .text_size(type_scale::footer())
                                    .text_color(color(colors.text_muted))
                                    .child(shortcut),
                            )
                        })
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

#[cfg(test)]
mod tests {

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

        // Enter reuses a running copy; a fresh one is a deliberate second
        // choice, and copying the name is the third.
        assert_eq!(item.primary().verb(), "Open");
        assert_eq!(
            item.actions[1],
            Action::LaunchNew(LaunchTarget::ShellExecute("x".into()))
        );
        assert_eq!(item.actions[2], Action::CopyToClipboard("Firefox".into()));
    }
}
