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
//!
//! The calculator's answer is the one exception, and a bounded one (7.40): it
//! is drawn as a card `card.rows` rows tall, above the list and in the
//! place of that many rows, so the window does not change height when one
//! appears.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::prelude::*;
use gpui::{
    Animation, AnimationExt as _, App, Context, Entity, FocusHandle, Focusable, HighlightStyle,
    MouseButton, ScrollHandle, ScrollStrategy, Styled, StyledText, Subscription, Task,
    UniformListScrollHandle, Window, actions, div, ease_out_quint, img, svg, uniform_list,
};
use lumen_core::favorites::Change as FavoriteChange;
use lumen_core::file_index::{FileIndex, Store, WalkRules, digest};
use lumen_core::item::ActionGroup;
use lumen_core::providers::QuicklinkProvider;
use lumen_core::root::{FILES_CAP, Hit};
use lumen_core::style::{
    CardDivider, CardHeader, CardLabels, CardOperators, Metrics, SectionCase, Variants,
};
use lumen_core::theme::Appearance;
use lumen_core::{
    Action, Candidate, Config, Frecency, Item, Paths, Providers, RootSearch, Theme, layout,
};
use lumen_platform::Platform;

use crate::icons::IconCache;
use crate::keymap::Shortcuts;
use crate::quicklink_form::{FormEvent, QuicklinkForm};
use crate::snippet_form::{SnippetForm, SnippetFormEvent};
use crate::styling::{Geometry, color, rule_above, rule_below, type_scale};
use crate::text_field::{TextField, TextFieldEvent};
use crate::timer_form::{TimerForm, TimerFormEvent};

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
        /// Run the selected row's second action: another way of doing what
        /// `Enter` does.
        RunSecondaryAction,
        /// Hide the launcher, restoring focus to where it came from.
        Dismiss,
        /// Clear the whole query.
        ClearQuery,
        /// Show or hide the action panel for the selected row.
        ToggleActionPanel,
        /// Type the selected quicklink's argument (F6).
        AskArgument,
        /// Open the picker on a view's first scope, or move it to the next
        /// (F7).
        NextScope,
        /// Open the picker on a view's last scope, or move it to the previous.
        PreviousScope,
        /// Add the selected row to the favourites, or take it out (F15).
        ToggleFavorite,
        /// Move the selected favourite one place up (F15).
        MoveFavoriteUp,
        /// Move the selected favourite one place down (F15).
        MoveFavoriteDown,
    ]
);

/// The key context of the search row while the open view has a scope.
///
/// Deeper than `Launcher`, so its `Tab` wins there and only there: in the root
/// list `Tab` still types a quicklink's argument. GPUI ranks a binding by how
/// deep its context matched before anything else.
pub const SCOPE_CONTEXT: &str = "Scoped";

/// The wall clock as the timers write it.
fn timers_now() -> lumen_core::timers::Millis {
    lumen_core::timers::millis(std::time::SystemTime::now())
}

/// What the search field says when it is empty and nothing else applies.
const ROOT_PLACEHOLDER: &str = "Search for apps and commands...";

/// The fallback for a view that did not name its own.
const FIND: &str = "Search...";

/// What the search field says while it holds a quicklink's argument.
const ARGUMENT_PLACEHOLDER: &str = "Type what to search for...";

/// What the action panel's own field says while it is empty.
const ACTIONS_PLACEHOLDER: &str = "Search for actions\u{2026}";

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

/// A scope's option, prepared for the same matcher (F9).
fn option_candidate(option: &lumen_core::view::ScopeOption) -> Candidate {
    Candidate::new(Item {
        id: option.id.clone(),
        title: option.title.clone(),
        subtitle: None,
        icon: None,
        keywords: option.keywords.clone(),
        actions: Vec::new(),
        kind: lumen_core::Kind::Command,
    })
}

/// One row of the action panel, whichever mode produced it.
struct PanelEntry {
    title: String,
    /// The keys shown beside it, one label per keycap, when it has any.
    ///
    /// For a root row they are read from the keymap -- see
    /// [`crate::keymap::Shortcuts`] -- and for a view's they are what the
    /// extension asked to have shown, which is displayed, never bound: see
    /// `lumen_core::view::Action::shortcut`.
    shortcut: Vec<String>,
    /// The icon drawn at its start. The square is reserved whether or not
    /// there is one (I-10).
    glyph: Option<lumen_core::view::Icon>,
    /// Which run of rows it belongs to; the panel draws a rule where this
    /// changes. A view's actions and the browsers are one run each.
    group: ActionGroup,
}

/// Which row the action panel opens on: the first, as Raycast's does, which is
/// what Enter would run. `None` when there is nothing to show.
fn opening_selection(entries: usize) -> Option<usize> {
    (entries > 0).then_some(0)
}

/// The root list's panel rows for `actions`, with the keys the keymap gives
/// them.
///
/// A favourite shows the chord that names it, wherever it falls; otherwise the
/// first row shows what `Enter` is bound to and the second what Ctrl+Enter
/// is. Free of the launcher so it can be checked without standing one up.
fn root_panel_entries(actions: Vec<Action>, shortcuts: &Shortcuts) -> Vec<PanelEntry> {
    actions
        .into_iter()
        .enumerate()
        .map(|(index, action)| PanelEntry {
            title: action.verb().to_string(),
            shortcut: match (&action, index) {
                (Action::Favorite(FavoriteChange::Add | FavoriteChange::Remove), _) => {
                    shortcuts.toggle_favorite.clone()
                }
                (Action::Favorite(FavoriteChange::MoveUp), _) => shortcuts.move_up.clone(),
                (Action::Favorite(FavoriteChange::MoveDown), _) => shortcuts.move_down.clone(),
                (_, 0) => shortcuts.primary.clone(),
                (_, 1) => shortcuts.secondary.clone(),
                _ => Vec::new(),
            },
            glyph: Some(lumen_core::view::Icon::Glyph {
                name: action.glyph().into(),
            }),
            group: action.group(),
        })
        .collect()
}

/// Which of the panel's rows `query` keeps, by their place in the panel and
/// in its order; `None` when nothing is typed, which keeps every row.
///
/// `rank` is the root list's matcher, so `cop` finds *Copy* as it would an
/// app. It ranks best first; the panel keeps its own order instead, because
/// its groups are what the rules between rows are drawn from.
fn panel_matches<'a>(
    titles: impl Iterator<Item = &'a str>,
    query: &str,
    rank: impl FnOnce(&str, &[Candidate]) -> Vec<usize>,
) -> Option<Vec<usize>> {
    let query = query.trim();
    if query.is_empty() {
        return None;
    }
    let candidates: Vec<Candidate> = titles
        .map(|title| {
            Candidate::new(Item {
                id: title.to_string(),
                title: title.to_string(),
                subtitle: None,
                icon: None,
                keywords: Vec::new(),
                actions: Vec::new(),
                kind: lumen_core::Kind::Command,
            })
        })
        .collect();
    let mut matches = rank(query, &candidates);
    matches.sort_unstable();
    Some(matches)
}

/// The panel's rows as drawn: places in its entries, in order. Every row
/// with no search; a match that no longer exists is dropped.
fn visible_panel_rows(count: usize, matches: Option<&[usize]>) -> Vec<usize> {
    match matches {
        Some(matches) => matches.iter().copied().filter(|row| *row < count).collect(),
        None => (0..count).collect(),
    }
}

/// Where the highlight goes when the panel's search changes: to the first
/// row that is left, whichever it was on. A closed panel stays closed.
fn refiltered_selection(open: Option<usize>) -> Option<usize> {
    open.map(|_| 0)
}

/// What Ctrl+Enter runs: the second action, when there is one.
fn second_action(actions: Vec<Action>) -> Option<Action> {
    actions.into_iter().nth(1)
}

/// The file index the store at `path` holds, when it was walked with
/// `rules`: what a start installs at once, in milliseconds, instead of
/// waiting for a walk.
fn load_files(
    path: &std::path::Path,
    rules: &WalkRules,
    frecency: &Frecency,
) -> Option<Arc<FileIndex>> {
    let started = std::time::Instant::now();
    let index = match Store::open(path).and_then(FileIndex::load) {
        Ok(index) => index,
        Err(error) => {
            log::warn!("could not open the file index: {error:#}");
            return None;
        }
    };
    // Empty when no walk finished, which matches no rules.
    if index.digest() != digest(rules) {
        return None;
    }
    index.set_boosts(frecency);
    log::info!(
        "loaded the file index in {} ms: {}",
        started.elapsed().as_millis(),
        describe_files(&index)
    );
    Some(Arc::new(index))
}

/// Walk `rules` and make what was found the file index, written through
/// `current`'s store when one is installed (see `FileIndex::rebuild`).
///
/// `None` when the build `build` was superseded -- the newer one waits for
/// this one, writes its own walk, and this one must not write over it -- or
/// when the walk could not be written.
fn build_files(
    path: &std::path::Path,
    rules: &WalkRules,
    current: Option<Arc<FileIndex>>,
    frecency: &Frecency,
    builds: &AtomicU64,
    build: u64,
) -> Option<Arc<FileIndex>> {
    let started = std::time::Instant::now();
    let superseded = || builds.load(Ordering::SeqCst) != build;
    match FileIndex::rebuild(path, rules, current.as_deref(), &superseded) {
        Ok(None) => None,
        Ok(Some(index)) => {
            index.set_boosts(frecency);
            log::info!(
                "built the file index in {} ms: {}",
                started.elapsed().as_millis(),
                describe_files(&index)
            );
            Some(Arc::new(index))
        }
        Err(error) => {
            log::error!("could not build the file index: {error:#}");
            None
        }
    }
}

/// What the log says of an index: the figures slice 1 is measured by.
fn describe_files(index: &FileIndex) -> String {
    let truncated = index
        .truncated()
        .map(|scope| {
            format!(
                "; full at {} entries, the rest of {} is not searched",
                lumen_core::file_index::CEILING,
                scope.display()
            )
        })
        .unwrap_or_default();
    format!(
        "{} entries, {:?} by scope, {} bytes of names in memory{truncated}",
        index.len(),
        index.per_scope(),
        index.memory_bytes()
    )
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
    /// Saving a quicklink (F6) or a timer preset (F8). Like a view, it takes
    /// the list's place.
    Form(FormState),
}

/// A scope's options, shown in place of the list (F7).
///
/// Not the action panel: that sizes itself to every row it has, and a scope
/// can have thirty. This is capped at `layout::view::PICKER_ROWS` and scrolls.
///
/// Searchable (F9): while it is open the search field filters its options,
/// because arrowing through 166 currencies is not choosing one. The view's
/// query is set aside while it is open and given back when it closes.
struct Picker {
    /// Which of the view's scopes.
    scope: usize,
    /// Indices into the scope's options that match what is typed, best
    /// first; every option, in order, when nothing is.
    matches: Vec<usize>,
    /// An index into `matches`.
    highlighted: usize,
    scroll: UniformListScrollHandle,
    /// The view's query, to put back in the field when the picker closes.
    query: String,
    /// The search `matches` was computed for.
    filtered: String,
}

enum PickerMove {
    By(isize),
    First,
    Last,
}

/// A form in the list's place, and the subscriptions that hear it.
struct FormState {
    form: FormKind,
    /// What the search row says instead of taking input.
    title: &'static str,
    /// The form's own height, which the window hugs. A timer preset's grows
    /// when *Cycle* is turned on.
    height: f32,
    _events: Subscription,
    /// Follows the form's height, for a form whose height changes.
    _resize: Option<Subscription>,
}

enum FormKind {
    Quicklink(Entity<QuicklinkForm>),
    Timer(Entity<TimerForm>),
    Snippet(Entity<SnippetForm>),
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
    /// The action panel's own search, under its rows. Focused while the
    /// panel is open, so typing narrows the actions and leaves the query --
    /// and the list ranked from it -- as they were.
    action_field: Entity<TextField>,
    /// Kept alive so the panel's edits keep arriving; never read.
    _action_changes: Subscription,
    platform: Arc<dyn Platform>,
    paths: Paths,
    config: Config,
    theme: Theme,
    icons: IconCache,
    /// Pictures the host wrote, for *Clipboard History*'s images (F12).
    /// Emptied whenever the launcher hides.
    images: crate::images::ImageCache,

    /// What the root list is ranked from, and how (AD-03: the policy is in
    /// `core`, and this view only draws its hits).
    root: RootSearch,
    /// A pass fetching quicklink icons is running; one at a time.
    fetching_icons: bool,
    /// The daemon was asked for exchange rates since the window was shown
    /// (F9): once per showing, so a failing fetch is not asked again on
    /// every key.
    rates_asked: bool,
    /// Whether the places have been asked to be read for a question about
    /// time (F13). Once: read, they are kept.
    places_asked: bool,
    /// The city a forecast was last asked for from the root list (F13), so
    /// a query is not asked again on every keystroke.
    weather_asked: Option<u32>,
    /// The clipboard history, once the daemon has opened it.
    ///
    /// Shared with the recorder rather than opened twice: one database, one
    /// writer, and a read that cannot see a half-written row.
    clipboard: Option<Arc<parking_lot::Mutex<lumen_core::clipboard::History>>>,
    /// The file index (F16), once loaded or built. Shared with the scans in
    /// flight, so a rebuild replaces it without waiting for them.
    files: Option<Arc<FileIndex>>,
    /// Bumped by every build of the file index started, so an older one
    /// that finishes later never replaces a newer one.
    files_build: Arc<AtomicU64>,
    /// Bumped on every keystroke, so a scan of the file index for an
    /// earlier one stops, and its answer is dropped.
    files_query: Arc<AtomicU64>,

    /// A copy of the field's text, updated on every edit. Ranking reads it on
    /// every keystroke, and reaching into another entity for it each time
    /// would need an `App` everywhere a query is read.
    query: String,
    selected: usize,
    /// The root list as drawn: section headers and hits, in order (F15).
    /// The card, when there is one, is drawn above it and is not a row.
    rows: Vec<crate::view::Row>,
    /// `list` rather than `uniform_list`: a header is shorter than a row
    /// (7.29). Every height is still known before anything is drawn, so the
    /// window is sized and the selection revealed by arithmetic.
    list: gpui::ListState,

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
    /// The popover's rows scroll past the style's `actions.max-rows`; this is
    /// what keeps the highlighted one in view.
    panel_scroll: ScrollHandle,
    /// The panel is showing browsers for "Open With..." rather than the row's
    /// actions: the link to open, waiting for one to be chosen. Only
    /// meaningful while the panel is open.
    browser_menu: Option<lumen_core::providers::quicklinks::OpenLink>,
    /// The rows the panel's search keeps, by their place among its entries;
    /// `None` when nothing is typed there. `action_panel` indexes the rows
    /// shown, not the entries.
    panel_search: Option<Vec<usize>>,
    /// The installed browsers' names, read with the index so that "Open
    /// With..." shows them without a registry walk while the user waits.
    browsers: Vec<String>,
    /// A view's scope picker, when it is open. Never at the same time as the
    /// action panel.
    picker: Option<Picker>,

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
    /// What focus blocks, while a focus timer runs (F8); `None` otherwise.
    /// The daemon keeps it current, so the check on `Enter` waits on nothing.
    focus: Option<lumen_core::timers::focus::Guard>,
    /// The timers and stopwatches, for the footer's line (F8); `None` when
    /// there are none. Pushed by the daemon when they change.
    timers: Option<lumen_core::timers::Clock>,
    /// Whether the window is active, so the footer's clock ticks only while
    /// someone can see it.
    active: bool,
    /// Bumped by every tick asked for; a superseded one does nothing.
    tick: u64,
    /// The keys the action panel shows beside its rows, read from the keymap
    /// when the launcher was made (I-14).
    shortcuts: Shortcuts,
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
        let icons = IconCache::new(platform.clone(), theme.style.metrics.icon_resolution());

        let query_field =
            cx.new(|cx| TextField::search(ROOT_PLACEHOLDER, theme.colors.clone(), cx));
        let query_changes = cx.subscribe(&query_field, |launcher, _, event, cx| match event {
            TextFieldEvent::Changed(text) => {
                launcher.query.clone_from(text);
                launcher.rerank(cx);
                launcher.query_files(cx);
                cx.notify();
            }
            // Backspace in an empty argument takes the pill away, as it would
            // the character to its left.
            TextFieldEvent::DeletedPastStart => {
                if launcher.argument.is_some() {
                    launcher.leave_argument(cx);
                } else if launcher.back_out_of_path(cx) {
                    // A view's pill, one level at a time (F13).
                    cx.notify();
                }
            }
        });

        let action_field =
            cx.new(|cx| TextField::search(ACTIONS_PLACEHOLDER, theme.colors.clone(), cx));
        // Only the panel hears this field: what is typed here is never a
        // query, so it never re-ranks the list behind the popover.
        let action_changes = cx.subscribe(&action_field, |launcher, _, event, cx| {
            if let TextFieldEvent::Changed(text) = event {
                launcher.refilter_panel(text, cx);
            }
        });

        let mut root = RootSearch::new(Providers::builtin(), frecency);
        root.set_favorites(lumen_core::favorites::Favorites::load(
            &paths.favorites_file(),
        ));
        root.set_visible_rows(config.visible_rows(&theme.style.metrics));
        root.set_arrangement(config.root.clone());
        root.set_weather_units(config.weather.units);
        // `time in vancouver` is answered in the system's way of writing a
        // time, as the World Clock view is (F13).
        let writer = platform.clone();
        root.set_time_format(Arc::new(move |when| {
            writer
                .format_date(when, lumen_platform::DateStyle::Time, None)
                .unwrap_or_else(|_| format!("{:02}:{:02}", when.hour, when.minute))
        }));

        let mut launcher = Self {
            query_field,
            _query_changes: query_changes,
            action_field,
            _action_changes: action_changes,
            platform,
            paths,
            config,
            theme,
            icons,
            images: crate::images::ImageCache::new(),
            root,
            fetching_icons: false,
            rates_asked: false,
            places_asked: false,
            weather_asked: None,
            clipboard: None,
            files: None,
            files_build: Arc::new(AtomicU64::new(0)),
            files_query: Arc::new(AtomicU64::new(0)),
            query: String::new(),
            selected: 0,
            rows: Vec::new(),
            list: crate::view::list_state(0),
            mode: Mode::Root,
            argument: None,
            focused_once: false,
            _activation: None,
            action_panel: None,
            panel_scroll: ScrollHandle::new(),
            browser_menu: None,
            panel_search: None,
            browsers: Vec::new(),
            picker: None,
            appearances: 0,
            _theme_reload: None,
            focus: None,
            timers: None,
            active: false,
            tick: 0,
            shortcuts: Shortcuts::read(&cx.key_bindings().borrow()),
        };

        launcher.start_indexing(cx);
        launcher.refresh_files(cx);
        launcher
    }

    /// Forget every picture drawn, and free its texture (F12).
    ///
    /// Called by the daemon when the launcher hides: a screenshot's pixels
    /// are worth their memory only while somebody is looking at them.
    pub fn release_images(&mut self, cx: &mut Context<Self>) {
        self.images.release(cx);
    }

    /// Give the launcher the clipboard history, for the view and for
    /// `{clipboard offset=1}` in a snippet. Entries are no longer matched in
    /// the root list (F12).
    pub fn use_clipboard_history(
        &mut self,
        history: Arc<parking_lot::Mutex<lumen_core::clipboard::History>>,
    ) {
        self.clipboard = Some(history);
    }

    /// The clipboard history, for *Clipboard History* (F12) and
    /// `{clipboard offset=1}` in a snippet (F11). `None` when it is off or
    /// could not be opened.
    pub fn clipboard_history(
        &self,
    ) -> Option<Arc<parking_lot::Mutex<lumen_core::clipboard::History>>> {
        self.clipboard.clone()
    }

    /// Open the file index, and walk the scopes again behind it (F16).
    ///
    /// Everything on the background executor, and nothing waits for it
    /// (I-3). When nothing is installed yet, the index the store last
    /// recorded is loaded and installed at once -- milliseconds -- if it was
    /// walked with today's rules. Then the scopes are walked again and the
    /// result swapped in when it is done: until slice 2 watches for changes,
    /// that walk is what keeps the index current. Separate from
    /// `start_indexing` and started alongside it, because the walk is much
    /// the slower of the two and the applications must not wait for it.
    pub fn refresh_files(&mut self, cx: &mut Context<Self>) {
        self.open_files(true, cx);
    }

    /// [`Self::refresh_files`], walking again only when `rewalk` says so or
    /// the index at hand was walked with other rules.
    ///
    /// A configuration change asks with `rewalk` false: `in-root` and
    /// `enabled` are not how the index is walked, and neither is `~` written
    /// as the path it stands for, so what decides is the rules' digest, not
    /// the configuration. Turning file search off drops the index and root's
    /// questions to it; turning it on again loads it back from the store,
    /// in milliseconds, without a walk.
    fn open_files(&mut self, rewalk: bool, cx: &mut Context<Self>) {
        let build = self.files_build.fetch_add(1, Ordering::SeqCst) + 1;
        // What root showed came from the old rules, or the old level.
        if self.root.clear_files() && matches!(self.mode, Mode::Root) {
            self.rerank(cx);
            cx.notify();
        }
        if !self.config.files.enabled {
            self.files = None;
            self.files_query.fetch_add(1, Ordering::SeqCst);
            return;
        }
        self.query_files(cx);

        let config = self.config.files.clone();
        let path = self.paths.file_index_file();
        let frecency = self.root.frecency().clone();
        let installed = self.files.as_ref().map(|index| index.digest().to_string());
        let builds = self.files_build.clone();
        cx.spawn(async move |this, cx| {
            let (rules, loaded, current) = {
                let path = path.clone();
                let frecency = frecency.clone();
                cx.background_executor()
                    .spawn(async move {
                        // Resolving reads the disk too, so it is done here.
                        let rules = config.walk_rules();
                        let current = installed.as_deref() == Some(digest(&rules).as_str());
                        let loaded = installed
                            .is_none()
                            .then(|| load_files(&path, &rules, &frecency))
                            .flatten();
                        (rules, loaded, current)
                    })
                    .await
            };
            // Loaded only when walked with these rules, so it is current too.
            let current = current || loaded.is_some();
            if let Some(index) = loaded {
                this.update(cx, |this, cx| {
                    if this.files_build.load(Ordering::SeqCst) == build {
                        this.install_files(index, cx);
                    }
                })
                .ok();
            }
            if current && !rewalk {
                return;
            }

            let Ok(current) = this.update(cx, |this, _| this.files.clone()) else {
                return;
            };
            let built = cx
                .background_executor()
                .spawn(
                    async move { build_files(&path, &rules, current, &frecency, &builds, build) },
                )
                .await;
            if let Some(index) = built {
                this.update(cx, |this, cx| {
                    if this.files_build.load(Ordering::SeqCst) == build {
                        this.install_files(index, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    /// Make `index` the file index, and ask it what is typed.
    fn install_files(&mut self, index: Arc<FileIndex>, cx: &mut Context<Self>) {
        self.files = Some(index);
        self.query_files(cx);
    }

    /// Ask the file index what is typed, off the UI thread, and place what
    /// it answers when it does (F16).
    ///
    /// Every call supersedes the one before: its scan stops, and its answer
    /// is dropped. Root search has already drawn everything else; the files
    /// arrive into a section of their own at the end, under the selection
    /// rather than in its place (I-10).
    fn query_files(&mut self, cx: &mut Context<Self>) {
        let generation = self.files_query.fetch_add(1, Ordering::SeqCst) + 1;
        if !matches!(self.mode, Mode::Root) || self.argument.is_some() {
            return;
        }
        let (Some(index), Some(kinds)) = (self.files.clone(), self.config.files.root_kinds())
        else {
            return;
        };
        let query = self.query.trim().to_string();
        if query.is_empty() {
            return;
        }

        let current = self.files_query.clone();
        cx.spawn(async move |this, cx| {
            let asked = query.clone();
            let hits = cx
                .background_executor()
                .spawn(async move {
                    let cancelled = || current.load(Ordering::Relaxed) != generation;
                    index.root_query(&asked, kinds, FILES_CAP, &cancelled)
                })
                .await;
            this.update(cx, |this, cx| {
                let current = this.files_query.load(Ordering::SeqCst) == generation
                    && this.query.trim() == query
                    && matches!(this.mode, Mode::Root)
                    && this.argument.is_none();
                if current {
                    this.root.set_files(&query, hits);
                    this.place_files(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Rank again for files that arrived, keeping the selection where it is.
    ///
    /// Not `rerank`, which puts the selection back on the first row: that is
    /// right for a keystroke, and wrong for rows that arrived under somebody
    /// already moving through the list. The selected row stays selected
    /// wherever it now is; when it is gone, the selection stays at its place,
    /// within the list.
    fn place_files(&mut self, cx: &mut Context<Self>) {
        let previous_height = self.content_height();
        let selected = self.selected_item().map(|item| item.id.clone());
        self.root.rank(&self.query);
        let kept = selected
            .as_ref()
            .and_then(|id| self.hits().iter().position(|hit| self.item(hit).id == *id));
        match kept {
            Some(at) => self.selected = at,
            None => {
                self.selected = self.selected.min(self.hits().len().saturating_sub(1));
                // The panel described a row that is gone.
                self.action_panel = None;
            }
        }
        self.rebuild_rows();
        self.reveal_row();
        self.resize_if_needed(previous_height, cx);
        cx.notify();
    }

    /// Rebuild the index: applications, scripts, snippets and clipboard
    /// history -- and walk the file index's scopes again.
    ///
    /// Public so the daemon can ask for it after the user edits a script or a
    /// snippet, rather than making them restart to see the change. Until
    /// slice 2 watches for changes, this reload is also the way to bring the
    /// file index up to date by hand.
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
        let presets = self.config.translate.presets();
        let timers = self.config.timers.presets();
        let timer_presets = timers.clone();

        cx.spawn(async move |this, cx| {
            // Built and compared on the background thread: enumeration and
            // preparing the haystacks are the expensive halves, and the answer
            // to a re-index is usually "nothing changed".
            let (rebuilt, quicklinks, sites, browsers, rates, weather) = cx
                .background_executor()
                .spawn(async move {
                    // The table the currency view last kept (F9), so a
                    // conversion typed at the root is answered at once.
                    let rates = lumen_core::currency::State::load(&paths.currency_file()).rates;
                    // The forecasts last kept (F13), for `weather in ...`.
                    let weather = lumen_core::weather::Cache::load(&paths.weather_file());
                    // Read once, for both halves: the index holds the links
                    // as catalogue entries, and root search needs the links
                    // themselves for answers and fallbacks -- an alias can
                    // change without any catalogue entry changing.
                    let mut quicklinks =
                        lumen_core::providers::quicklinks::load(&paths.quicklinks_file());
                    lumen_core::providers::quicklinks::attach_icons(
                        &mut quicklinks,
                        &paths.favicons_dir(),
                    );
                    let sites: Vec<String> = quicklinks
                        .iter()
                        .filter_map(|link| link.favicon_site())
                        .collect();
                    let browsers: Vec<String> = platform
                        .installed_browsers()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|browser| browser.name)
                        .collect();
                    let rebuilt = lumen_core::root::build_index(
                        platform.as_ref(),
                        &paths,
                        &quicklinks,
                        presets,
                        timers,
                    )
                    .map(|rebuilt| {
                        let unchanged = lumen_core::root::same_index(&current, &rebuilt);
                        (!unchanged).then_some(rebuilt)
                    });
                    (
                        rebuilt,
                        QuicklinkProvider::new(quicklinks),
                        sites,
                        browsers,
                        rates,
                        weather,
                    )
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
                this.browsers = browsers;
                this.fetch_icons(sites, cx);
                let links_changed = this.root.replace_quicklinks(quicklinks);
                let presets_changed = this.root.replace_timer_presets(timer_presets);
                let rates_changed = this.root.replace_rates(rates);
                let weather_changed = this.root.replace_weather(weather);
                // Nothing to show means nothing to re-rank: that would put the
                // selection back on the first row under somebody who had just
                // moved it (see `same_index`).
                if index.is_none()
                    && !links_changed
                    && !presets_changed
                    && !rates_changed
                    && !weather_changed
                {
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

    /// Fetch the icons `sites` are missing, then reindex if any arrived.
    ///
    /// After the index, never before it: the list is usable with glyphs, and
    /// an icon is something to improve it with, not wait for (I-3). Most
    /// passes find every icon cached and touch the network not at all.
    fn fetch_icons(&mut self, sites: Vec<String>, cx: &mut Context<Self>) {
        if self.fetching_icons || sites.is_empty() {
            return;
        }
        self.fetching_icons = true;
        let dir = self.paths.favicons_dir();

        cx.spawn(async move |this, cx| {
            let written =
                cx.background_executor()
                    .spawn(async move {
                        lumen_core::providers::quicklinks::favicon::refresh(&dir, &sites)
                    })
                    .await;
            this.update(cx, |this, cx| {
                this.fetching_icons = false;
                // The next pass finds these cached and fetches nothing, so
                // this cannot go round more than once.
                if written > 0 {
                    this.start_indexing(cx);
                }
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
        self.rates_asked = false;
        self.weather_asked = None;

        // A view does not survive the launcher being dismissed. Coming back to
        // somebody else's list a day later, with no memory of opening it, is
        // the kind of state that makes an interface feel haunted.
        self.picker = None;
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
        self.active = active;
        if active {
            self.focused_once = true;
            self.schedule_tick(cx);
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
            match &state.form {
                FormKind::Quicklink(form) => {
                    form.update(cx, |form, cx| form.set_colors(colors, cx))
                }
                FormKind::Timer(form) => form.update(cx, |form, cx| form.set_colors(colors, cx)),
                FormKind::Snippet(form) => form.update(cx, |form, cx| form.set_colors(colors, cx)),
            }
        }
        self.action_field
            .update(cx, |field, cx| field.set_colors(colors.clone(), cx));
        self.query_field
            .update(cx, |field, cx| field.set_colors(colors, cx));
        let restyled = theme.style != self.theme.style;
        // Icons are extracted at a resolution that follows their square, so
        // a style that changes the square needs them extracted again, or a
        // larger icon is a small one stretched (F1, F15). Once per change of
        // style, which is something a person does, not something typing does.
        self.root
            .set_visible_rows(self.config.visible_rows(&theme.style.metrics));
        let resolution = theme.style.metrics.icon_resolution();
        if resolution != self.theme.style.metrics.icon_resolution() {
            self.icons = IconCache::new(self.platform.clone(), resolution);
        }
        self.theme = theme;
        // The headers' case is the style's, and is baked into the rows.
        if restyled {
            self.rebuild_rows();
        }
        // A style is geometry: the window's height and its anchor may both
        // have moved, so it is re-placed even when the height alone has not
        // -- an empty list is as tall in any style with the same search field
        // and footer, and its full height still differs (F15).
        if restyled {
            cx.emit(LauncherEvent::Resized {
                height: self.content_height(),
                anchor: self.anchor_height(),
                width: self.width(),
            });
        }
        cx.notify();
    }

    /// Take a configuration that was edited while the daemon runs.
    ///
    /// Row count and file-search folders are the launcher's to apply; where
    /// the window sits is the daemon's, which re-places it after this.
    pub fn apply_config(&mut self, config: Config, cx: &mut Context<Self>) {
        let previous_height = self.content_height();
        let files_changed = config.files != self.config.files;
        // Presets are rows in the index, so a changed preset is a re-index.
        let presets_changed = config.translate.presets != self.config.translate.presets
            || config.timers.presets != self.config.timers.presets;

        self.root
            .set_visible_rows(config.visible_rows(&self.theme.style.metrics));
        let units_changed = self.root.set_weather_units(config.weather.units);
        let arranged = config.root != self.config.root;
        self.root.set_arrangement(config.root.clone());
        self.config = config;
        if (units_changed || arranged) && matches!(self.mode, Mode::Root) {
            self.rerank(cx);
        }

        if files_changed {
            self.open_files(false, cx);
        }
        if presets_changed {
            self.start_indexing(cx);
        }
        self.resize_if_needed(previous_height, cx);
        cx.notify();
    }

    pub fn appearance(&self) -> Appearance {
        self.theme.appearance
    }

    /// Rebuild the drawn rows from the ranked hits and their sections.
    ///
    /// Called whenever the hits change. A header is drawn only over a section
    /// that has a row under it in the list: a *Results* section holding only
    /// the card would otherwise leave its header alone beneath it.
    fn rebuild_rows(&mut self) {
        use crate::view::Row;

        let first = usize::from(self.card_rows() > 0);
        let case = self.theme.style.variants.section_case;
        let mut rows = Vec::with_capacity(self.hits().len() + self.root.sections().len());
        if self.root.sections().is_empty() {
            rows.extend((first..self.hits().len()).map(Row::Item));
        }
        for section in self.root.sections() {
            let items = section.start.max(first)..section.start + section.len;
            if items.is_empty() {
                continue;
            }
            if section.header {
                let title = match case {
                    SectionCase::Title => section.title.clone(),
                    SectionCase::Upper => section.title.to_uppercase(),
                };
                rows.push(Row::Section(title.into(), None));
            }
            rows.extend(items.map(Row::Item));
        }
        self.list.reset(rows.len());
        self.rows = rows;
    }

    /// How tall the list's rows are, in whole rows (I-10).
    ///
    /// The rows a full window would hold, less the card's, accumulated whole
    /// until the next would not fit -- as a view's are (7.29), and for the
    /// same reason: with headers shorter than rows, capping the sum would
    /// leave a row cut through at the bottom edge.
    fn rows_height(&self) -> f32 {
        let metrics = self.metrics();
        let cap = self.config.list_height(metrics) - self.card_height();
        let mut used = 0.0;
        for row in &self.rows {
            let height = crate::view::row_height(row, metrics);
            if used + height > cap {
                break;
            }
            used += height;
        }
        used
    }

    /// How many rows the calculator's card takes (F14): the style's
    /// `card.rows` when the first result is its answer, and none otherwise.
    fn card_rows(&self) -> usize {
        let answered = matches!(self.mode, Mode::Root)
            && self
                .hits()
                .first()
                .is_some_and(|hit| lumen_core::providers::calculator::is_answer(self.item(hit)));
        if answered {
            self.metrics().card_rows()
        } else {
            0
        }
    }

    /// How tall the card's place is: its rows, and its header when the style
    /// draws one. Nothing at all when there is no answer.
    fn card_height(&self) -> f32 {
        card_extent(self.card_rows(), self.metrics(), &self.theme.style.variants)
    }

    /// How tall the window should be for whatever it is currently showing.
    ///
    /// The scope picker replaces the list and its rows are shorter than a
    /// result row. The action panel floats over it (7.45), so the window is the
    /// list's own height unless the popover needs more than that.
    pub fn content_height(&self) -> f32 {
        if self.picker.is_some() {
            // At least one row: a search that matches nothing says so.
            let rows = self
                .picker_count()
                .clamp(1, lumen_core::layout::view::PICKER_ROWS);
            return self.config.picker_height(self.metrics(), rows);
        }

        let list = self.underlying_height();
        let groups = self.panel_groups();
        if self.action_panel.is_some() && !groups.is_empty() {
            return self.config.panel_height(
                self.metrics(),
                list,
                lumen_core::layout::action_panel::height(&groups, self.metrics().actions_rows()),
            );
        }
        list
    }

    /// The height of the window for what is behind the action panel: what it
    /// would be with the panel closed. Also what the list is laid out to, so
    /// a window grown for the popover does not stretch it.
    fn underlying_height(&self) -> f32 {
        let metrics = self.metrics();
        match &self.mode {
            // Nothing found: the search field and the footer, as before.
            Mode::Root if self.hits().is_empty() => self.config.window_height(metrics, 0),
            Mode::Root => {
                metrics.search_height
                    + metrics.list_padding * 2.0
                    + self.card_height()
                    + self.rows_height()
                    + metrics.footer_height
            }
            Mode::Form(state) => metrics.search_height + state.height + metrics.footer_height,
            // Loading too: a view that has not arrived has no rows, and a view
            // with no rows is as tall as a full list, so arriving resizes
            // nothing (7.48).
            Mode::View(state) => {
                metrics.search_height
                    + metrics.list_padding * 2.0
                    + self.view_rows_height(state)
                    + metrics.footer_height
            }
        }
    }

    /// How much vertical space a view's rows take: the style's list height,
    /// as the root list's is, through `view::rows_height`.
    fn view_rows_height(&self, state: &ViewState) -> f32 {
        use crate::view::Shape;

        let metrics = self.metrics();
        let shape = match &state.view {
            lumen_core::view::View::List(list) if list.showing_detail => Shape::Pane,
            lumen_core::view::View::List(_) => Shape::List,
            lumen_core::view::View::Detail(_) => Shape::Detail,
        };
        crate::view::rows_height(
            &state.rows,
            shape,
            metrics,
            self.config.list_height(metrics),
            self.config.full_list_height(metrics),
        )
    }

    /// Ask for a resize, but only when the height actually changed.
    ///
    /// Emitting on every keystroke would ask the platform to move the window to
    /// the rectangle it is already in, which is a `SetWindowPos` and a fresh
    /// swap-chain buffer for nothing.
    fn resize_if_needed(&mut self, previous: f32, cx: &mut Context<Self>) {
        let height = self.content_height();
        if height != previous {
            cx.emit(LauncherEvent::Resized {
                height,
                anchor: self.anchor_height(),
                width: self.width(),
            });
        }
    }

    /// Close the action panel, and the browser page that shares it, and give
    /// the window back the height the list needs. `false` when it was not open.
    ///
    /// The one way to close the panel from outside a function that resizes
    /// anyway: the window may have grown for the popover, so closing it
    /// without this leaves the list drawn into a window taller than it.
    fn close_action_panel(&mut self, cx: &mut Context<Self>) -> bool {
        let previous_height = self.content_height();
        let was_open = self.action_panel.take().is_some();
        self.browser_menu = None;
        self.resize_if_needed(previous_height, cx);
        was_open
    }

    fn rerank(&mut self, cx: &mut Context<Self>) {
        // A form has no list to rank. The file index is still built while it
        // is open -- files are indexed whatever the launcher shows -- and the
        // root list is ranked when the form is left.
        if matches!(self.mode, Mode::Form(_)) {
            return;
        }

        let previous_height = self.content_height();

        // While the picker is open the field searches its options, and the
        // view is neither filtered nor told (F9).
        if self.picker.is_some() {
            self.refilter_picker();
            self.resize_if_needed(previous_height, cx);
            return;
        }

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

        // The ordering rules -- answers first, files in their own section at
        // the end -- are `RootSearch`'s. Nothing in it waits on anything
        // (I-3): the files are whatever the file index has answered so far.
        match &self.argument {
            Some(argument) => {
                self.root.rank_argument(&argument.id, &self.query);
            }
            None => {
                self.root.rank(&self.query);
            }
        }

        // A conversion with no table, or one that is due: the daemon fetches
        // one, and the answer appears when it lands. This keystroke is not
        // held for it (I-3).
        if !self.rates_asked && self.root.wants_rates(lumen_core::currency::now) {
            self.rates_asked = true;
            // Asked again: the row says *Fetching* while it is out (F14).
            self.root.set_rates_failed(false);
            cx.emit(LauncherEvent::RefreshRates);
        }

        // A forecast missing or old for a city named whole: the daemon
        // fetches it, and the answer fills in when it lands (F13). Asked once
        // per city per showing, so a failing one is not retried on every key.
        if let Some(city) = self.root.wants_weather()
            && self.weather_asked != Some(city)
        {
            self.weather_asked = Some(city);
            cx.emit(LauncherEvent::RefreshWeather { city });
        }

        // A question about time before the places are in memory: they are
        // read off this thread, and the answer appears when they are (F13).
        if !self.places_asked && self.root.wants_places() {
            self.places_asked = true;
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .spawn(async {
                        lumen_core::places::catalogue();
                    })
                    .await;
                this.update(cx, |launcher, cx| {
                    if matches!(launcher.mode, Mode::Root) {
                        launcher.rerank(cx);
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }

        // The selection always returns to the top on a new query. Trying to keep
        // it on "the same item" across a re-rank is the kind of cleverness that
        // makes a launcher feel unpredictable.
        self.selected = 0;

        // A panel describing the previous row must not survive into the next
        // one, or Enter runs an action the user can no longer see.
        self.action_panel = None;

        self.rebuild_rows();
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
            // The same: translating is network work, and that is the daemon's
            // too. It calls `begin_view` straight back.
            lumen_core::ViewSource::Translate { preset } => {
                cx.emit(LauncherEvent::RunTranslate {
                    preset: preset.clone(),
                });
                return;
            }
            // The timers are the daemon's (F8), and so is the view of them.
            lumen_core::ViewSource::Timers => {
                cx.emit(LauncherEvent::RunTimers);
                return;
            }
            lumen_core::ViewSource::AlertSounds => {
                cx.emit(LauncherEvent::RunAlertSounds);
                return;
            }
            // Rates are fetched by the daemon, like translations.
            lumen_core::ViewSource::Currency => {
                cx.emit(LauncherEvent::RunCurrency);
                return;
            }
            // Definitions are looked up by the daemon, like translations.
            lumen_core::ViewSource::Dictionary => {
                cx.emit(LauncherEvent::RunDictionary);
                return;
            }
            // The file and the clipboard are the daemon's (F11).
            lumen_core::ViewSource::Snippets => {
                cx.emit(LauncherEvent::RunSnippets { query: None });
                return;
            }
            // The database and the clipboard are the daemon's (F12).
            lumen_core::ViewSource::Clipboard => {
                cx.emit(LauncherEvent::RunClipboard);
                return;
            }
            // The places are read and the minute is ticked by the daemon
            // (F13), off this thread.
            lumen_core::ViewSource::Clock => {
                cx.emit(LauncherEvent::RunClock);
                return;
            }
            // Forecasts are fetched by the daemon, like rates (F13).
            lumen_core::ViewSource::Weather { city } => {
                cx.emit(LauncherEvent::RunWeather { city: city.clone() });
                return;
            }
            // The file and the clipboard are the daemon's (F14).
            lumen_core::ViewSource::CalculatorHistory => {
                cx.emit(LauncherEvent::RunCalculatorHistory);
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
        self.picker = None;
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

    /// How tall the window is with a full root list: where its top edge is
    /// placed from, so the search field stays still as the results change.
    pub fn anchor_height(&self) -> f32 {
        let metrics = self.metrics();
        metrics.search_height
            + metrics.list_padding * 2.0
            + self.config.full_list_height(metrics)
            + metrics.footer_height
    }

    /// The window's width: the style's, or the config's when it sets one.
    pub fn width(&self) -> f32 {
        self.config.width(self.metrics())
    }

    /// The active style's geometry, as numbers, for sizing the window.
    fn metrics(&self) -> &lumen_core::style::Metrics {
        &self.theme.style.metrics
    }

    /// The active style's geometry, for drawing.
    fn geometry(&self) -> Geometry {
        Geometry::of(&self.theme)
    }

    /// Take over the list for an extension that is starting (F4, step 3).
    ///
    /// Called before the extension has said anything. The launcher shows that
    /// it is loading rather than freezing or staying on the root list: the user
    /// pressed Enter and something has to acknowledge it. As tall as a full
    /// list, as the view will be if it turns out empty (7.48).
    pub fn begin_view(&mut self, sink: ViewSink, cx: &mut Context<Self>) {
        let previous_height = self.content_height();

        self.clear_query_text(cx);
        self.action_panel = None;
        self.argument = None;
        self.picker = None;
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

        // The row the user is on, by the view's own id, so that a re-render
        // does not move them. Filtering puts the selection on the first row,
        // which is right for a new query and wrong for an answer arriving:
        // the translator fills its rows in one at a time, and the selection
        // jumping to the top as each lands is the list moving under the user.
        let selected_id = match &state.view {
            lumen_core::view::View::List(list) => {
                list.item(state.selected).map(|item| item.id.clone())
            }
            lumen_core::view::View::Detail(_) => None,
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
        self.keep_selection(selected_id);

        // An open picker survives a render, which may have changed its
        // options; it closes if its scope is gone.
        let scope_left = self
            .picker
            .as_ref()
            .and_then(|picker| self.scopes().get(picker.scope))
            .is_some_and(|scope| !scope.options.is_empty());
        if scope_left {
            self.refilter_picker();
        } else {
            self.take_picker(cx);
        }
        self.resize_if_needed(previous_height, cx);
        cx.notify();
    }

    /// Put the selection back on the item with this id, if it is still shown.
    fn keep_selection(&mut self, id: Option<String>) {
        let Some(id) = id else {
            return;
        };
        let Mode::View(state) = &mut self.mode else {
            return;
        };
        let lumen_core::view::View::List(list) = &state.view else {
            return;
        };
        let Some(index) = list.items().position(|item| item.id == id) else {
            return;
        };
        if state
            .rows
            .iter()
            .any(|row| matches!(row, crate::view::Row::Item(item) if *item == index))
        {
            state.selected = index;
            self.reveal_view_selection();
        }
    }

    /// Scroll the open view so its selected item is fully visible.
    ///
    /// By the row, not the item: a header occupies a row too, and scrolling
    /// to the item's index would stop short by one per section.
    fn reveal_view_selection(&self) {
        let Mode::View(state) = &self.mode else {
            return;
        };
        let Some(row) = state.rows.iter().position(
            |row| matches!(row, crate::view::Row::Item(index) if *index == state.selected),
        ) else {
            return;
        };
        let top = state.list.logical_scroll_top();
        let (item_ix, offset) = crate::view::reveal(
            &state.rows,
            self.metrics(),
            (top.item_ix, f32::from(top.offset_in_item)),
            row,
            self.view_rows_height(state),
        );
        state.list.scroll_to(gpui::ListOffset {
            item_ix,
            offset_in_item: gpui::px(offset),
        });
    }

    /// Close the launcher on the view's behalf: something it did is finished,
    /// such as copying a translation.
    /// The timers as they are now, for the footer (F8).
    pub fn set_timers(&mut self, clock: lumen_core::timers::Clock, cx: &mut Context<Self>) {
        self.timers = (!clock.is_idle()).then_some(clock);
        self.schedule_tick(cx);
        cx.notify();
    }

    /// Redraw when the footer's clock next turns over -- only while the window
    /// is active and something is counting. Hidden, nothing wakes (section 5).
    fn schedule_tick(&mut self, cx: &mut Context<Self>) {
        self.tick += 1;
        if !self.active {
            return;
        }
        let Some(wait) = self
            .timers
            .as_ref()
            .and_then(|clock| lumen_core::timers::view::next_tick(clock, timers_now()))
        else {
            return;
        };
        let asked = self.tick;
        cx.spawn(async move |this, cx| {
            // A little after, never before: early would redraw the same second.
            let wait = wait + std::time::Duration::from_millis(5);
            cx.background_executor().timer(wait).await;
            this.update(cx, |launcher, cx| {
                if launcher.tick == asked {
                    cx.notify();
                    launcher.schedule_tick(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// The footer's line about the timers, if there is one to say.
    fn timers_line(&self) -> Option<String> {
        let clock = self.timers.as_ref()?;
        lumen_core::timers::view::summary(clock, timers_now())
    }

    /// Whether focus is on, and what it blocks (F8).
    pub fn set_focus(
        &mut self,
        guard: Option<lumen_core::timers::focus::Guard>,
        _cx: &mut Context<Self>,
    ) {
        if guard.is_some() != self.focus.is_some() {
            log::debug!("focus is {}", if guard.is_some() { "on" } else { "off" });
        }
        self.focus = guard;
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(LauncherEvent::Dismissed);
    }

    /// Close and paste `paste` into the window the launcher was opened over
    /// (F11).
    ///
    /// In place of the dismissal, as arranging is: the daemon hides, which
    /// hands the foreground back, and keeps the handle that hiding would
    /// otherwise consume, to paste into.
    pub fn paste(&mut self, paste: lumen_platform::Paste, cx: &mut Context<Self>) {
        cx.emit(LauncherEvent::Paste(paste));
    }

    /// A table of exchange rates the daemon fetched or read (F9), for the
    /// root list's conversions.
    pub fn set_rates(
        &mut self,
        rates: Option<lumen_core::currency::Rates>,
        cx: &mut Context<Self>,
    ) {
        if self.root.replace_rates(rates) && matches!(self.mode, Mode::Root) {
            self.rerank(cx);
            cx.notify();
        }
    }

    /// The table asked for from the root list could not be fetched (F14):
    /// a calculation in money says so, or falls back to the table it has.
    pub fn rates_failed(&mut self, cx: &mut Context<Self>) {
        if self.root.set_rates_failed(true) && matches!(self.mode, Mode::Root) {
            self.rerank(cx);
            cx.notify();
        }
    }

    /// The forecasts the daemon keeps (F13), for the root list's answers.
    pub fn set_weather(&mut self, cache: lumen_core::weather::Cache, cx: &mut Context<Self>) {
        if self.root.replace_weather(cache) && matches!(self.mode, Mode::Root) {
            self.rerank(cx);
            cx.notify();
        }
    }

    /// A forecast asked for from the root list could not be fetched (F13).
    pub fn weather_failed(
        &mut self,
        key: String,
        failure: lumen_core::weather::Failure,
        cx: &mut Context<Self>,
    ) {
        self.root.weather_failed(key, failure);
        if matches!(self.mode, Mode::Root) {
            self.rerank(cx);
            cx.notify();
        }
    }

    /// Tab and Shift+Tab: open the picker on the first or last scope, or,
    /// with it open, move it to the next or previous scope.
    fn step_scope(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.scopes().len();
        if count == 0 {
            return;
        }
        let scope = match &self.picker {
            Some(picker) => (picker.scope as isize + delta).rem_euclid(count as isize) as usize,
            None if delta < 0 => count - 1,
            None => 0,
        };
        self.open_picker(scope, cx);
    }

    fn next_scope(&mut self, _: &NextScope, _: &mut Window, cx: &mut Context<Self>) {
        self.step_scope(1, cx);
    }

    fn previous_scope(&mut self, _: &PreviousScope, _: &mut Window, cx: &mut Context<Self>) {
        self.step_scope(-1, cx);
    }

    /// Show `scope`'s options in place of the list, on the one in force, and
    /// empty the field to search them.
    fn open_picker(&mut self, scope: usize, cx: &mut Context<Self>) {
        if self.scopes().get(scope).is_none() {
            return;
        }
        let previous_height = self.content_height();
        self.action_panel = None;
        self.browser_menu = None;
        // Moving to another scope keeps the query the first one set aside:
        // what is in the field now is only a search of the options.
        let query = match self.picker.take() {
            Some(picker) => picker.query,
            None => self.query.clone(),
        };
        self.clear_query_text(cx);
        self.picker = Some(Picker {
            scope,
            matches: Vec::new(),
            highlighted: 0,
            scroll: UniformListScrollHandle::new(),
            query,
            filtered: String::new(),
        });
        self.refilter_picker();
        if let Some(picker) = &self.picker {
            picker
                .scroll
                .scroll_to_item(picker.highlighted, ScrollStrategy::Center);
        }
        self.resize_if_needed(previous_height, cx);
        cx.notify();
    }

    fn close_picker(&mut self, cx: &mut Context<Self>) {
        let previous_height = self.content_height();
        if self.take_picker(cx) {
            self.resize_if_needed(previous_height, cx);
            cx.notify();
        }
    }

    /// Drop the picker and give the field back the view's query. Not an
    /// edit: the view was never told the picker's search, so it has nothing
    /// to be told back.
    fn take_picker(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(picker) = self.picker.take() else {
            return false;
        };
        self.query.clone_from(&picker.query);
        self.query_field
            .update(cx, |field, cx| field.set_text(picker.query, cx));
        true
    }

    /// Match the open picker's options against the field, with the root
    /// list's matcher, so `can` finds the Canadian dollar as it would an app.
    ///
    /// The highlight goes to the best match when the search changes, to the
    /// option in force when there is no search, and otherwise stays on the
    /// option it was on: a render must not move it.
    fn refilter_picker(&mut self) {
        let query = self.query.trim().to_string();
        let Some(picker) = &self.picker else {
            return;
        };
        let Some(scope) = self.scopes().get(picker.scope) else {
            return;
        };
        let before = picker.matches.get(picker.highlighted).copied();
        let in_force = scope.position();
        let candidates: Option<Vec<Candidate>> =
            (!query.is_empty()).then(|| scope.options.iter().map(option_candidate).collect());
        let every = scope.options.len();

        let matches: Vec<usize> = match candidates {
            Some(candidates) => self.root.filter(&query, &candidates),
            None => (0..every).collect(),
        };
        let Some(picker) = &mut self.picker else {
            return;
        };
        let searched = picker.matches.is_empty() || query != picker.filtered;
        let target = match before {
            _ if searched && !query.is_empty() => None,
            Some(before) if !searched => Some(before),
            _ => Some(in_force),
        };
        picker.highlighted = target
            .and_then(|target| matches.iter().position(|index| *index == target))
            .unwrap_or(0);
        picker.matches = matches;
        if searched {
            picker
                .scroll
                .scroll_to_item(picker.highlighted, ScrollStrategy::Top);
        }
        picker.filtered = query;
    }

    /// Move the picker's highlight by `delta`, wrapping; or to an end.
    fn move_picker(&mut self, to: PickerMove, cx: &mut Context<Self>) {
        let count = self.picker_count();
        let Some(picker) = &mut self.picker else {
            return;
        };
        if count == 0 {
            return;
        }
        picker.highlighted = match to {
            PickerMove::By(delta) => {
                (picker.highlighted as isize + delta).rem_euclid(count as isize) as usize
            }
            PickerMove::First => 0,
            PickerMove::Last => count - 1,
        };
        picker
            .scroll
            .scroll_to_item(picker.highlighted, ScrollStrategy::Top);
        cx.notify();
    }

    /// Take the highlighted option: drawn at once rather than when the view
    /// answers, so the pill never lags the key, and told to the view, whose
    /// next render confirms it.
    fn choose_in_picker(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = &self.picker else {
            return;
        };
        let scope_index = picker.scope;
        let Some(highlighted) = picker.matches.get(picker.highlighted).copied() else {
            return;
        };
        let Mode::View(state) = &mut self.mode else {
            return;
        };
        let lumen_core::view::View::List(list) = &mut state.view else {
            return;
        };
        let Some(scope) = list.scopes.get_mut(scope_index) else {
            return;
        };
        let Some(option) = scope
            .options
            .get(highlighted)
            .map(|option| option.id.clone())
        else {
            return;
        };
        let changed = scope.selected != option;
        scope.selected.clone_from(&option);
        let id = scope.id.clone();

        self.close_picker(cx);
        if changed {
            self.tell_extension(lumen_core::view::Event::ScopeChanged { scope: id, option });
        }
    }

    /// The open view's scopes; none outside a list view.
    fn scopes(&self) -> &[lumen_core::view::Scope] {
        match &self.mode {
            Mode::View(state) => match &state.view {
                lumen_core::view::View::List(list) => &list.scopes,
                lumen_core::view::View::Detail(_) => &[],
            },
            _ => &[],
        }
    }

    /// The open view's levels, outermost first (F13): what its pill says.
    fn view_path(&self) -> &[String] {
        match &self.mode {
            Mode::View(state) => match &state.view {
                lumen_core::view::View::List(list) => &list.path,
                lumen_core::view::View::Detail(_) => &[],
            },
            _ => &[],
        }
    }

    /// Take the innermost level off the view's pill, if it has one: the view
    /// is told, and answers with the level above. `false` with no pill, so
    /// the key means what it meant before.
    fn back_out_of_path(&mut self, cx: &mut Context<Self>) -> bool {
        if self.view_path().is_empty() || self.picker.is_some() {
            return false;
        }
        self.close_action_panel(cx);
        self.tell_extension(lumen_core::view::Event::Back);
        true
    }

    /// Whether somebody can see the launcher: what a view that ticks asks
    /// before it asks for the next tick (F13). Hidden, nothing wakes.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// How many options the open picker shows: those that match.
    fn picker_count(&self) -> usize {
        self.picker
            .as_ref()
            .map_or(0, |picker| picker.matches.len())
    }

    /// The picker's `row`th option as shown, and whether it is in force.
    fn picker_option(&self, row: usize) -> Option<(&lumen_core::view::ScopeOption, bool)> {
        let picker = self.picker.as_ref()?;
        let scope = self.scopes().get(picker.scope)?;
        let option = scope.options.get(*picker.matches.get(row)?)?;
        Some((option, scope.selected == option.id))
    }

    /// The query the open view is filtered by: the field's, or while the
    /// picker searches its options, the one it set aside.
    fn view_query(&self) -> &str {
        match &self.picker {
            Some(picker) => &picker.query,
            None => &self.query,
        }
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
        self.images.release(cx);

        let previous_height = self.content_height();
        self.picker = None;
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
        let query = self.view_query().trim().to_string();

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
                rows.insert(
                    first,
                    crate::view::Row::Section(
                        title.clone().into(),
                        section.subtitle.clone().map(Into::into),
                    ),
                );
            }
        }

        // The selection lands on the first surviving row. Keeping it on an item
        // the filter just removed would leave Enter pointing at something that
        // is no longer on screen.
        state.selected = rows
            .iter()
            .find_map(|row| match row {
                crate::view::Row::Item(index) => Some(*index),
                crate::view::Row::Section(..) => None,
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
                crate::view::Row::Section(..) => None,
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
        self.reveal_view_selection();

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

    /// The actions available for the selected row: its own, then what can
    /// be done to it as a favourite (F15).
    fn available_actions(&self) -> Vec<Action> {
        let Some(hit) = self.hits().get(self.selected) else {
            return Vec::new();
        };
        let mut actions = self.item(hit).actions.clone();
        if matches!(self.mode, Mode::Root) {
            actions.extend(
                self.root
                    .favorite_changes(hit)
                    .into_iter()
                    .map(Action::Favorite),
            );
        }
        actions
    }

    /// Add, remove or move the selected row as a favourite, if that is
    /// something it can have done to it from where it is.
    fn change_favorite(&mut self, change: FavoriteChange, cx: &mut Context<Self>) {
        if !matches!(self.mode, Mode::Root) || self.argument.is_some() {
            return;
        }
        let Some(hit) = self.hits().get(self.selected).cloned() else {
            return;
        };
        if !self.root.favorite_changes(&hit).contains(&change) {
            return;
        }
        let id = self.item(&hit).id.clone();
        let Some(favorites) = self.root.change_favorite(change, &id) else {
            return;
        };
        // On the background executor, like frecency: disk I/O for a list of
        // a few ids is not worth a frame.
        let path = self.paths.favorites_file();
        cx.background_spawn(async move {
            if let Err(error) = favorites.save(&path) {
                log::warn!("could not save the favourites: {error:#}");
            }
        })
        .detach();
        self.rerank_keeping(&id, cx);
    }

    fn toggle_favorite(&mut self, _: &ToggleFavorite, _: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self.hits().get(self.selected) else {
            return;
        };
        let changes = self.root.favorite_changes(hit);
        let change = if changes.contains(&FavoriteChange::Add) {
            FavoriteChange::Add
        } else {
            FavoriteChange::Remove
        };
        self.change_favorite(change, cx);
    }

    fn move_favorite_up(&mut self, _: &MoveFavoriteUp, _: &mut Window, cx: &mut Context<Self>) {
        self.change_favorite(FavoriteChange::MoveUp, cx);
    }

    fn move_favorite_down(&mut self, _: &MoveFavoriteDown, _: &mut Window, cx: &mut Context<Self>) {
        self.change_favorite(FavoriteChange::MoveDown, cx);
    }

    /// Rank again, keeping the selection on `id` wherever it lands, so a
    /// favourite moved or added is still the row under the user's eye.
    fn rerank_keeping(&mut self, id: &str, cx: &mut Context<Self>) {
        self.rerank(cx);
        if let Some(at) = self.hits().iter().position(|hit| self.item(hit).id == id) {
            self.selected = at;
            self.reveal_row();
        }
        cx.notify();
    }

    /// The action panel's rows, whichever mode is showing.
    ///
    /// One shape for both, because the panel is one piece of interface. A root
    /// row's actions are Lumen's own and name themselves with a verb; a view's
    /// are the extension's and name themselves with whatever it chose, plus the
    /// key combination it wants shown beside them.
    fn panel_entries(&self) -> Vec<PanelEntry> {
        if self.showing_browsers() {
            return self
                .browsers
                .iter()
                .map(|name| PanelEntry {
                    title: name.clone(),
                    shortcut: Vec::new(),
                    glyph: Some(lumen_core::view::Icon::Glyph {
                        name: "globe".into(),
                    }),
                    group: ActionGroup::Primary,
                })
                .collect();
        }
        match &self.mode {
            Mode::Root => root_panel_entries(self.available_actions(), &self.shortcuts),
            Mode::View(state) => state
                .view
                .actions(state.selected)
                .iter()
                .map(|action| PanelEntry {
                    title: action.title.clone(),
                    shortcut: action
                        .shortcut
                        .as_deref()
                        .map(crate::overlay::keycaps)
                        .unwrap_or_default(),
                    glyph: action.icon.clone(),
                    group: ActionGroup::Primary,
                })
                .collect(),
            Mode::Form(_) => Vec::new(),
        }
    }

    /// The groups of the panel's rows, in order: what decides where the rules
    /// go and how tall the popover is.
    fn panel_groups(&self) -> Vec<ActionGroup> {
        self.panel_entries()
            .iter()
            .map(|entry| entry.group)
            .collect()
    }

    /// What the popover's header names: the thing the actions are for.
    ///
    /// The selected row's title in the root list and in a view; nothing for a
    /// detail view, which has no row to name. The header's line is reserved
    /// either way (I-10).
    fn panel_header(&self) -> String {
        if self.showing_browsers() {
            return "Open With\u{2026}".into();
        }
        match &self.mode {
            Mode::Root => self
                .selected_item()
                .map(|item| item.title.clone())
                .unwrap_or_default(),
            Mode::View(state) => match &state.view {
                lumen_core::view::View::List(list) => list
                    .item(state.selected)
                    .map(|item| item.title.clone())
                    .unwrap_or_default(),
                lumen_core::view::View::Detail(_) => String::new(),
            },
            Mode::Form(_) => String::new(),
        }
    }

    /// Scroll the popover just far enough to show the highlighted row.
    ///
    /// The rules between groups are children of the scrolling container too,
    /// so the row's place among them is not its index.
    fn reveal_panel_selection(&self) {
        let Some(selected) = self.action_panel else {
            return;
        };
        self.panel_scroll
            .scroll_to_item(layout::action_panel::child_index(
                &self.shown_panel_groups(),
                selected,
            ));
    }

    /// The panel's rows as drawn, by their place among its entries: every
    /// one, or those its search keeps.
    fn panel_rows(&self) -> Vec<usize> {
        visible_panel_rows(self.panel_entries().len(), self.panel_search.as_deref())
    }

    /// The groups of the rows drawn, which is where the rules go: only
    /// between two shown rows of different groups.
    fn shown_panel_groups(&self) -> Vec<ActionGroup> {
        let groups = self.panel_groups();
        self.panel_rows()
            .into_iter()
            .map(|row| groups[row])
            .collect()
    }

    /// The entry the highlight is on, by its place among all of them; `None`
    /// when the panel is closed or its search left nothing.
    fn panel_choice(&self) -> Option<usize> {
        self.action_panel
            .and_then(|at| self.panel_rows().get(at).copied())
    }

    /// Narrow the panel to what its field says, with the root list's matcher,
    /// and put the highlight on the first row left.
    fn refilter_panel(&mut self, text: &str, cx: &mut Context<Self>) {
        let entries = self.panel_entries();
        let matches = panel_matches(
            entries.iter().map(|entry| entry.title.as_str()),
            text,
            |query, candidates| self.root.filter(query, candidates),
        );
        self.panel_search = matches;
        self.action_panel = refiltered_selection(self.action_panel);
        self.reveal_panel_selection();
        cx.notify();
    }

    /// Empty the panel's field without treating it as an edit.
    fn reset_panel_search(&mut self, cx: &mut Context<Self>) {
        self.panel_search = None;
        self.action_field
            .update(cx, |field, cx| field.set_text(String::new(), cx));
    }

    /// Once the panel has closed, give the query its focus back and forget the
    /// panel's search.
    ///
    /// Settled where the launcher is drawn rather than at each way of closing
    /// it -- Escape, Ctrl+K, an action run, a click outside, a re-rank -- so
    /// that a new way cannot leave the keys going to a field nobody can see.
    fn settle_panel_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.action_panel.is_some() {
            return;
        }
        if self.action_field.focus_handle(cx).is_focused(window) {
            self.query_field.read(cx).focus(window);
        }
        if self.panel_search.is_some() || !self.action_field.read(cx).text().is_empty() {
            self.reset_panel_search(cx);
        }
    }

    /// Whether the open panel lists browsers rather than actions.
    fn showing_browsers(&self) -> bool {
        self.action_panel.is_some() && self.browser_menu.is_some()
    }

    /// Replace the panel's actions with the installed browsers.
    fn choose_browser(
        &mut self,
        open: lumen_core::providers::quicklinks::OpenLink,
        cx: &mut Context<Self>,
    ) {
        if self.browsers.is_empty() {
            // Not read yet, or none registered: opening the link where it
            // would have opened is better than a panel with nothing in it.
            log::warn!("no installed browsers to choose from");
            return;
        }
        let previous_height = self.content_height();
        // The same field searches the browsers, from nothing typed.
        self.reset_panel_search(cx);
        self.browser_menu = Some(open);
        self.action_panel = Some(0);
        self.reveal_panel_selection();
        self.resize_if_needed(previous_height, cx);
        cx.notify();
    }

    fn toggle_action_panel(
        &mut self,
        _: &ToggleActionPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The panel and the picker share the list's place; the panel wins.
        self.close_picker(cx);
        let previous_height = self.content_height();
        // Ctrl+K from the browser list closes the panel altogether, like
        // from the actions: it is one panel, on two pages.
        self.browser_menu = None;
        self.reset_panel_search(cx);

        self.action_panel = match self.action_panel {
            Some(_) => None,
            None => opening_selection(self.panel_entries().len()),
        };
        // Opening moves the keys to the panel's field, empty; the query keeps
        // its text. Closing gives them back: see `settle_panel_focus`.
        if self.action_panel.is_some() {
            self.action_field.read(cx).focus(window);
        } else {
            self.settle_panel_focus(window, cx);
        }
        self.reveal_panel_selection();

        self.resize_if_needed(previous_height, cx);
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        if self.picker.is_some() {
            self.move_picker(PickerMove::By(1), cx);
            return;
        }
        // While the panel is open the arrow keys move within it. Two lists on
        // screen and one set of arrow keys: whichever is in front wins.
        if let Some(selected) = self.action_panel {
            let count = self.panel_rows().len();
            if count > 0 {
                self.action_panel = Some((selected + 1) % count);
                self.reveal_panel_selection();
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
        if self.picker.is_some() {
            self.move_picker(PickerMove::By(-1), cx);
            return;
        }
        if let Some(selected) = self.action_panel {
            let count = self.panel_rows().len();
            if count > 0 {
                self.action_panel = Some(selected.checked_sub(1).unwrap_or(count - 1));
                self.reveal_panel_selection();
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
        if self.picker.is_some() {
            self.move_picker(PickerMove::First, cx);
            return;
        }
        if matches!(self.mode, Mode::View(_)) {
            let first = self.view_items().first().copied();
            if let (Some(first), Mode::View(state)) = (first, &mut self.mode) {
                state.selected = first;
                self.reveal_view_selection();
                cx.notify();
            }
            return;
        }

        self.selected = 0;
        self.reveal_selection(cx);
    }

    fn select_last(&mut self, _: &SelectLast, _: &mut Window, cx: &mut Context<Self>) {
        if self.picker.is_some() {
            self.move_picker(PickerMove::Last, cx);
            return;
        }
        if matches!(self.mode, Mode::View(_)) {
            let last = self.view_items().last().copied();
            if let (Some(last), Mode::View(state)) = (last, &mut self.mode) {
                state.selected = last;
                self.reveal_view_selection();
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
        self.close_action_panel(cx);
        // F2 acceptance criterion: the list jumps directly to the selection.
        // There is no smooth scrolling, because smooth scrolling in a list being
        // driven by arrow keys is motion the user did not ask for.
        self.reveal_row();
        cx.notify();
    }

    /// Scroll the root list just enough to show the selected row, by its
    /// known height (I-10); moving up to a section's first row shows its
    /// header too. The card is above the list, so selecting it puts the
    /// list back at its start.
    fn reveal_row(&self) {
        let Some(row) = self.rows.iter().position(
            |row| matches!(row, crate::view::Row::Item(index) if *index == self.selected),
        ) else {
            self.list.scroll_to(gpui::ListOffset::default());
            return;
        };
        let top = self.list.logical_scroll_top();
        let (item_ix, offset) = crate::view::reveal(
            &self.rows,
            self.metrics(),
            (top.item_ix, f32::from(top.offset_in_item)),
            row,
            self.rows_height(),
        );
        self.list.scroll_to(gpui::ListOffset {
            item_ix,
            offset_in_item: gpui::px(offset),
        });
    }

    fn activate(&mut self, _: &Activate, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.is_some() {
            self.choose_in_picker(cx);
            return;
        }
        // The panel's search left nothing: there is nothing for Enter to run,
        // and running the row's primary instead would be a surprise.
        let choice = self.panel_choice();
        if self.action_panel.is_some() && choice.is_none() {
            return;
        }
        if matches!(self.mode, Mode::View(_)) {
            self.activate_in_view(cx);
            return;
        }

        let menu = self.browser_menu.take();
        let Some(item) = self.selected_item() else {
            return;
        };

        // Whichever action the panel is pointing at, or the primary when it is
        // closed. Falling back to the primary rather than doing nothing means a
        // panel left open on a row whose actions have changed still does the
        // obvious thing. On the browser page, the link in the chosen browser.
        let action = match (choice, menu) {
            (Some(index), Some(open)) => match self.browsers.get(index) {
                Some(name) => Action::OpenLink(lumen_core::providers::quicklinks::OpenLink {
                    browser: Some(name.clone()),
                    ..open
                }),
                None => Action::OpenLink(open),
            },
            (Some(index), None) => self
                .available_actions()
                .get(index)
                .cloned()
                .unwrap_or(item.primary()),
            (None, _) => item.primary(),
        };

        self.run_action(action, window, cx);
    }

    /// Ctrl+Enter: the second of the selected row's actions, which is what the
    /// panel's second row says it does. Nothing when the row has no second
    /// action, and nothing outside the root list: a view's actions are the
    /// extension's to name, and a form or a picker has no row to act on.
    fn run_secondary_action(
        &mut self,
        _: &RunSecondaryAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.mode, Mode::Root)
            || self.picker.is_some()
            || self.argument.is_some()
            || self.browser_menu.is_some()
        {
            return;
        }
        let Some(action) = second_action(self.available_actions()) else {
            return;
        };
        self.run_action(action, window, cx);
    }

    /// Carry out `action` on the selected row: what `Enter` does with the
    /// primary, whichever way the action was reached.
    fn run_action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        // A favourite is changed in place: nothing opens, nothing is
        // recorded as used, and the launcher stays.
        if let Action::Favorite(change) = action {
            self.close_action_panel(cx);
            self.change_favorite(change, cx);
            return;
        }
        let Some(item) = self.selected_item() else {
            return;
        };
        let id = item.id.clone();
        let calculated = calculated(item, &action);
        let blocked = self
            .focus
            .as_ref()
            .and_then(|guard| guard.blocks(item, &action));
        self.close_action_panel(cx);

        // Focus is on and this is on its list (F8): the daemon shows why, in
        // place of the list, with the way through. Nothing is recorded: it
        // has not been opened. A link keeps the browser it would have used.
        if let Some(what) = blocked {
            let action = match action {
                Action::OpenLink(open) => {
                    Action::OpenLink(open.or_browser(self.config.preferred_browser.as_deref()))
                }
                action => action,
            };
            cx.emit(LauncherEvent::Blocked { what, action });
            return;
        }

        // Choosing a browser has opened nothing yet, so, like asking for an
        // argument, it records no use.
        if let Action::ChooseBrowser(open) = action {
            self.choose_browser(open, cx);
            return;
        }

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
        // Except a private window: remembering what was opened in one, even
        // only by name, is what private means not to do.
        let private = matches!(&action, Action::OpenLink(open) if open.private);
        if !private {
            let history = self.root.record_use(&id);
            // A file or folder opened is one a short query finds from now on
            // (F16), so the index takes the new boosts -- by path, from the
            // database, so off this thread.
            if (id.starts_with("file:") || id.starts_with("folder:"))
                && let Some(index) = self.files.clone()
            {
                let history = history.clone();
                cx.background_spawn(async move { index.set_boosts(&history) })
                    .detach();
            }
            let path = self.paths.frecency_file();
            cx.background_spawn(async move {
                if let Err(error) = history.save(&path) {
                    log::warn!("could not persist usage statistics: {error:#}");
                }
            })
            .detach();
        }
        if let Some(calculated) = calculated {
            cx.emit(calculated);
        }

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

        // So do the forms.
        if action == Action::CreateSnippet {
            self.open_snippet_form(Default::default(), None, false, window, cx);
            return;
        }
        if action == Action::CreateQuicklink {
            self.open_form(None, window, cx);
            return;
        }
        if let Action::SaveAsQuicklink { name, link } = action {
            self.open_form(Some((name, link)), window, cx);
            return;
        }
        if let Action::OpenTimerForm { preset, replacing } = action {
            self.open_timer_form(preset, replacing, window, cx);
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

        // So are the timers (F8): the daemon holds them and the one sleep
        // until the next of them ends.
        if let Action::Timer(command) = action {
            cx.emit(LauncherEvent::Timer(command));
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
                // The launcher is already gone; this is the only place left
                // to say so.
                let (title, body) = lumen_core::item::failure_notice(&action, &error);
                if let Err(error) = platform.notify(&title, &body) {
                    log::warn!("could not show the failure either: {error:#}");
                }
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
        self.close_action_panel(cx);
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
    /// `prefill` is a name and link to start from, for a typed URL.
    fn open_form(
        &mut self,
        prefill: Option<(String, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous_height = self.content_height();

        let path = self.paths.quicklinks_file();
        let colors = self.theme.colors.clone();
        let form = cx.new(|cx| {
            let mut form = QuicklinkForm::new(path, None, colors, cx);
            form.open_menus_upward(cx);
            if let Some((name, link)) = prefill {
                form.prefill(name, link, cx);
            }
            form
        });
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
        self.picker = None;
        self.mode = Mode::Form(FormState {
            form: FormKind::Quicklink(form.clone()),
            title: "Create Quicklink",
            height: lumen_core::layout::form::HEIGHT,
            _events: events,
            _resize: None,
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

    /// Show the timer preset form in place of the list (F8): `preset` to
    /// start from, editing the preset on disk called `replacing` if there is
    /// one. Public for the daemon, which opens it for *Save as Preset* in the
    /// *Timers* view.
    pub fn open_timer_form(
        &mut self,
        preset: lumen_core::timers::PresetConfig,
        replacing: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous_height = self.content_height();
        let title = match (&replacing, preset.duration.is_empty()) {
            (Some(_), _) => "Edit Timer Preset",
            (None, true) => "Create Timer Preset",
            (None, false) => "Save as Timer Preset",
        };

        let path = self.paths.config_file();
        let colors = self.theme.colors.clone();
        let form = cx.new(|cx| {
            let mut form = TimerForm::new(path, preset, replacing, colors, cx);
            form.open_menus_upward(cx);
            form
        });
        let events = cx.subscribe_in(&form, window, |this, _, event, window, cx| {
            let saved = match event {
                TimerFormEvent::Saved(name) => Some(name.clone()),
                TimerFormEvent::Cancelled => None,
                // The form says why, and stays.
                TimerFormEvent::Failed => return,
            };
            this.leave_form(saved, window, cx);
        });
        let resize = cx.observe(&form, |this, form, cx| {
            let previous = this.content_height();
            if let Mode::Form(state) = &mut this.mode {
                state.height = form.read(cx).height();
            }
            this.resize_if_needed(previous, cx);
        });
        let height = form.read(cx).height();

        self.action_panel = None;
        self.argument = None;
        self.clear_query_text(cx);
        self.picker = None;
        self.mode = Mode::Form(FormState {
            form: FormKind::Timer(form.clone()),
            title,
            height,
            _events: events,
            _resize: Some(resize),
        });
        form.read(cx).focus(window, cx);
        self.resize_if_needed(previous_height, cx);
        cx.notify();
    }

    /// Show the snippet form in place of the list (F11): `snippet` to start
    /// from, saving over the snippet called `replacing` when there is one.
    /// `from_view` is whether it was opened from *Search Snippets*, which is
    /// where leaving it without saving goes back to. Public for the daemon,
    /// which opens it for *Edit*, *Duplicate* and *Create* in that view.
    pub fn open_snippet_form(
        &mut self,
        snippet: lumen_core::snippets::Snippet,
        replacing: Option<String>,
        from_view: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous_height = self.content_height();
        let title = if replacing.is_some() {
            "Edit Snippet"
        } else {
            "Create Snippet"
        };

        let path = self.paths.snippets_file();
        let colors = self.theme.colors.clone();
        let form = cx.new(|cx| SnippetForm::new(path, snippet, replacing, colors, cx));
        let events = cx.subscribe_in(&form, window, move |this, _, event, window, cx| {
            // Back to the list either way; a save shows what was saved.
            let query = match event {
                SnippetFormEvent::Saved(name) => Some(name.clone()),
                SnippetFormEvent::Cancelled if from_view => None,
                SnippetFormEvent::Cancelled => {
                    this.leave_form(None, window, cx);
                    return;
                }
            };
            this.leave_form(None, window, cx);
            cx.emit(LauncherEvent::RunSnippets { query });
        });
        let height = form.read(cx).height();

        self.action_panel = None;
        self.argument = None;
        self.clear_query_text(cx);
        self.picker = None;
        self.mode = Mode::Form(FormState {
            form: FormKind::Snippet(form.clone()),
            title,
            height,
            _events: events,
            _resize: None,
        });
        form.read(cx).focus(window, cx);
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

        self.picker = None;
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
        let action = match self.panel_choice() {
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

        self.close_action_panel(cx);
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

        // The picker is the top layer: Escape closes it and nothing else.
        if self.picker.is_some() {
            self.close_picker(cx);
            return;
        }

        // Escape unwinds one layer at a time: the panel, then the query, then
        // the launcher. One key, and it always means "back" -- never "back two
        // steps because the first one happened to be empty".
        // The browser page backs up to the actions, on the row it came from.
        if self.showing_browsers() {
            let previous_height = self.content_height();
            self.reset_panel_search(cx);
            self.browser_menu = None;
            self.action_panel = self
                .available_actions()
                .iter()
                .position(|action| matches!(action, Action::ChooseBrowser(_)));
            self.resize_if_needed(previous_height, cx);
            cx.notify();
            return;
        }
        if self.close_action_panel(cx) {
            cx.notify();
            return;
        }

        // In a view the layers are one deeper: the panel, the view's query,
        // the view, and only then the launcher. Leaving the view on the same
        // key is what makes it feel like somewhere you went rather than
        // somewhere you are stuck.
        if matches!(self.mode, Mode::View(_)) {
            // A pill is one more layer: its text, then each level, then the
            // view (F13).
            if self.query.is_empty() && self.back_out_of_path(cx) {
                cx.notify();
            } else if self.query.is_empty() {
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
    /// Put `text` in the search field of the open view, as if typed (F10).
    ///
    /// For one built-in view opening another on the same word -- *Look Up
    /// "display"* from Translate. The daemon calls it, never a view: an
    /// extension that could write the field could type over the user.
    pub fn set_query(&mut self, text: String, cx: &mut Context<Self>) {
        if !matches!(self.mode, Mode::View(_)) || self.picker.is_some() {
            return;
        }
        self.query.clone_from(&text);
        self.query_field
            .update(cx, |field, cx| field.set_text(text, cx));
        self.rerank(cx);
        cx.notify();
    }

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
    /// The translate view was chosen (F7), on a preset by name or on the last.
    RunTranslate {
        preset: Option<String>,
    },
    /// The *Timers* view was chosen (F8).
    RunTimers,
    /// *Preview Alert Sounds* was chosen (F8, slice 3).
    RunAlertSounds,
    /// The currency view was chosen (F9).
    RunCurrency,
    /// The dictionary was chosen (F10).
    RunDictionary,
    /// *Search Snippets* was chosen (F11), or a snippet was saved: `query`
    /// is put in the search field, so the saved snippet is what is shown.
    RunSnippets {
        query: Option<String>,
    },
    /// *Clipboard History* was chosen (F12).
    RunClipboard,
    /// *World Clock* was chosen (F13).
    RunClock,
    /// *Weather* was chosen (F13), on a city's report or on the list.
    RunWeather {
        city: Option<String>,
    },
    /// `weather in ...` was typed for a city whose forecast is missing or
    /// old (F13). The daemon fetches it and hands the forecasts back with
    /// `Launcher::set_weather`.
    RefreshWeather {
        city: u32,
    },
    /// A conversion was typed in the root list with no table of rates, or
    /// one that is due (F9). The daemon fetches one and hands it back with
    /// `Launcher::set_rates`.
    RefreshRates,
    /// *Calculator History* was chosen (F14).
    RunCalculatorHistory,
    /// A calculation was used -- its answer copied -- and the daemon keeps
    /// it in the calculator's history (F14). Comes before `Dismissed`.
    Calculated {
        expression: String,
        result: String,
    },
    /// Focus refused to open `what` (F8, slice 6). `action` is what would
    /// have run, for *Open Anyway*. Replaces `Dismissed`: the launcher stays
    /// up to say why.
    Blocked {
        what: String,
        action: lumen_core::Action,
    },
    /// A timer is to be started or stopped (F8). Follows `Dismissed`.
    Timer(lumen_core::timers::Command),
    /// A layout was chosen for the window that held the foreground.
    ///
    /// Carries no window handle: the daemon is the only thing that has one,
    /// and it is the same handle it is about to restore focus to. This event
    /// also *replaces* `Dismissed` rather than following it, because hiding
    /// takes that handle and the arrangement still needs it.
    ArrangeWindow(lumen_platform::WindowLayout),
    /// Hide, then paste into the window the launcher was opened over (F11).
    Paste(lumen_platform::Paste),
    /// The content changed size, so the window should be resized to hug it.
    ///
    /// A height rather than a row count, because the launcher has two kinds of
    /// content and they are not made of the same rows: a result row is tall
    /// enough for a title and a subtitle, an action row holds one line. When
    /// this carried a row count the action panel was drawn into a window still
    /// sized for eight results, and two actions sat above six rows of nothing.
    ///
    /// With the anchor: how tall a full root list is in the active style,
    /// which is where the window's top edge is placed from (F15). A style
    /// changes it, and the launcher is what knows the style.
    ///
    /// And the width, which a style sets (F15).
    Resized {
        height: f32,
        anchor: f32,
        width: f32,
    },
}

impl gpui::EventEmitter<LauncherEvent> for Launcher {}

impl Focusable for Launcher {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.query_field.focus_handle(cx)
    }
}

impl Render for Launcher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.settle_panel_focus(window, cx);
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
            .on_action(cx.listener(Self::run_secondary_action))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::clear_query))
            .on_action(cx.listener(Self::toggle_action_panel))
            .on_action(cx.listener(Self::ask_argument))
            .on_action(cx.listener(Self::next_scope))
            .on_action(cx.listener(Self::previous_scope))
            .on_action(cx.listener(Self::toggle_favorite))
            .on_action(cx.listener(Self::move_favorite_up))
            .on_action(cx.listener(Self::move_favorite_down))
            .size_full()
            .flex()
            .flex_col()
            .rounded(self.geometry().window_radius())
            .border_1()
            .border_color(color(colors.border_window))
            .bg(color(background))
            .text_color(color(colors.text_primary));

        root = crate::styling::typography(root, &self.theme);
        // The style's tracking, here and nowhere lower: the query, the rows,
        // the headers and the footer inherit it (7.41).
        root = crate::styling::tracking(root, &self.geometry());

        // One region, three possible occupants, in order of precedence: the
        // scope picker, then an extension's view, then the root list. The
        // action panel is not one of them: it floats over whichever is there.
        let body = match (self.render_picker(cx), &self.mode) {
            (Some(picker), _) => picker,
            (None, Mode::Form(state)) => match &state.form {
                FormKind::Quicklink(form) => form.clone().into_any_element(),
                FormKind::Timer(form) => form.clone().into_any_element(),
                FormKind::Snippet(form) => form.clone().into_any_element(),
            },
            (None, Mode::View(state)) => self.render_view(state, cx),
            (None, Mode::Root) => self.render_results(cx).into_any_element(),
        };
        // The region between the search field and the footer, and the one
        // part of the window that gives: it takes whatever height the window
        // has once those two have theirs, which is never exactly what was
        // asked for -- the window's border is inside it, the platform rounds
        // its size to whole pixels, and it catches up with a new height a
        // frame late. The body keeps its own height inside and is clipped to
        // the region, so a body that cannot shrink, as a list beside its
        // detail pane cannot, never pushes the footer out (I-10). The popover
        // floats over it, unclipped, so its shadow is not cut.
        let region = div()
            .relative()
            .flex_1()
            .min_h(gpui::px(0.0))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .overflow_hidden()
                    .child(body),
            )
            .children(self.render_action_panel(cx));

        root.child(self.render_search_field(cx))
            .child(region)
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
        let geometry = self.geometry();

        // A view says what its own search does. "Search for apps and
        // commands" over somebody's pull requests is the launcher talking
        // over an extension in its own window.
        // A form's fields take the input, so the search row only says what
        // the form is.
        if let Mode::Form(state) = &self.mode {
            return div()
                .flex()
                .flex_none()
                .items_center()
                .h(geometry.search_height())
                .px(geometry.gutter())
                .map(|bar| rule_below(bar, &geometry))
                .border_color(color(colors.border_subtle))
                .text_size(geometry.text_query())
                .font_weight(geometry.weight_query())
                .text_color(color(colors.text_primary))
                .children(search_icon(&geometry, colors))
                .child(state.title);
        }

        let placeholder = match &self.mode {
            // The field searches the picker's options while it is open.
            Mode::View(_) if self.picker.is_some() => FIND,
            Mode::View(state) => state.view.placeholder().unwrap_or(FIND),
            Mode::Form(_) => ROOT_PLACEHOLDER,
            Mode::Root if self.argument.is_some() => ARGUMENT_PLACEHOLDER,
            Mode::Root => ROOT_PLACEHOLDER,
        };
        self.query_field.update(cx, |field, _| {
            field.set_placeholder(placeholder.to_string())
        });

        // Each scope's name and its option in force: "From Spanish".
        let open = self.picker.as_ref().map(|picker| picker.scope);
        let pills: Vec<_> = self
            .scopes()
            .iter()
            .enumerate()
            .map(|(index, scope)| {
                let option = scope
                    .current()
                    .map(|option| option.title.clone())
                    .unwrap_or_default();
                div()
                    .id(("scope", index))
                    .flex_none()
                    .flex()
                    .gap(crate::styling::metrics::argument_pill_padding_x())
                    .ml(crate::styling::metrics::argument_pill_gap())
                    .px(crate::styling::metrics::argument_pill_padding_x())
                    .py(crate::styling::metrics::argument_pill_padding_y())
                    .rounded(crate::styling::metrics::argument_pill_radius())
                    .bg(color(colors.selection_background))
                    .border_1()
                    // The one whose options are showing is marked, so the
                    // list below reads as belonging to it.
                    .border_color(color(if open == Some(index) {
                        colors.accent
                    } else {
                        colors.selection_border
                    }))
                    .text_size(type_scale::title())
                    .cursor_pointer()
                    .on_click(
                        cx.listener(move |launcher, event: &gpui::ClickEvent, _, cx| {
                            // The mouse only; the keys are the keymap's (I-14).
                            if event.is_keyboard() {
                                return;
                            }
                            if launcher.picker.as_ref().map(|picker| picker.scope) == Some(index) {
                                launcher.close_picker(cx);
                            } else {
                                launcher.open_picker(index, cx);
                            }
                        }),
                    )
                    .child(
                        div()
                            .text_color(color(colors.text_muted))
                            .child(scope.title.clone()),
                    )
                    .child(div().text_color(color(colors.text_primary)).child(option))
            })
            .collect();

        div()
            .flex()
            .flex_none()
            .items_center()
            .h(geometry.search_height())
            .px(geometry.gutter())
            .map(|bar| rule_below(bar, &geometry))
            .border_color(color(colors.border_subtle))
            .text_size(geometry.text_query())
            .font_weight(geometry.weight_query())
            .when(!pills.is_empty(), |row| row.key_context(SCOPE_CONTEXT))
            .children(search_icon(&geometry, colors))
            // A view's levels, where the text would start, as the argument
            // is (F13): what is typed is a search within them.
            .when(!self.view_path().is_empty(), |row| {
                row.child(
                    div()
                        .flex_none()
                        .mr(crate::styling::metrics::argument_pill_gap())
                        .px(crate::styling::metrics::argument_pill_padding_x())
                        .py(crate::styling::metrics::argument_pill_padding_y())
                        .rounded(crate::styling::metrics::argument_pill_radius())
                        .bg(color(colors.selection_background))
                        .border_1()
                        .border_color(color(colors.selection_border))
                        .text_size(type_scale::title())
                        .text_color(color(colors.text_primary))
                        .child(self.view_path().join(" \u{203a} ")),
                )
            })
            // The link the field is filling in, where the text would start:
            // what you type reads as its continuation.
            .when_some(self.argument.as_ref(), |row, argument| {
                row.child(
                    div()
                        .flex_none()
                        .mr(crate::styling::metrics::argument_pill_gap())
                        .px(crate::styling::metrics::argument_pill_padding_x())
                        .py(crate::styling::metrics::argument_pill_padding_y())
                        .rounded(crate::styling::metrics::argument_pill_radius())
                        .bg(color(colors.selection_background))
                        .border_1()
                        .border_color(color(colors.selection_border))
                        .text_size(type_scale::title())
                        .text_color(color(colors.text_primary))
                        .child(argument.name.clone()),
                )
            })
            .child(self.query_field.clone())
            // A scope applies to the whole list, so it sits at the end of the
            // field rather than where the text starts: it is a setting of the
            // search, not the beginning of it. Drawn as the argument pill is,
            // because it is the same kind of thing -- what the query is for.
            .children(pills)
    }

    /// The open scope's options, in place of the list.
    ///
    /// Drawn with the action panel's rows, so it reads as the same kind of
    /// thing, but in a virtualised list capped at `PICKER_ROWS`: a scope may
    /// have thirty options and the panel sizes itself to every one.
    fn render_picker(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        use lumen_core::layout::action_panel as panel;

        let picker = self.picker.as_ref()?;
        let count = self.picker_count();
        let rows = count.clamp(1, lumen_core::layout::view::PICKER_ROWS);
        let region = div()
            .p(gpui::px(panel::PADDING))
            .h(gpui::px(
                panel::PADDING * 2.0 + panel::ROW_HEIGHT * rows as f32,
            ))
            .bg(color(self.theme.colors.surface_elevated));

        // A search that matches nothing keeps one row's height, and says so.
        if count == 0 {
            return Some(
                region
                    .child(
                        div()
                            .h(gpui::px(panel::ROW_HEIGHT))
                            .px(self.geometry().gutter())
                            .flex()
                            .items_center()
                            .text_size(type_scale::title())
                            .text_color(color(self.theme.colors.text_muted))
                            .child("No matches"),
                    )
                    .into_any_element(),
            );
        }

        Some(
            region
                .child(
                    uniform_list(
                        "scope-options",
                        count,
                        cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                            range
                                .map(|index| this.render_picker_row(index, cx))
                                .collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(picker.scroll.clone())
                    .h_full(),
                )
                .into_any_element(),
        )
    }

    fn render_picker_row(&self, index: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        use lumen_core::layout::action_panel as panel;

        let colors = &self.theme.colors;
        let highlighted = self
            .picker
            .as_ref()
            .is_some_and(|picker| picker.highlighted == index);
        let option = self.picker_option(index);
        let in_force = option.is_some_and(|(_, in_force)| in_force);
        let title = option
            .map(|(option, _)| option.title.clone())
            .unwrap_or_default();

        div()
            .id(("scope-option", index))
            .h(gpui::px(panel::ROW_HEIGHT))
            .px(self.geometry().gutter())
            .flex()
            .items_center()
            .justify_between()
            .rounded(gpui::px(panel::ROW_RADIUS))
            .text_size(type_scale::title())
            .when(highlighted, |row| {
                row.bg(color(colors.selection_background))
                    .text_color(color(colors.text_primary))
            })
            .when(!highlighted, |row| row.text_color(color(colors.text_muted)))
            .cursor_pointer()
            .on_click(cx.listener(move |launcher, _, _, cx| {
                if let Some(picker) = &mut launcher.picker {
                    picker.highlighted = index;
                }
                launcher.choose_in_picker(cx);
            }))
            .child(title)
            // The option in force, so opening the picker shows where it is.
            .when(in_force, |row| {
                row.child(
                    div()
                        .text_size(type_scale::footer())
                        .text_color(color(colors.text_muted))
                        .child("\u{2713}"),
                )
            })
            .into_any_element()
    }

    /// The height of the region the list occupies.
    fn results_height(&self) -> gpui::Pixels {
        let geometry = self.geometry();
        gpui::px(self.card_height()) + gpui::px(self.rows_height()) + geometry.list_padding() * 2.0
    }

    /// Draw the open view (F4, step 2).
    ///
    /// The launcher owns the search field and the footer; the renderer owns
    /// everything between them. That split is the one an extension will see:
    /// it supplies rows, and the chrome around them is never its to change.
    fn render_view(&self, state: &ViewState, cx: &mut gpui::App) -> gpui::AnyElement {
        // The list's own height, not the window's: with the popover open the
        // window may be taller, and the rows behind it must not stretch.
        let height = gpui::px(
            self.underlying_height() - self.metrics().search_height - self.metrics().footer_height,
        );

        // The loading state and the empty state are given the same room, the
        // whole region, as tall as a full list: the window does not jump when
        // the first render arrives, empty or not (7.48).
        if state.loading {
            return crate::view::render_loading(&self.theme, height);
        }

        match &state.view {
            lumen_core::view::View::List(list) => crate::view::render_list(
                list,
                &state.rows,
                &state.list,
                state.selected,
                &self.theme,
                height,
                &self.images,
                cx,
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

        // The card, when there is one, is the first result, drawn above the
        // list in the place of `card_rows` rows; the list holds the rest,
        // under their sections' headers.
        let card = self.card_rows();
        let count = self.rows.len();
        let entity = cx.entity();

        div()
            .py(self.geometry().list_padding())
            .h(self.results_height())
            .flex()
            .flex_col()
            .when(card > 0 && self.card_header_shown(), |results| {
                results.child(self.render_card_header())
            })
            .when(card > 0, |results| results.child(self.render_card(&entity)))
            .when(count > 0, |results| {
                results.child(
                    gpui::list(
                        self.list.clone(),
                        cx.processor(move |this, index: usize, _window, cx| {
                            this.render_root_row(index, &entity, cx)
                        }),
                    )
                    .flex_1(),
                )
            })
            .into_any_element()
    }

    fn card_header_shown(&self) -> bool {
        self.theme.style.variants.card_header == CardHeader::Shown
    }

    /// *Calculator* over the card, as any section's header is drawn and in
    /// its height (`card.header`).
    fn render_card_header(&self) -> gpui::AnyElement {
        let title = match self.theme.style.variants.section_case {
            SectionCase::Title => CARD_HEADER.to_string(),
            SectionCase::Upper => CARD_HEADER.to_uppercase(),
        };
        div()
            .flex_none()
            .child(crate::view::render_section(
                title.into(),
                None,
                &self.theme.colors,
                &self.geometry(),
            ))
            .into_any_element()
    }

    /// The calculator's answer, as the screenshot F14 was designed from
    /// draws it: what was typed, an arrow, and the result, each named
    /// beneath. `card.rows` rows tall, always (7.40). For money,
    /// the result is named by the rates' date, so a table a week old shows
    /// on the card itself.
    fn render_card(&self, entity: &Entity<Self>) -> gpui::AnyElement {
        let colors = &self.theme.colors;
        let geometry = self.geometry();
        let variants = *geometry.variants();
        let item = self.item(&self.hits()[0]);
        let selected = self.selected == 0;
        let expression = self.query.trim().to_string();
        let named = card_label(&expression, item.subtitle.as_deref());
        let chips = variants.card_labels == CardLabels::Chips;
        let divided = variants.card_divider == CardDivider::Line;

        let side = |value: gpui::AnyElement, label: String| {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .items_center()
                .gap(geometry.card_label_gap())
                .child(
                    div()
                        .max_w_full()
                        .text_size(geometry.text_card())
                        .font_weight(geometry.weight_card())
                        .text_color(color(colors.text_primary))
                        .truncate()
                        .child(value),
                )
                .child(
                    div()
                        .max_w_full()
                        .text_size(geometry.text_footer())
                        .truncate()
                        .map(|label_box| {
                            if chips {
                                label_box
                                    .px(gpui::px(lumen_core::layout::CARD_CHIP_PADDING_X))
                                    .py(gpui::px(lumen_core::layout::CARD_CHIP_PADDING_Y))
                                    .rounded(gpui::px(lumen_core::layout::CARD_CHIP_RADIUS))
                                    .bg(color(colors.border_strong))
                                    .text_color(color(colors.text_primary))
                            } else {
                                label_box.text_color(color(colors.text_muted))
                            }
                        })
                        .child(label),
                )
        };

        let expression_value = match variants.card_operators {
            CardOperators::Plain => expression.clone().into_any_element(),
            CardOperators::Dimmed => {
                let muted = HighlightStyle {
                    color: Some(color(colors.text_muted)),
                    ..Default::default()
                };
                StyledText::new(expression.clone())
                    .with_highlights(
                        operator_ranges(&expression)
                            .into_iter()
                            .map(|range| (range, muted)),
                    )
                    .into_any_element()
            }
        };

        // With the line, the arrow is what interrupts it, and is drawn to be
        // read; without, it is the quiet mark it was.
        let arrow = div()
            .flex_none()
            .text_size(geometry.text_title())
            .text_color(color(if divided {
                colors.text_primary
            } else {
                colors.text_muted
            }))
            .child("\u{2192}");
        let between = if divided {
            let rule = || {
                div()
                    .flex_1()
                    .w(gpui::px(lumen_core::layout::CARD_DIVIDER_WIDTH))
                    // Strong, not subtle: it is drawn on the card's fill,
                    // which a palette may give the subtle border's colour.
                    .bg(color(colors.border_strong))
            };
            div()
                .flex_none()
                .h_full()
                .flex()
                .flex_col()
                .items_center()
                .child(rule())
                .child(
                    div()
                        .py(gpui::px(lumen_core::layout::CARD_ARROW_GAP))
                        .child(arrow),
                )
                .child(rule())
                .into_any_element()
        } else {
            arrow.into_any_element()
        };

        let entity = entity.clone();
        div()
            .id("calculator-card")
            .h(geometry.row_height() * self.card_rows() as f32)
            .flex_none()
            .mx(geometry.row_inset())
            .px(geometry.gutter() - geometry.row_inset())
            .flex()
            .items_center()
            .gap(geometry.gutter())
            .rounded(geometry.row_radius())
            .border_1()
            .when(selected, |card| {
                card.bg(color(colors.selection_background))
                    .border_color(geometry.selection_border(colors))
            })
            // Reserved when not selected, as a row's is (I-10).
            .when(!selected, |card| {
                card.border_color(gpui::transparent_black())
            })
            .on_click(move |_event, window, cx| {
                entity.update(cx, |this, cx| {
                    this.selected = 0;
                    this.activate(&Activate, window, cx);
                });
            })
            .child(side(expression_value, "Expression".into()))
            .child(between)
            .child(side(item.title.clone().into_any_element(), named))
            .into_any_element()
    }

    /// One drawn row of the root list: a section's header, or a hit.
    fn render_root_row(
        &self,
        index: usize,
        entity: &Entity<Self>,
        cx: &mut App,
    ) -> gpui::AnyElement {
        match self.rows.get(index) {
            Some(crate::view::Row::Section(title, subtitle)) => crate::view::render_section(
                title.clone(),
                subtitle.clone(),
                &self.theme.colors,
                &self.geometry(),
            ),
            Some(crate::view::Row::Item(hit)) => self.render_row(*hit, entity, cx),
            None => div().into_any_element(),
        }
    }

    fn render_row(&self, index: usize, entity: &Entity<Self>, cx: &mut App) -> gpui::AnyElement {
        let colors = &self.theme.colors;
        let geometry = self.geometry();
        let hit = &self.hits()[index];
        let item = self.item(hit);
        let selected = index == self.selected;

        let icon = item
            .icon
            .as_ref()
            .and_then(|source| self.icons.get(source, cx));

        let entity = entity.clone();

        // The inset is padding on a wrapper, not a margin on the row: gpui's
        // `list` lays an item out at its full width from its origin and
        // ignores the item's own margins, so a margin left the selection
        // touching the window's edges (F15).
        div()
            .px(geometry.row_inset())
            .child(
                div()
                    .id(index)
                    .h(geometry.row_height())
                    // Full width, and this is load-bearing rather than tidy. Without it
                    // the row is only as wide as its content, `flex_1` below has no
                    // free space to claim, and the kind label ends up butted against
                    // the subtitle -- so it sits at a different horizontal position on
                    // every row, which is precisely the layout shift I-10 forbids.
                    .w_full()
                    .px(geometry.gutter() - geometry.row_inset())
                    .flex()
                    .items_center()
                    .gap(geometry.icon_gap())
                    .rounded(geometry.row_radius())
                    .when(selected, |row| {
                        row.bg(color(colors.selection_background))
                            .border_1()
                            .border_color(geometry.selection_border(colors))
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
                            .size(geometry.icon_size())
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .map(|slot| match icon {
                                Some(image) => slot.child(img(image).size_full()),
                                None if item.icon.is_none() => slot.child(
                                    svg()
                                        .path(crate::icons::glyph(item.kind))
                                        .size(geometry.icon_size())
                                        .text_color(color(colors.icon)),
                                ),
                                None => slot,
                            }),
                    )
                    .child(crate::view::row_text(
                        div()
                            .text_size(geometry.text_title())
                            .font_weight(geometry.weight_title())
                            .text_color(color(colors.text_primary))
                            .child(highlighted(&item.title, &hit.highlights, colors))
                            .into_any_element(),
                        item.subtitle.clone().map(Into::into),
                        colors,
                        &geometry,
                    ))
                    .child(
                        div()
                            .flex_none()
                            .pl(geometry.icon_gap())
                            .text_size(geometry.text_accessory())
                            .text_color(color(colors.text_muted))
                            .child(item.kind_label()),
                    ),
            )
            .into_any_element()
    }

    fn render_footer(&self) -> impl IntoElement {
        let colors = &self.theme.colors;
        let geometry = self.geometry();
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
            Mode::Form(FormState {
                form: FormKind::Snippet(_),
                ..
            }) => "Save Snippet".to_string(),
            Mode::Form(_) => "Save".to_string(),
        };
        // Enter makes a new line in the snippet editor.
        let save_keys: &[&str] = match &self.mode {
            Mode::Form(FormState {
                form: FormKind::Snippet(_),
                ..
            }) => &["Ctrl", "\u{23ce}"],
            _ => &["\u{23ce}"],
        };

        let count = match &self.mode {
            Mode::Root => self.hits().len(),
            Mode::View(_) => self.view_items().len(),
            Mode::Form(_) => 0,
        };
        let scopes = self.scopes();
        let picking = self
            .picker
            .as_ref()
            .and_then(|picker| scopes.get(picker.scope).map(|scope| (picker.scope, scope)));
        let status = match (&self.mode, count) {
            (Mode::Form(_), _) => "Next field  Tab".to_string(),
            // What is counting, while nothing is typed: the launcher opens
            // on it. Once a search starts, how many it found is the news (F8).
            (Mode::Root, _)
                if self.argument.is_none()
                    && self.query.trim().is_empty()
                    && self.timers_line().is_some() =>
            {
                self.timers_line().unwrap_or_default()
            }
            // Where Tab goes next, when there is somewhere.
            (Mode::View(_), _) if picking.is_some() && scopes.len() > 1 => {
                let (index, _) = picking.expect("checked");
                let next = &scopes[(index + 1) % scopes.len()];
                format!("{}  Tab", next.title)
            }
            (Mode::View(_), _) if picking.is_some() => String::new(),
            // What Tab does is not visible from the pills alone.
            (Mode::View(_), _) if !scopes.is_empty() => {
                let titles: Vec<&str> = scopes.iter().map(|scope| scope.title.as_str()).collect();
                format!("{}  Tab", titles.join(" / "))
            }
            _ if self.theme.style.variants.footer_count == lumen_core::style::Shown::Hidden => {
                String::new()
            }
            (_, 1) => "1 result".to_string(),
            (_, count) => format!("{count} results"),
        };
        let verb = if picking.is_some() {
            "Choose".to_string()
        } else {
            verb
        };

        // Fixed: the window grows for the popover a frame after the layout is
        // asked for it, and in that frame a footer that can shrink does (I-10).
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .h(geometry.footer_height())
            .px(geometry.gutter())
            .map(|footer| rule_above(footer, &geometry))
            .border_color(color(colors.border_subtle))
            // Its own surface when the palette gives it one, rounded with
            // the window's bottom corners. The window's radius rather than
            // that less its border: a pixel's difference in the curve does
            // not show, and a literal for it would (I-5).
            .bg(color(colors.surface_footer))
            .rounded_b(geometry.window_radius())
            .text_size(geometry.text_footer())
            .text_color(color(colors.text_muted))
            .when(
                geometry.variants().footer_icon == lumen_core::style::FooterIcon::Logo,
                |footer| {
                    footer.child(
                        svg()
                            .path(crate::icons::LOGO)
                            .flex_none()
                            .size(geometry.footer_icon())
                            .mr(geometry.keycap_gap() * 2.0)
                            .text_color(color(colors.text_muted)),
                    )
                },
            )
            // Truncated rather than pushing the keys off the edge: a long
            // timer name must not move them (I-10).
            .child(div().flex_1().min_w_0().truncate().child(status))
            .child({
                let mut hints: Vec<(String, &[&str])> = vec![(verb, save_keys)];
                // Advertised only when there is a second action to reach. A
                // hint for a panel that would open empty teaches the user
                // that the hint lies.
                if entries.len() > 1 && picking.is_none() {
                    hints.push(("Actions".into(), &["Ctrl", "K"]));
                }
                let leave = match self.mode {
                    Mode::Root if self.argument.is_none() => "Dismiss",
                    Mode::Root | Mode::View(_) | Mode::Form(_) => "Back",
                };
                if geometry.variants().footer_escape == lumen_core::style::Shown::Shown {
                    hints.push((leave.into(), &["Esc"]));
                }

                div()
                    .flex()
                    .items_center()
                    .gap(geometry.icon_gap())
                    .children(hints.into_iter().enumerate().map(|(index, (label, keys))| {
                        render_hint(index, label, keys, colors, &geometry)
                    }))
            })
    }

    /// The height of the region between the search field and the footer: all
    /// the room the window has for the list and whatever floats over it.
    fn region_height(&self) -> f32 {
        self.content_height() - self.metrics().search_height - self.metrics().footer_height
    }

    /// The action panel, when it is open: a popover floating over the list,
    /// anchored where the style puts it -- the bottom-right just above the
    /// footer unless it says otherwise (7.45, 7.47).
    ///
    /// Drawn inside the launcher's own window rather than given one of its own:
    /// unlike the shortcut overlay it is an ordinary part of the list's
    /// interface, it wants the focus the launcher already has, and it would
    /// gain nothing from being a separate surface but the cost of being one.
    ///
    /// An overlay was the first design and it failed for two reasons, both met
    /// here. The window is sized to its content, so there was no room for a
    /// floating element: `Config::panel_height` now grows it when the list is
    /// too short to stand the popover in. And an absolutely positioned element
    /// with only `right` and `bottom` gives taffy nothing to derive a height
    /// from, so it laid out at zero: the popover is given its own, and the
    /// backdrop behind it the region's.
    ///
    /// What is returned fills the region. Its transparent backdrop takes the
    /// mouse so that a click on the list behind closes the popover instead of
    /// running the row under it, and so that nothing behind reacts to the
    /// pointer while it is open.
    fn render_action_panel(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        use lumen_core::layout::action_panel as panel;

        let selected = self.action_panel?;
        let entries = self.panel_entries();
        if entries.is_empty() {
            return None;
        }

        let colors = &self.theme.colors;
        let geometry = self.geometry();
        let entity = cx.entity();
        // Sized from every row, not the ones the search leaves: typing in the
        // panel's field never moves the window (I-10).
        let groups: Vec<_> = entries.iter().map(|entry| entry.group).collect();
        let metrics = self.metrics();
        let height = panel::height(&groups, metrics.actions_rows());
        let width = metrics.actions_width(self.width());
        let [left, top] = geometry.variants().actions_anchor.origin(
            [self.width(), self.region_height()],
            [width, height],
            [metrics.actions_offset_x, metrics.actions_offset_y],
        );

        let shown = self.panel_rows();
        let shown_groups: Vec<_> = shown.iter().map(|row| groups[*row]).collect();
        let mut entries: Vec<Option<PanelEntry>> = entries.into_iter().map(Some).collect();
        let mut rows = Vec::with_capacity(shown.len() + 1);
        // A search that matches nothing keeps a row, and says so, as the
        // picker does.
        if shown.is_empty() {
            rows.push(
                div()
                    .flex_none()
                    .h(gpui::px(panel::ROW_HEIGHT))
                    .px(geometry.gutter())
                    .flex()
                    .items_center()
                    .text_size(type_scale::title())
                    .text_color(color(colors.text_muted))
                    .child("No matching actions")
                    .into_any_element(),
            );
        }
        for (index, (row, rule)) in shown
            .into_iter()
            .zip(panel::breaks(&shown_groups))
            .enumerate()
        {
            let Some(entry) = entries[row].take() else {
                continue;
            };
            if rule {
                rows.push(
                    div()
                        .flex_none()
                        .h(gpui::px(panel::SEPARATOR_HEIGHT))
                        .my(gpui::px(panel::SEPARATOR_MARGIN))
                        .bg(color(colors.border_subtle))
                        .into_any_element(),
                );
            }
            let entity = entity.clone();
            rows.push(
                div()
                    .id(("action", index))
                    .flex_none()
                    .h(gpui::px(panel::ROW_HEIGHT))
                    .px(geometry.gutter())
                    .flex()
                    .items_center()
                    .gap(gpui::px(panel::ICON_GAP))
                    .rounded(gpui::px(panel::ROW_RADIUS))
                    .text_size(type_scale::title())
                    .text_color(color(colors.text_primary))
                    .when(index == selected, |row| {
                        row.bg(color(colors.selection_background))
                    })
                    .cursor_pointer()
                    .on_click(move |_event, window, cx| {
                        entity.update(cx, |this, cx| {
                            this.action_panel = Some(index);
                            this.activate(&Activate, window, cx);
                        });
                    })
                    // The square is there whether or not the action has a
                    // glyph, so the titles line up (I-10).
                    .child(crate::view::render_icon(
                        entry.glyph.as_ref(),
                        gpui::px(panel::ICON_SIZE),
                        colors,
                        &geometry,
                        &self.images,
                        cx,
                    ))
                    .child(div().flex_1().min_w_0().truncate().child(entry.title))
                    // The keys beside it: the first action says what Enter
                    // already does, which is what stops the panel looking like
                    // a list of things Enter cannot reach. A root row's come
                    // from the keymap; a view's are what the extension asked
                    // to have shown.
                    .when(!entry.shortcut.is_empty(), |row| {
                        row.child(render_keys(&entry.shortcut, colors, &geometry))
                    })
                    .into_any_element(),
            );
        }

        let popover = div()
            .absolute()
            .left(gpui::px(left))
            .top(gpui::px(top))
            .w(gpui::px(width))
            .h(gpui::px(height))
            // Over the backdrop, which would otherwise take a click on a row.
            .occlude()
            .flex()
            .flex_col()
            .rounded(gpui::px(panel::RADIUS))
            // One pixel, which is `panel::BORDER`: GPUI names the width, not
            // takes it.
            .border_1()
            .border_color(color(colors.border_strong))
            // Opaque whatever the theme's alpha: it floats over text, and the
            // list must not show through it.
            .bg(color(colors.surface_elevated.with_alpha(255)))
            .shadow_md()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .p(gpui::px(panel::PADDING))
                    .child(
                        div()
                            .flex_none()
                            .h(gpui::px(panel::HEADER_HEIGHT))
                            .px(geometry.gutter())
                            .flex()
                            .items_center()
                            .truncate()
                            .text_size(type_scale::footer())
                            .text_color(color(colors.text_muted))
                            .child(self.panel_header()),
                    )
                    // As tall as every row needs, whatever the search leaves:
                    // the rows left sit at its top, and nothing jumps.
                    .child(
                        div()
                            .id("action-rows")
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .overflow_y_scroll()
                            .track_scroll(&self.panel_scroll)
                            .children(rows),
                    ),
            )
            // The panel's own field, the width of the popover under a rule,
            // its text where the rows' icons start.
            .child(
                div()
                    .flex_none()
                    .h(gpui::px(panel::SEARCH_HEIGHT))
                    .px(gpui::px(panel::PADDING) + geometry.gutter())
                    .flex()
                    .items_center()
                    .map(|field| rule_above(field, &geometry))
                    .border_color(color(colors.border_subtle))
                    .text_size(type_scale::title())
                    .child(self.action_field.clone()),
            );

        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.close_action_panel(cx);
                        cx.notify();
                    }),
                )
                .child(popover)
                .into_any_element(),
        )
    }
}

/// The keys beside an action, drawn the way the footer draws its own: as text
/// or as keycaps, whichever the style asks for (F15).
fn render_keys(
    keys: &[String],
    colors: &lumen_core::theme::Colors,
    geometry: &Geometry,
) -> gpui::AnyElement {
    use lumen_core::style::FooterHints;

    match geometry.variants().footer_hints {
        FooterHints::Text => div()
            .text_size(type_scale::footer())
            .text_color(color(colors.text_muted))
            .child(keys.join("+"))
            .into_any_element(),
        FooterHints::Keycaps => div()
            .flex()
            .items_center()
            .gap(geometry.keycap_gap())
            .children(keys.iter().map(|key| render_keycap(key, colors, geometry)))
            .into_any_element(),
    }
}

/// One key in a cap of its own: filled or outlined, as the style draws it.
fn render_keycap(key: &str, colors: &lumen_core::theme::Colors, geometry: &Geometry) -> gpui::Div {
    use lumen_core::style::Keycap;

    // A key is one or two letters, which tracking does not suit.
    let cap = crate::styling::untracked(div())
        .h(geometry.keycap_height())
        .min_w(geometry.keycap_height())
        .px(geometry.keycap_padding())
        .flex()
        .items_center()
        .justify_center()
        .rounded(geometry.keycap_radius())
        .border_1();
    match geometry.variants().keycap {
        Keycap::Filled => cap
            .bg(color(colors.selection_background))
            .border_color(color(colors.border_subtle))
            .text_color(color(colors.text_primary)),
        // The footer shows through, so the key reads as quieter than the label
        // beside it, as Raycast draws it.
        Keycap::Outline => cap
            .border_color(color(colors.border_strong))
            .text_color(color(colors.text_muted)),
    }
    .child(key.to_string())
}

/// The search field's icon, when the style asks for one (F15): the row
/// icon's size and gap, so the query starts where the rows' titles do.
fn search_icon(
    geometry: &Geometry,
    colors: &lumen_core::theme::Colors,
) -> Option<gpui::AnyElement> {
    use lumen_core::style::SearchIcon;

    let path = match geometry.variants().search_icon {
        SearchIcon::None => return None,
        SearchIcon::Search => crate::icons::SEARCH,
        SearchIcon::Logo => crate::icons::LOGO,
    };
    Some(
        svg()
            .path(path)
            .flex_none()
            .size(geometry.icon_size())
            .mr(geometry.icon_gap())
            .text_color(color(colors.icon))
            .into_any_element(),
    )
}

/// One of the footer's hints: what a key does, and the key.
///
/// As text, `Actions  Ctrl+K`; as keycaps, the label and each key in a cap of
/// its own, with a rule before every hint but the first (F15). Either way the
/// keys are the keymap's, written out here only for show (I-14).
fn render_hint(
    index: usize,
    label: String,
    keys: &[&str],
    colors: &lumen_core::theme::Colors,
    geometry: &Geometry,
) -> gpui::AnyElement {
    use lumen_core::style::FooterHints;

    match geometry.variants().footer_hints {
        FooterHints::Text => div()
            .child(format!("{label}  {}", keys.join("+")))
            .into_any_element(),
        FooterHints::Keycaps => div()
            .flex()
            .items_center()
            .gap(geometry.keycap_gap())
            // The rule takes the hint's own gap on its far side, so the
            // spacing either side of it is the footer's.
            .when(index > 0, |hint| {
                hint.child(
                    div()
                        .h(geometry.keycap_height() * 0.6)
                        .mr(geometry.icon_gap() - geometry.keycap_gap())
                        .border_l_1()
                        .border_color(color(colors.border_strong)),
                )
            })
            // What Enter does reads first; the rest is there if wanted.
            .child(
                div()
                    .mr(geometry.keycap_gap())
                    .text_color(color(if index == 0 {
                        colors.text_primary
                    } else {
                        colors.text_muted
                    }))
                    .child(label),
            )
            .children(keys.iter().map(|key| render_keycap(key, colors, geometry)))
            .into_any_element(),
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

/// The header over the calculator's card when the style draws one.
const CARD_HEADER: &str = "Calculator";

/// The characters the card's expression draws as operators: the ones the
/// calculator reads, in the forms people type them.
const OPERATORS: [char; 9] = ['+', '-', '\u{2212}', '*', '\u{d7}', '/', '\u{f7}', '^', '%'];

/// Where the operators are in `expression`, as byte ranges: one per
/// operator, so a styled run never touches an operand.
fn operator_ranges(expression: &str) -> Vec<std::ops::Range<usize>> {
    expression
        .char_indices()
        .filter(|(_, c)| OPERATORS.contains(c))
        .map(|(start, c)| start..start + c.len_utf8())
        .collect()
}

/// The room the card's place takes: its rows and, when the style shows one,
/// the header over them; none when the card is not drawn (I-10).
fn card_extent(rows: usize, metrics: &Metrics, variants: &Variants) -> f32 {
    if rows == 0 {
        return 0.0;
    }
    let header = match variants.card_header {
        CardHeader::Hidden => 0.0,
        CardHeader::Shown => metrics.section_height,
    };
    rows as f32 * metrics.row_height + header
}

/// What names the result in the calculator's card: *Result*, or what the
/// answer's subtitle says after what was typed -- the rates' date, for money
/// (F14).
fn card_label(expression: &str, subtitle: Option<&str>) -> String {
    let said = subtitle
        .and_then(|subtitle| subtitle.strip_prefix(expression))
        .and_then(|rest| rest.strip_prefix(" \u{b7} "));
    let mut letters = said.unwrap_or_default().chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => "Result".into(),
    }
}

/// What the daemon keeps when `action` runs on `item`: a calculation copied
/// is one used (F14) -- what was typed, and what was copied.
fn calculated(item: &Item, action: &Action) -> Option<LauncherEvent> {
    match (&item.kind, action) {
        (
            lumen_core::item::Kind::Calculation,
            Action::CopyToClipboard(result) | Action::CopyAnswer(result),
        ) => item
            .subtitle
            .clone()
            .map(|expression| LauncherEvent::Calculated {
                expression,
                result: result.clone(),
            }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {

    use super::{
        LauncherEvent, calculated, card_extent, card_label, opening_selection, operator_ranges,
        panel_matches, refiltered_selection, root_panel_entries, second_action, visible_panel_rows,
    };
    use lumen_core::favorites::Change as FavoriteChange;
    use lumen_core::item::{ActionGroup, Builtin, Kind};
    use lumen_core::providers::Provider;
    use lumen_core::style::{CardHeader, Metrics, Variants};
    use lumen_core::{Action, Item};

    #[test]
    fn the_card_names_its_result_or_its_rates() {
        assert_eq!(card_label("2+2", Some("2+2")), "Result");
        assert_eq!(
            card_label(
                "10 usd in cad",
                Some("10 usd in cad \u{b7} rates of 28 Sep 2026 \u{b7} updating\u{2026}")
            ),
            "Rates of 28 Sep 2026 \u{b7} updating\u{2026}"
        );
        assert_eq!(card_label("2+2", None), "Result");
    }

    fn operators_in(expression: &str) -> Vec<&str> {
        operator_ranges(expression)
            .into_iter()
            .map(|range| &expression[range])
            .collect()
    }

    /// Each operator is its own run, so a digit between two is never swept
    /// into the dimmed text; the ranges are byte ranges, safe to slice on.
    #[test]
    fn operators_are_found_apart_from_their_operands() {
        assert_eq!(operators_in("2+2"), ["+"]);
        assert_eq!(operators_in("12 * (3 - 4) / 5"), ["*", "-", "/"]);
        assert_eq!(operators_in("2^10 % 3"), ["^", "%"]);
        // The ones the lexer reads besides the ASCII: the multiplication and
        // division signs, and the true minus.
        assert_eq!(
            operators_in("6\u{d7}7 \u{f7} 2 \u{2212} 1"),
            ["\u{d7}", "\u{f7}", "\u{2212}"]
        );
    }

    #[test]
    fn the_cards_place_counts_its_header() {
        let metrics = Metrics::BASE;
        let hidden = Variants::default();
        let shown = Variants {
            card_header: CardHeader::Shown,
            ..Variants::default()
        };
        assert_eq!(card_extent(0, &metrics, &shown), 0.0);
        assert_eq!(card_extent(2, &metrics, &hidden), 2.0 * metrics.row_height);
        assert_eq!(
            card_extent(3, &metrics, &shown),
            3.0 * metrics.row_height + metrics.section_height
        );
    }

    #[test]
    fn words_and_numbers_are_not_operators() {
        assert!(operator_ranges("sqrt 16").is_empty());
        assert!(operator_ranges("3.14").is_empty());
        assert!(operator_ranges("").is_empty());
        // Multi-byte text around an operator does not shift the ranges.
        assert_eq!(operators_in("\u{3c0} + \u{3c0}"), ["+"]);
    }

    /// The calculator's own row, copied, is kept; anything else copied is not.
    #[test]
    fn a_calculation_copied_is_kept() {
        let answer = lumen_core::providers::CalculatorProvider.answer("25% off 399");
        let row = &answer[0];
        assert_eq!(
            calculated(row, &row.primary()),
            Some(LauncherEvent::Calculated {
                expression: "25% off 399".into(),
                result: "299.25".into(),
            })
        );

        // Copying what was typed is not using an answer.
        assert_eq!(calculated(row, &row.actions[1]), None);

        let other = item_with(vec![Action::CopyToClipboard("x".into())]);
        assert_eq!(calculated(&other, &other.primary()), None);
    }

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

    /// The popover opens on the *first* row, as Raycast's does: it is the
    /// action Enter would run, shown highlighted with its key beside it.
    #[test]
    fn the_popover_opens_on_the_first_action() {
        let item = item_with(vec![
            Action::Builtin(Builtin::OpenSettings),
            Action::CopyToClipboard("x".into()),
        ]);

        let opened = opening_selection(item.actions.len());
        assert_eq!(opened, Some(0));
    }

    #[test]
    fn a_single_action_still_opens_somewhere_valid() {
        assert_eq!(opening_selection(1), Some(0));
    }

    #[test]
    fn an_item_with_no_actions_does_not_open_a_panel() {
        assert_eq!(opening_selection(0), None);
    }

    /// The panel is a different height from the list, so every path that
    /// closes it owes the window a resize. Standing up a `Launcher` needs a
    /// platform and a GPUI test app, so this guards the rule at the source: the
    /// panel is only cleared in a function that resizes (or re-reads the height)
    /// afterwards. A new site has to be added here, which is the moment to
    /// decide how it restores the height -- Escape once did not (it left the
    /// list rendering into the panel's window).
    #[test]
    fn only_functions_that_resize_clear_the_action_panel() {
        // Each of these captures the height before it clears the panel and
        // resizes after, or hands the height to the daemon (`on_shown`).
        const RESIZES: &[&str] = &[
            "close_action_panel",
            "on_shown",
            "rerank",
            "place_files",
            "open_view",
            "begin_view",
            "leave_view",
            "open_picker",
            "open_form",
            "open_timer_form",
            "open_snippet_form",
        ];

        // Normalised: a checkout with `core.autocrlf` has CRLF line ends.
        let source = include_str!("launcher.rs").replace("\r\n", "\n");
        let code = source.split("\n#[cfg(test)]\n").next().unwrap_or(&source);

        let mut current = "";
        let mut offenders = Vec::new();
        for (number, line) in code.lines().enumerate() {
            if let Some(at) = line.find("fn ")
                && line[..at]
                    .trim_start()
                    .chars()
                    .all(|c| c.is_alphabetic() || c == '(' || c == ')' || c.is_whitespace())
                && let Some(name) = line[at + 3..].split(['(', '<']).next()
            {
                current = name;
            }
            let clears =
                line.contains("action_panel = None") || line.contains("action_panel.take()");
            if clears && !line.trim_start().starts_with("//") && !RESIZES.contains(&current) {
                offenders.push(format!("line {} in `{current}`", number + 1));
            }
        }

        assert!(
            offenders.is_empty(),
            "these clear the action panel without a resize: {offenders:?}"
        );
    }

    /// The search field and the footer never give, and the region between
    /// them is the one element that does. A body that cannot shrink -- a list
    /// beside its detail pane -- once pushed the footer out of the window,
    /// because the region was not flexible and the window is never exactly the
    /// height asked for (its border is inside it).
    #[test]
    fn the_region_between_the_chrome_is_the_only_part_that_gives() {
        // Normalised: a checkout with `core.autocrlf` has CRLF line ends.
        let source = include_str!("launcher.rs").replace("\r\n", "\n");
        let code = source.split("\n#[cfg(test)]\n").next().unwrap_or(&source);
        let body = |name: &str| {
            let start = code
                .find(&format!("fn {name}("))
                .unwrap_or_else(|| panic!("`{name}` is gone"));
            let rest = &code[start..];
            let end = rest[3..]
                .find("\n    fn ")
                .map_or(rest.len(), |end| end + 3);
            &rest[..end]
        };

        for chrome in ["render_search_field", "render_footer"] {
            assert!(
                body(chrome).contains(".flex_none()"),
                "`{chrome}` must not shrink"
            );
        }
        let render = body("render");
        let region = &render[render.find("let region = div()").expect("the region")..];
        let region = &region[..region.find(';').expect("one statement")];
        assert!(region.contains(".flex_1()") && region.contains(".min_h("));
        assert!(
            !region.contains(".h("),
            "the region takes the window's height, not one of its own"
        );
    }

    fn firefox_actions() -> Vec<Action> {
        use lumen_platform::{AppEntry, LaunchTarget};

        let mut actions = Item::from_app(AppEntry {
            id: "app:x".into(),
            name: "Firefox".into(),
            detail: None,
            target: LaunchTarget::ShellExecute("x".into()),
            icon: None,
            keywords: Vec::new(),
        })
        .actions;
        actions.push(Action::Favorite(FavoriteChange::Add));
        actions
    }

    /// What the panel shows for an application, row by row: the title, the
    /// keys read from the bindings, the group that decides where the rules go
    /// and the glyph.
    #[test]
    fn an_applications_panel_rows_carry_their_keys_groups_and_glyphs() {
        let keymap = gpui::Keymap::new(crate::keymap::launcher_bindings());
        let shortcuts = crate::keymap::Shortcuts::read(&keymap);

        let rows = root_panel_entries(firefox_actions(), &shortcuts);

        let titles: Vec<_> = rows.iter().map(|row| row.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Open Application",
                "New Application Window",
                "Copy",
                "Add to Favorites"
            ]
        );
        assert_eq!(rows[0].shortcut, ["\u{21b5}"]);
        assert_eq!(rows[1].shortcut, ["Ctrl", "\u{21b5}"]);
        // No key of its own: Enter and Ctrl+Enter are spoken for.
        assert!(rows[2].shortcut.is_empty());
        assert_eq!(rows[3].shortcut, ["Ctrl", "Shift", "F"]);

        let groups: Vec<_> = rows.iter().map(|row| row.group).collect();
        assert_eq!(
            groups,
            [
                ActionGroup::Primary,
                ActionGroup::Alternate,
                ActionGroup::Copy,
                ActionGroup::Favorite
            ]
        );
        assert_eq!(
            rows[0].glyph,
            Some(lumen_core::view::Icon::Glyph {
                name: "open".into()
            })
        );
    }

    /// A favourite in second place shows its own chord, not Ctrl+Enter: a
    /// command's second action *is* the favourite, and the chord that names
    /// it is the one the user can also press from the list.
    #[test]
    fn a_favourite_in_second_place_shows_its_own_chord() {
        let keymap = gpui::Keymap::new(crate::keymap::launcher_bindings());
        let shortcuts = crate::keymap::Shortcuts::read(&keymap);

        let rows = root_panel_entries(
            vec![
                Action::Builtin(Builtin::OpenSettings),
                Action::Favorite(FavoriteChange::Add),
            ],
            &shortcuts,
        );

        assert_eq!(rows[1].shortcut, ["Ctrl", "Shift", "F"]);
    }

    /// Ctrl+Enter runs the second action, and does nothing when there is
    /// none: a row with one action has no second way of doing it.
    #[test]
    fn the_secondary_action_is_the_second_one_when_there_is_one() {
        assert_eq!(
            second_action(firefox_actions()),
            Some(Action::LaunchNew(
                lumen_platform::LaunchTarget::ShellExecute("x".into())
            ))
        );
        assert_eq!(
            second_action(vec![Action::Builtin(Builtin::OpenSettings)]),
            None
        );
        assert_eq!(second_action(Vec::new()), None);
    }

    fn match_actions(titles: &[&str], query: &str) -> Option<Vec<usize>> {
        let mut root = lumen_core::RootSearch::new(
            lumen_core::Providers::builtin(),
            lumen_core::Frecency::default(),
        );
        panel_matches(titles.iter().copied(), query, |query, candidates| {
            root.filter(query, candidates)
        })
    }

    /// Nothing typed keeps every row; a search keeps the matches in the
    /// panel's order, not the matcher's, with the root list's matcher.
    #[test]
    fn the_panels_search_keeps_its_matches_in_order() {
        let titles = [
            "Open Application",
            "Copy Path",
            "Copy Name",
            "Add to Favorites",
        ];

        assert_eq!(match_actions(&titles, ""), None);
        assert_eq!(match_actions(&titles, "  "), None);
        assert_eq!(match_actions(&titles, "name"), Some(vec![2]));
        assert_eq!(match_actions(&titles, "copy"), Some(vec![1, 2]));
        assert_eq!(match_actions(&titles, "zzqx"), Some(Vec::new()));
    }

    /// The rules are drawn from the rows that are left: one only between two
    /// shown rows of different groups.
    #[test]
    fn the_rules_follow_the_rows_a_search_leaves() {
        use lumen_core::layout::action_panel::breaks;

        let groups = [
            ActionGroup::Primary,
            ActionGroup::Alternate,
            ActionGroup::Copy,
            ActionGroup::Copy,
            ActionGroup::Favorite,
        ];
        let shown = |matches: Option<&[usize]>| -> Vec<bool> {
            let rows = visible_panel_rows(groups.len(), matches);
            breaks(&rows.iter().map(|row| groups[*row]).collect::<Vec<_>>())
        };

        assert_eq!(visible_panel_rows(3, None), [0, 1, 2]);
        assert_eq!(visible_panel_rows(3, Some(&[0, 2, 7])), [0, 2]);
        assert_eq!(shown(None), [false, true, true, false, true]);
        assert_eq!(shown(Some(&[2, 3])), [false, false]);
        assert_eq!(shown(Some(&[0, 3])), [false, true]);
        assert!(shown(Some(&[])).is_empty());
    }

    /// Every change of the search puts the highlight back on the first row
    /// left, and never opens a closed panel.
    #[test]
    fn a_new_search_highlights_the_first_match() {
        assert_eq!(refiltered_selection(Some(3)), Some(0));
        assert_eq!(refiltered_selection(Some(0)), Some(0));
        assert_eq!(refiltered_selection(None), None);
    }

    /// The popover is sized from every row, so a search that leaves fewer
    /// does not move the window (I-10): what is drawn while filtering fits in
    /// the room the unfiltered rows took.
    #[test]
    fn the_popover_keeps_its_height_while_its_rows_are_searched() {
        use lumen_core::layout::action_panel::height;

        let groups = [
            ActionGroup::Primary,
            ActionGroup::Alternate,
            ActionGroup::Copy,
            ActionGroup::Favorite,
        ];
        let cap = lumen_core::layout::action_panel::MAX_ROWS;
        let every = height(&groups, cap);
        let rows = visible_panel_rows(groups.len(), Some(&[1]));
        let left: Vec<_> = rows.iter().map(|row| groups[*row]).collect();

        assert!(height(&left, cap) < every);
        assert!(height(&[], cap) < every);
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
        assert_eq!(item.primary().verb(), "Open Application");
        assert_eq!(
            item.actions[1],
            Action::LaunchNew(LaunchTarget::ShellExecute("x".into()))
        );
        assert_eq!(item.actions[2], Action::CopyToClipboard("Firefox".into()));
    }
}
