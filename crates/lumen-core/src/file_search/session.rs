//! The *Search Files* view, as decisions without a window (F16).
//!
//! *Clipboard History*'s shape (F12): a list beside a pane, a *Type* scope,
//! actions on the selected row. The difference is size. The clipboard's
//! entries fit in the session, so it filters them itself; the index is
//! hundreds of thousands of names, so the session filters by asking: each
//! keystroke is an [`Effect::Search`] the host answers off the UI thread,
//! and an answer for a query since replaced is dropped by its generation.
//! The list is [`Filtering::Extension`] either way: the host never filters
//! it again.
//!
//! An answer can come in two phases (step c3b). The first is what the hot
//! layer and the disk's word index find, in milliseconds, and is drawn at
//! once. When the index is larger than its hot layer, a second follows:
//! the matches inside a word the first could not see, appended below it
//! ([`Effect::SearchInside`]), with *Searching...* said until it lands.
//! Once the names are in, a query of three letters or more asks Windows
//! Search for the files whose contents hold it ([`Effect::SearchContents`],
//! slice 6b), appended below the same way; their row is not marked, the
//! preview highlights the query instead ([`super::highlight`]).
//!
//! A query that is wholly a path (step 6d, [`browse`]) asks neither phase:
//! its folder is listed from the disk ([`Effect::Browse`]), answered
//! through [`Session::found`] by the same generation, in the *Type* filter.
//!
//! The pane is the same: reading a file's first bytes and its times is
//! disk work, so a row's detail is asked for ([`Effect::Describe`]) when the
//! selection lands on it, and until it arrives the row shows what is known
//! without the disk.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use lumen_platform::Drive;

use super::browse::{self, Typed};
use super::category::{CATEGORIES, Category};
use super::{Found, drives, preview};
use crate::view::{
    Action, EmptyState, Event, Filtering, Icon, Item, ItemDetail, List, Scope, ScopeOption,
    Section, View,
};

/// The id of the type scope.
pub const TYPE_SCOPE: &str = "type";
/// The option that shows every entry. Empty, because a category never is.
const ALL_TYPES: &str = "";
/// What `Enter` does, and the rest of the action panel.
pub const OPEN: &str = "open";
pub const SHOW_IN_FOLDER: &str = "show-in-folder";
pub const COPY_PATH: &str = "copy-path";
pub const OPEN_WITH: &str = "open-with";
pub const SAVE_AS_QUICKLINK: &str = "save-as-quicklink";
pub const EXCLUDE: &str = "exclude";
pub const REBUILD: &str = "rebuild";
/// On a drive's offer, and on the hint under an empty result (slice 5).
pub const CHOOSE_FOLDERS: &str = "choose-folders";
pub const NOT_NOW: &str = "not-now";
/// On the whole-drive warning.
pub const ADD_ANYWAY: &str = "add-anyway";
pub const CANCEL: &str = "cancel";
/// The most drives offered at once.
pub const MAX_OFFERS: usize = 2;

/// What the daemon is asked to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Draw this.
    Render(View),
    /// Ask the index for `query` in `filter`, off the UI thread, and hand
    /// the answer to [`Session::found`] with `generation`.
    Search {
        generation: u64,
        query: String,
        filter: Option<Category>,
    },
    /// List the folder `typed` names, of `filter`'s category, off the UI
    /// thread, and hand the answer to [`Session::found`] with `generation`.
    Browse {
        generation: u64,
        typed: Typed,
        filter: Option<Category>,
    },
    /// Ask the index for the second phase of the search of `generation`
    /// -- at most `limit` matches inside a word the first could not find
    /// ([`super::search_inside`]) -- off the UI thread, and hand the answer
    /// to [`Session::found_inside`].
    SearchInside {
        generation: u64,
        query: String,
        filter: Option<Category>,
        limit: usize,
    },
    /// Ask Windows Search for the files whose contents hold `words`
    /// (slice 6b), in the file-search scopes, off the UI thread, and hand
    /// the answer to [`Session::found_contents`] with `generation`. Asked
    /// once the names have wholly answered, from three letters.
    SearchContents {
        generation: u64,
        words: String,
        filter: Option<Category>,
    },
    /// Build the pane for this entry off the UI thread
    /// ([`preview::describe`]) and hand it to [`Session::described`].
    /// A text preview opens at the first of `words`, the query's.
    Describe(Found, Vec<String>),
    /// Open it in its application, or a folder in the file manager, and
    /// count it as used.
    Open(Found),
    /// Show it selected in its folder.
    ShowInFolder(PathBuf),
    /// Put this path on the clipboard.
    CopyPath(String),
    /// Ask the system which application should open it.
    OpenWith(PathBuf),
    /// Show the quicklink form filled in with this entry: its name, and
    /// its path as the link.
    SaveAsQuicklink { name: String, link: String },
    /// Leave it out of the index ([`super::exclude::exclude`]), then tell
    /// [`Session::excluded`].
    Exclude(Found),
    /// Walk every scope again.
    Rebuild,
    /// Open the folder picker at this drive's root, off the UI thread, and
    /// hand what is chosen to [`Session::picked`].
    PickFolder(PathBuf),
    /// Add this folder as a scope through the settings store, then tell
    /// [`Session::added`].
    AddScope(PathBuf),
    /// Remember *Not now* for this drive's root (`declined-drives`).
    DeclineDrive(PathBuf),
}

/// What the view says about the index itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    /// File search is on and has somewhere to look.
    pub scopes: bool,
    /// A build or a reconcile is running: what is found may be incomplete.
    pub indexing: bool,
}

pub struct Session {
    /// *Recently Used*, as the host read it when the view opened.
    recents: Vec<Found>,
    /// What is typed, trimmed.
    query: String,
    /// `None` is every type.
    filter: Option<Category>,
    /// The latest answer, and the query and filter it answers.
    results: Vec<Found>,
    answered: Option<(String, Option<Category>)>,
    /// The folder `results` lists, when they are a typed path's listing
    /// rather than the index's answer.
    listed: Option<PathBuf>,
    /// The answer shown is the first phase, and the second is under way,
    /// or the contents are.
    searching: bool,
    /// Windows Search is asked for contents (`files.search-contents`).
    contents: bool,
    /// What `searching` waits for is the contents, not the second phase.
    asking_contents: bool,
    /// Bumped for every search asked, so an older answer is known as one.
    generation: u64,
    /// Each pane built, by path, and the paths asked for with the query's
    /// words the pane was asked with.
    details: HashMap<PathBuf, ItemDetail>,
    asked: HashMap<PathBuf, Vec<String>>,
    /// Entries excluded while the view is open: gone from it at once, before
    /// the index has read the ignore file.
    gone: HashSet<PathBuf>,
    status: Status,
    /// Shortens *Where* in the pane shown before a row is described.
    home: Option<PathBuf>,
    /// Internal drives no scope is on and not declined (slice 5), offered
    /// above *Recently Used* and under an empty result.
    offers: Vec<Drive>,
    /// A whole drive picked, waiting for *Add anyway* or *Cancel*.
    whole_drive: Option<PathBuf>,
}

impl Session {
    pub fn new(recents: Vec<Found>, status: Status) -> Self {
        Self {
            recents,
            query: String::new(),
            filter: None,
            results: Vec::new(),
            answered: None,
            listed: None,
            searching: false,
            contents: false,
            asking_contents: false,
            generation: 0,
            details: HashMap::new(),
            asked: HashMap::new(),
            gone: HashSet::new(),
            status,
            home: None,
            offers: Vec::new(),
            whole_drive: None,
        }
    }

    /// The drives to offer folders from: [`drives::uncovered`].
    pub fn with_drives(mut self, offers: Vec<Drive>) -> Self {
        self.offers = offers;
        self
    }

    /// What the folder picker returned: `None` when it was cancelled. A
    /// folder is added; a whole drive is asked about first.
    pub fn picked(&mut self, folder: Option<PathBuf>) -> Vec<Effect> {
        let Some(folder) = folder else {
            return Vec::new();
        };
        if drives::is_drive_root(&folder) {
            self.whole_drive = Some(folder);
            return vec![Effect::Render(self.view())];
        }
        vec![Effect::AddScope(folder)]
    }

    /// `folder` is a scope now, or already was: its drive is covered, and
    /// offered no more.
    pub fn added(&mut self, folder: &Path) -> Vec<Effect> {
        self.offers
            .retain(|drive| !drives::same_drive(&drive.root, folder));
        vec![Effect::Render(self.view())]
    }

    /// Whether contents are asked for (`files.search-contents`).
    pub fn with_contents(mut self, contents: bool) -> Self {
        self.contents = contents;
        self
    }

    /// The user's home folder, to shorten *Where*.
    pub fn with_home(mut self, home: Option<PathBuf>) -> Self {
        self.home = home;
        self
    }

    /// The first render, and the first row's pane.
    pub fn open(&mut self) -> Vec<Effect> {
        self.shown_again()
    }

    pub fn event(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::QueryChanged { query } => {
                let query = query.trim().to_string();
                if query == self.query && !self.pending() {
                    return Vec::new();
                }
                self.query = query;
                self.ask()
            }
            Event::ScopeChanged { scope, option } if scope == TYPE_SCOPE => {
                let filter = Category::from_id(&option);
                if filter == self.filter && !self.pending() {
                    return Vec::new();
                }
                self.filter = filter;
                self.ask()
            }
            Event::SelectionChanged { item } => match self.shown_by_id(&item) {
                Some(found) => self.describe(found).into_iter().collect(),
                None => Vec::new(),
            },
            Event::ActionInvoked {
                item: Some(item),
                action,
            } => {
                if let Some(effects) = self.drive_action(&item, &action) {
                    return effects;
                }
                let Some(found) = self.shown_by_id(&item) else {
                    return Vec::new();
                };
                let path = found.path.clone();
                match action.as_str() {
                    OPEN => vec![Effect::Open(found)],
                    SHOW_IN_FOLDER => vec![Effect::ShowInFolder(path)],
                    COPY_PATH => vec![Effect::CopyPath(path.display().to_string())],
                    OPEN_WITH => vec![Effect::OpenWith(path)],
                    SAVE_AS_QUICKLINK => vec![Effect::SaveAsQuicklink {
                        name: found.name(),
                        link: path.display().to_string(),
                    }],
                    EXCLUDE => vec![Effect::Exclude(found)],
                    REBUILD => vec![Effect::Rebuild],
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    /// The index's whole answer to the search of `generation`. Dropped
    /// when a later one was asked: it answers a query nobody is looking at.
    pub fn found(&mut self, generation: u64, found: Vec<Found>) -> Vec<Effect> {
        if generation != self.generation {
            return Vec::new();
        }
        self.results = found;
        self.answered = Some((self.query.clone(), self.filter));
        self.listed = self.typed().map(|typed| typed.dir);
        self.searching = false;
        let contents = self.ask_contents(generation);
        let mut effects = self.shown_again();
        effects.extend(contents);
        effects
    }

    /// The first phase of the answer to the search of `generation`, with a
    /// second to follow (`docs/FILE-SEARCH.md`, "Step c3b"): drawn at once,
    /// saying *Searching...*, and the rest asked for up to the limit. A
    /// first phase already at the limit is the whole answer.
    pub fn found_first(&mut self, generation: u64, found: Vec<Found>) -> Vec<Effect> {
        // A listing read the folder whole: it has no second phase.
        if generation != self.generation || self.typed().is_some() {
            return self.found(generation, found);
        }
        let room = super::LIMIT.saturating_sub(found.len());
        self.results = found;
        self.answered = Some((self.query.clone(), self.filter));
        self.searching = room > 0;
        let mut effects = self.shown_again();
        if self.searching {
            effects.push(Effect::SearchInside {
                generation,
                query: self.query.clone(),
                filter: self.filter,
                limit: room,
            });
        }
        effects
    }

    /// The second phase of the search of `generation`: appended below the
    /// first, without what it already shows and to the limit, so no row
    /// shown moves and the selection stays where it is. Dropped when a
    /// later search was asked.
    pub fn found_inside(&mut self, generation: u64, found: Vec<Found>) -> Vec<Effect> {
        if generation != self.generation || !self.searching || self.asking_contents {
            return Vec::new();
        }
        self.searching = false;
        self.append(found, None);
        let contents = self.ask_contents(generation);
        let mut effects = self.shown_again();
        effects.extend(contents);
        effects
    }

    /// Windows Search's answer for the contents of `generation` (slice
    /// 6b): appended below the names as the second phase is, in the *Type*
    /// filter, the search then whole. Dropped when a later search was asked.
    pub fn found_contents(&mut self, generation: u64, found: Vec<Found>) -> Vec<Effect> {
        if generation != self.generation || !self.asking_contents {
            return Vec::new();
        }
        self.searching = false;
        self.asking_contents = false;
        self.append(found, self.filter);
        self.shown_again()
    }

    /// `found` below the rows shown, without one already there, in
    /// `filter`, and to the limit: no row shown moves.
    fn append(&mut self, found: Vec<Found>, filter: Option<Category>) {
        let mut shown: HashSet<Found> = self.results.iter().cloned().collect();
        for found in found {
            if self.results.len() == super::LIMIT {
                break;
            }
            let held = filter.is_none_or(|filter| filter.holds(&found.name(), found.is_dir));
            if held && shown.insert(found.clone()) {
                self.results.push(found);
            }
        }
    }

    /// The contents asked for, once the names of `generation` are in: from
    /// three letters, never for a typed path, and only with room left.
    fn ask_contents(&mut self, generation: u64) -> Option<Effect> {
        let asked = self.contents
            && self.query.chars().count() >= super::highlight::FROM_CHARACTERS
            && self.typed().is_none()
            && self.results.len() < super::LIMIT;
        if !asked {
            return None;
        }
        self.searching = true;
        self.asking_contents = true;
        Some(Effect::SearchContents {
            generation,
            words: self.query.clone(),
            filter: self.filter,
        })
    }

    /// What `Tab` makes of the query with the row `selected`: the typed
    /// path completed to it ([`browse::complete`]), a folder's ending in a
    /// separator. `None` when what is typed is no path, or the row is not
    /// one of its folder's listing: then `Tab` is the *Type* filter's.
    pub fn completion(&self, selected: &str) -> Option<String> {
        let typed = self.typed()?;
        if self.listed.as_ref() != Some(&typed.dir) {
            return None;
        }
        let found = self.shown_by_id(selected)?;
        Some(browse::complete(&typed, &found))
    }

    /// [`Self::completion`] for every row shown, by id: what the host holds
    /// beside the view it draws, so it knows on its own thread whether
    /// `Tab` is the path's. Empty when no listing of the typed folder is in.
    pub fn completions(&self) -> HashMap<String, String> {
        let Some(typed) = self.typed() else {
            return HashMap::new();
        };
        if self.listed.as_ref() != Some(&typed.dir) {
            return HashMap::new();
        }
        self.shown()
            .map(|found| (found.id(), browse::complete(&typed, found)))
            .collect()
    }

    /// What is typed, trimmed: the query [`Self::completions`] answers.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The path the whole query names (step 6d), whose folder is listed
    /// from the disk instead of asking the index.
    fn typed(&self) -> Option<Typed> {
        browse::typed(&self.query, self.home.as_deref())
    }

    /// What is typed has not been wholly answered. A keystroke that
    /// changes nothing -- a space trimmed away -- has still cancelled the
    /// search under way, so then it is asked again.
    fn pending(&self) -> bool {
        !self.query.is_empty()
            && (self.searching
                || self.answered.as_ref() != Some(&(self.query.clone(), self.filter)))
    }

    /// The pane for `path`, built.
    pub fn described(&mut self, path: PathBuf, detail: ItemDetail) -> Vec<Effect> {
        let shown = self.shown().any(|found| found.path == path);
        self.details.insert(path, detail);
        if shown {
            vec![Effect::Render(self.view())]
        } else {
            Vec::new()
        }
    }

    /// The index started or finished a build, or file search changed.
    /// A finished build may have more to say, so the query is asked again.
    pub fn set_status(&mut self, status: Status) -> Vec<Effect> {
        if status == self.status {
            return Vec::new();
        }
        let finished = self.status.indexing && !status.indexing;
        self.status = status;
        if finished && !self.query.is_empty() {
            let mut effects = vec![Effect::Render(self.view())];
            effects.extend(self.ask());
            return effects;
        }
        vec![Effect::Render(self.view())]
    }

    /// `path` was written into an ignore file: it leaves the view now.
    pub fn excluded(&mut self, path: &Path) -> Vec<Effect> {
        self.gone.insert(path.to_path_buf());
        self.shown_again()
    }

    /// An action on a drive's offer, the hint under an empty result, or the
    /// whole-drive warning; `None` when `item` is none of them.
    fn drive_action(&mut self, item: &str, action: &str) -> Option<Vec<Effect>> {
        if let Some(root) = item.strip_prefix(WHOLE_DRIVE_ID) {
            let root = PathBuf::from(root);
            if self.whole_drive.as_ref() != Some(&root) {
                return Some(Vec::new());
            }
            return Some(match action {
                ADD_ANYWAY => {
                    self.whole_drive = None;
                    vec![Effect::AddScope(root)]
                }
                CANCEL => {
                    self.whole_drive = None;
                    vec![Effect::Render(self.view())]
                }
                _ => Vec::new(),
            });
        }
        let root = item
            .strip_prefix(OFFER_ID)
            .or_else(|| item.strip_prefix(HINT_ID))?;
        let root = PathBuf::from(root);
        if !self.offers.iter().any(|drive| drive.root == root) {
            return Some(Vec::new());
        }
        Some(match action {
            CHOOSE_FOLDERS => vec![Effect::PickFolder(root)],
            NOT_NOW => {
                self.offers.retain(|drive| drive.root != root);
                vec![Effect::DeclineDrive(root), Effect::Render(self.view())]
            }
            _ => Vec::new(),
        })
    }

    /// Search again for what is typed, or show the recents for nothing.
    fn ask(&mut self) -> Vec<Effect> {
        self.generation += 1;
        self.searching = false;
        self.asking_contents = false;
        if self.query.is_empty() {
            self.results.clear();
            self.answered = None;
            self.listed = None;
            return self.shown_again();
        }
        if let Some(typed) = self.typed() {
            return vec![Effect::Browse {
                generation: self.generation,
                typed,
                filter: self.filter,
            }];
        }
        vec![Effect::Search {
            generation: self.generation,
            query: self.query.clone(),
            filter: self.filter,
        }]
    }

    /// Draw, and ask for the first row's pane: where the selection is after
    /// the list changes under it.
    fn shown_again(&mut self) -> Vec<Effect> {
        let mut effects = vec![Effect::Render(self.view())];
        let first = self.shown().next().cloned();
        if let Some(first) = first {
            effects.extend(self.describe(first));
        }
        effects
    }

    fn describe(&mut self, found: Found) -> Option<Effect> {
        // Asked again for other words only when the pane has a text
        // preview, or may have: one is read from just above their match.
        let words = super::highlight::words(&self.query);
        let again = match self.asked.get(&found.path) {
            None => true,
            Some(asked) => {
                *asked != words
                    && self
                        .details
                        .get(&found.path)
                        .is_none_or(|detail| detail.code.is_some())
            }
        };
        if !again {
            return None;
        }
        self.asked.insert(found.path.clone(), words.clone());
        Some(Effect::Describe(found, words))
    }

    /// The rows, in order: the recents in the filter for an empty query,
    /// otherwise the latest answer. A typed path shows only a listing of
    /// its folder, in the filter as well: no row of the index's answers it.
    fn shown(&self) -> impl Iterator<Item = &Found> {
        let recents = self.query.is_empty();
        let typed = self.typed().map(|typed| typed.dir);
        let rows: &[Found] = if recents {
            &self.recents
        } else if typed.is_some() && typed != self.listed {
            &[]
        } else {
            &self.results
        };
        let narrowed = recents || typed.is_some();
        rows.iter().filter(move |found| {
            !self.gone.contains(&found.path)
                && (!narrowed
                    || self
                        .filter
                        .is_none_or(|filter| filter.holds(&found.name(), found.is_dir)))
        })
    }

    fn shown_by_id(&self, id: &str) -> Option<Found> {
        let wanted = Found::from_id(id)?;
        self.shown().find(|found| **found == wanted).cloned()
    }

    pub fn view(&self) -> View {
        let shown: Vec<&Found> = self.shown().collect();
        let items: Vec<Item> = shown
            .iter()
            .zip(prefixes(&shown))
            .map(|(found, prefix)| Item {
                prefix,
                ..self.item(found)
            })
            .collect();
        let title = if self.query.is_empty() {
            "Recently Used"
        } else {
            "Results"
        };
        // Above everything: the whole drive picked, then, before anything
        // is typed, the drives offered. Untitled: each row says what it is.
        let mut lead: Vec<Item> = self
            .whole_drive
            .as_deref()
            .map(whole_drive_row)
            .into_iter()
            .collect();
        if self.query.is_empty() {
            lead.extend(self.offers.iter().take(MAX_OFFERS).map(offer_row));
        }
        let mut sections = Vec::new();
        if !lead.is_empty() {
            sections.push(Section {
                title: None,
                subtitle: None,
                items: lead,
            });
        }
        if !items.is_empty() {
            sections.push(Section {
                title: Some(title.into()),
                // The second phase is the nearer news: it ends in moments.
                // A listing is the disk's, whatever the index is doing.
                subtitle: if self.searching {
                    Some("Searching\u{2026}".into())
                } else if self.typed().is_some() {
                    None
                } else {
                    self.status.indexing.then(|| "Indexing\u{2026}".into())
                },
                items,
            });
        } else if let Some(drive) = self.offers.first().filter(|_| self.found_nothing()) {
            // An empty result explains itself: as a row, so Enter opens
            // the picker at the drive.
            sections.push(Section {
                title: None,
                subtitle: None,
                items: vec![hint_row(drive)],
            });
        }

        View::List(List {
            placeholder: Some("Search files\u{2026}".into()),
            filtering: Filtering::Extension,
            scopes: vec![self.type_scope()],
            path: Vec::new(),
            empty: Some(self.empty()),
            showing_detail: true,
            sections,
        })
    }

    /// What is typed was answered, with nothing, by an index that is not
    /// still being built.
    fn found_nothing(&self) -> bool {
        self.status.scopes
            && self.typed().is_none()
            && !self.status.indexing
            && !self.searching
            && !self.query.is_empty()
            && self.results.is_empty()
            && self.answered.as_ref() == Some(&(self.query.clone(), self.filter))
    }

    fn empty(&self) -> EmptyState {
        // A listing needs no index: only whether it has answered.
        let (title, description) = if self.typed().is_some() {
            if self.pending() {
                ("", None)
            } else {
                ("No files match", None)
            }
        } else if !self.status.scopes {
            (
                "Nothing is indexed",
                Some("Add a folder to \"files.scopes\" in config.json, with \"files.enabled\" on"),
            )
        } else if self.status.indexing {
            (
                "Indexing\u{2026}",
                Some("Files appear here as soon as they are indexed"),
            )
        } else if self.query.is_empty() {
            (
                "No recent files",
                Some("Type to search every indexed file and folder"),
            )
        } else if self.pending() {
            // Asked, and not wholly answered yet: a search is milliseconds,
            // and saying "no match" for them would flash.
            ("", None)
        } else {
            ("No files match", None)
        };
        EmptyState {
            title: title.into(),
            description: description.map(Into::into),
            icon: Some(glyph("search")),
        }
    }

    /// *All Types*, then each category, as the clipboard's *Type* filter.
    fn type_scope(&self) -> Scope {
        let options = std::iter::once(ScopeOption {
            id: ALL_TYPES.into(),
            title: "All Types".into(),
            keywords: Vec::new(),
        })
        .chain(CATEGORIES.into_iter().map(|category| ScopeOption {
            id: category.id().into(),
            title: category.label().into(),
            keywords: Vec::new(),
        }))
        .collect();

        Scope {
            id: TYPE_SCOPE.into(),
            title: "Type".into(),
            options,
            selected: self
                .filter
                .map(|filter| filter.id().to_string())
                .unwrap_or_default(),
        }
    }

    fn item(&self, found: &Found) -> Item {
        let detail =
            self.details.get(&found.path).cloned().unwrap_or_else(|| {
                preview::summary(&found.path, found.is_dir, self.home.as_deref())
            });
        Item {
            id: found.id(),
            title: found.name(),
            prefix: None,
            subtitle: None,
            icon: Some(icon(found)),
            accessories: Vec::new(),
            keywords: Vec::new(),
            actions: vec![
                action(OPEN, "Open"),
                action(SHOW_IN_FOLDER, "Show in Folder"),
                action(COPY_PATH, "Copy Path"),
                action(OPEN_WITH, "Open With\u{2026}"),
                action(SAVE_AS_QUICKLINK, "Save as Quicklink"),
                action(EXCLUDE, "Exclude from Index"),
                action(REBUILD, "Rebuild File Index"),
            ],
            detail: Some(detail),
        }
    }
}

/// What each of `shown` draws dimmed before its name: its folder's name,
/// `styles/`, when another row has the same name in any case, so the two
/// can be told apart (F16). `None` for a name of its own. An entry at the
/// top of a drive is told apart by the drive, `C:/`.
pub fn prefixes(shown: &[&Found]) -> Vec<Option<String>> {
    let names: Vec<String> = shown
        .iter()
        .map(|found| found.name().to_lowercase())
        .collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for name in &names {
        *counts.entry(name).or_default() += 1;
    }
    shown
        .iter()
        .zip(&names)
        .map(|(found, name)| {
            if counts[name.as_str()] < 2 {
                return None;
            }
            let parent = found.path.parent()?;
            let folder = match parent.file_name() {
                Some(folder) => folder.to_string_lossy().into_owned(),
                None => parent
                    .to_string_lossy()
                    .trim_end_matches(['\\', '/'])
                    .to_string(),
            };
            Some(format!("{folder}/"))
        })
        .collect()
}

/// The ids of the drive rows (slice 5), the drive's root after the prefix.
/// Never an entry's: those are `file:` and `folder:`.
const OFFER_ID: &str = "drive:";
const HINT_ID: &str = "drive-hint:";
const WHOLE_DRIVE_ID: &str = "whole-drive:";

/// What the pane says beside a drive's offer and the empty result's hint.
const OFFER_DETAIL: &str = "Lumen searches the folders it is given, and none is on this drive. \
                            **Choose folders\u{2026}** opens the picker on it: add the folders \
                            you search rather than the whole drive, which makes the index on \
                            disk larger and its first build longer, and fills it with files \
                            you won't look for. **Not now** stops offering it; a folder can \
                            still be added in Settings, on the File Search page.";

/// *Files on D:\ (Data) aren't searched*, with *Choose folders...* and
/// *Not now*.
fn offer_row(drive: &Drive) -> Item {
    let root = drive.root.display();
    let title = if drive.label.is_empty() {
        format!("Files on {root} aren't searched")
    } else {
        format!("Files on {root} ({}) aren't searched", drive.label)
    };
    drive_row(
        format!("{OFFER_ID}{root}"),
        title,
        vec![
            action(CHOOSE_FOLDERS, "Choose folders\u{2026}"),
            action(NOT_NOW, "Not now"),
        ],
        OFFER_DETAIL.into(),
    )
}

/// The empty result's hint: nothing matched, and this drive is not
/// searched.
fn hint_row(drive: &Drive) -> Item {
    let root = drive.root.display();
    drive_row(
        format!("{HINT_ID}{root}"),
        format!("No files match. Files on {root} aren't searched."),
        vec![action(CHOOSE_FOLDERS, "Choose folders\u{2026}")],
        OFFER_DETAIL.into(),
    )
}

/// A drive's root was picked: what it costs, before it is added.
fn whole_drive_row(root: &Path) -> Item {
    let root = root.display();
    drive_row(
        format!("{WHOLE_DRIVE_ID}{root}"),
        format!(
            "{root} is the whole drive: a larger index, slower to build, mostly of files \
             you won't look for."
        ),
        vec![action(ADD_ANYWAY, "Add anyway"), action(CANCEL, "Cancel")],
        format!(
            "Every file on {root} is indexed, games and libraries included: the \
             index on disk grows larger, its first build takes longer, and it \
             fills with files you won't look for. Choose the folders you search \
             instead."
        ),
    )
}

fn drive_row(id: String, title: String, actions: Vec<Action>, detail: String) -> Item {
    Item {
        id,
        title,
        icon: Some(glyph("folder")),
        actions,
        detail: Some(ItemDetail {
            markdown: detail,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The shell's icon for the entry, over its category's glyph until it
/// arrives.
fn icon(found: &Found) -> Icon {
    let fallback = match Category::of(&found.name(), found.is_dir) {
        Some(Category::Folders) => "folder",
        Some(Category::Images) => "image",
        Some(Category::Code) => "script",
        Some(Category::Documents) => "text",
        _ => "file",
    };
    Icon::File {
        path: found.path.clone(),
        folder: found.is_dir,
        fallback: fallback.into(),
    }
}

fn glyph(name: &str) -> Icon {
    Icon::Glyph { name: name.into() }
}

fn action(id: &str, title: &str) -> Action {
    Action {
        id: id.into(),
        title: title.into(),
        shortcut: None,
        icon: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_index::FileIndex;
    use crate::file_index::walk::tests::Tree;

    const READY: Status = Status {
        scopes: true,
        indexing: false,
    };

    fn file(path: &str) -> Found {
        Found {
            path: path.into(),
            is_dir: false,
        }
    }

    fn folder(path: &str) -> Found {
        Found {
            path: path.into(),
            is_dir: true,
        }
    }

    fn list(view: &View) -> &List {
        match view {
            View::List(list) => list,
            View::Detail(_) => panic!("expected a list"),
        }
    }

    fn titles(session: &Session) -> Vec<String> {
        list(&session.view())
            .items()
            .map(|item| item.title.clone())
            .collect()
    }

    fn query(session: &mut Session, query: &str) -> Vec<Effect> {
        session.event(Event::QueryChanged {
            query: query.into(),
        })
    }

    fn filter(session: &mut Session, option: &str) -> Vec<Effect> {
        session.event(Event::ScopeChanged {
            scope: TYPE_SCOPE.into(),
            option: option.into(),
        })
    }

    fn generation(effects: &[Effect]) -> u64 {
        effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Search { generation, .. } => Some(*generation),
                _ => None,
            })
            .expect("a search was asked")
    }

    fn recents() -> Vec<Found> {
        vec![
            file("C:\\styles\\carbon.json"),
            file("C:\\shots\\Screenshot.png"),
            folder("C:\\projects"),
        ]
    }

    #[test]
    fn with_nothing_typed_the_recents_are_shown_and_the_first_described() {
        let mut session = Session::new(recents(), READY);
        let effects = session.open();
        let View::List(list) = (match &effects[0] {
            Effect::Render(view) => view.clone(),
            other => panic!("expected a render, got {other:?}"),
        }) else {
            panic!("expected a list")
        };
        assert_eq!(list.sections.len(), 1);
        assert_eq!(list.sections[0].title.as_deref(), Some("Recently Used"));
        assert_eq!(list.len(), 3);
        assert_eq!(list.filtering, Filtering::Extension);
        assert!(list.showing_detail);
        assert_eq!(
            effects[1],
            Effect::Describe(file("C:\\styles\\carbon.json"), Vec::new())
        );
    }

    #[test]
    fn the_type_filter_narrows_the_recents() {
        let mut session = Session::new(recents(), READY);
        filter(&mut session, "images");
        assert_eq!(titles(&session), ["Screenshot.png"]);
        filter(&mut session, "folders");
        assert_eq!(titles(&session), ["projects"]);
        filter(&mut session, ALL_TYPES);
        assert_eq!(titles(&session).len(), 3);
    }

    #[test]
    fn the_filter_is_titled_all_types_with_each_category() {
        let session = Session::new(Vec::new(), READY);
        let view = session.view();
        let scope = &list(&view).scopes[0];
        let options: Vec<&str> = scope.options.iter().map(|o| o.title.as_str()).collect();
        assert_eq!(
            options,
            [
                "All Types",
                "Folders",
                "Documents",
                "Images",
                "Video",
                "Audio",
                "Archives",
                "Code"
            ]
        );
        assert_eq!(scope.current().unwrap().title, "All Types");
    }

    #[test]
    fn typing_asks_the_index_and_its_answer_replaces_the_recents() {
        let mut session = Session::new(recents(), READY);
        let effects = query(&mut session, " inv ");
        assert_eq!(
            effects,
            [Effect::Search {
                generation: 1,
                query: "inv".into(),
                filter: None
            }]
        );
        let effects = session.found(1, vec![file("C:\\Invoice.pdf")]);
        assert!(matches!(effects[0], Effect::Render(_)));
        assert_eq!(
            effects[1],
            Effect::Describe(file("C:\\Invoice.pdf"), vec!["inv".into()])
        );
        assert_eq!(titles(&session), ["Invoice.pdf"]);
        let view = session.view();
        assert_eq!(list(&view).sections[0].title.as_deref(), Some("Results"));

        // Back to nothing: the recents again, without asking.
        let effects = query(&mut session, "");
        assert!(matches!(effects[0], Effect::Render(_)));
        assert_eq!(titles(&session).len(), 3);
    }

    #[test]
    fn an_answer_to_an_older_query_is_dropped() {
        let mut session = Session::new(Vec::new(), READY);
        let first = generation(&query(&mut session, "inv"));
        let second = generation(&query(&mut session, "invo"));
        assert!(session.found(first, vec![file("C:\\old.pdf")]).is_empty());
        session.found(second, vec![file("C:\\Invoice.pdf")]);
        assert_eq!(titles(&session), ["Invoice.pdf"]);
    }

    #[test]
    fn changing_the_type_while_typing_searches_again_in_it() {
        let mut session = Session::new(Vec::new(), READY);
        query(&mut session, "plan");
        let effects = filter(&mut session, "code");
        assert_eq!(
            effects,
            [Effect::Search {
                generation: 2,
                query: "plan".into(),
                filter: Some(Category::Code)
            }]
        );
        // Answered, the same type again asks nothing.
        session.found(2, vec![file("C:\\plan.rs")]);
        assert!(filter(&mut session, "code").is_empty());
    }

    #[test]
    fn a_row_is_described_once_when_the_selection_reaches_it() {
        let mut session = Session::new(recents(), READY);
        session.open();
        let id = folder("C:\\projects").id();
        let effects = session.event(Event::SelectionChanged { item: id.clone() });
        assert_eq!(
            effects,
            [Effect::Describe(folder("C:\\projects"), Vec::new())]
        );
        assert!(
            session
                .event(Event::SelectionChanged { item: id.clone() })
                .is_empty()
        );

        let detail = ItemDetail {
            text: Some("built".into()),
            ..Default::default()
        };
        let effects = session.described("C:\\projects".into(), detail.clone());
        assert!(matches!(effects[..], [Effect::Render(_)]));
        let view = session.view();
        let item = list(&view).items().find(|item| item.id == id).unwrap();
        assert_eq!(item.detail.as_ref(), Some(&detail));
    }

    /// A text preview opens at the query's words, so a new query asks for
    /// it again; a pane without one is not read twice (slice 6b).
    #[test]
    fn a_text_preview_is_described_again_for_other_words() {
        let mut session = Session::new(Vec::new(), READY);
        session.open();
        let found = vec![file("C:\\index.html"), folder("C:\\lorem")];
        let asked = generation(&query(&mut session, "lorem"));
        let effects = session.found(asked, found.clone());
        assert_eq!(
            effects[1],
            Effect::Describe(file("C:\\index.html"), vec!["lorem".into()])
        );
        let code = ItemDetail {
            code: Some("text".into()),
            ..Default::default()
        };
        session.described("C:\\index.html".into(), code);
        let folder_id = folder("C:\\lorem").id();
        assert_eq!(
            session.event(Event::SelectionChanged {
                item: folder_id.clone()
            }),
            [Effect::Describe(folder("C:\\lorem"), vec!["lorem".into()])]
        );
        session.described("C:\\lorem".into(), ItemDetail::default());

        let asked = generation(&query(&mut session, "ipsum"));
        let effects = session.found(asked, found);
        assert_eq!(
            effects[1],
            Effect::Describe(file("C:\\index.html"), vec!["ipsum".into()])
        );
        assert!(
            session
                .event(Event::SelectionChanged { item: folder_id })
                .is_empty()
        );
    }

    /// The pane is never blank while the disk is read.
    #[test]
    fn before_it_is_described_a_row_shows_its_name_place_and_type() {
        let session = Session::new(vec![file("C:\\styles\\carbon.json")], READY);
        let view = session.view();
        let detail = list(&view).items().next().unwrap().detail.clone().unwrap();
        let rows: Vec<(&str, &str)> = detail
            .metadata
            .iter()
            .map(|row| (row.label.as_str(), row.value.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("Name", "carbon.json"),
                ("Where", "C:\\styles"),
                ("Type", "JSON")
            ]
        );
    }

    #[test]
    fn enter_opens_and_the_panel_holds_the_rest() {
        let mut session = Session::new(recents(), READY);
        let view = session.view();
        let item = list(&view).items().next().unwrap();
        let actions: Vec<&str> = item.actions.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(
            actions,
            [
                "Open",
                "Show in Folder",
                "Copy Path",
                "Open With\u{2026}",
                "Save as Quicklink",
                "Exclude from Index",
                "Rebuild File Index"
            ]
        );

        let id = file("C:\\styles\\carbon.json").id();
        let mut invoke = |action: &str| {
            session.event(Event::ActionInvoked {
                item: Some(id.clone()),
                action: action.into(),
            })
        };
        let path = PathBuf::from("C:\\styles\\carbon.json");
        assert_eq!(
            invoke(OPEN),
            [Effect::Open(file("C:\\styles\\carbon.json"))]
        );
        assert_eq!(invoke(SHOW_IN_FOLDER), [Effect::ShowInFolder(path.clone())]);
        assert_eq!(
            invoke(COPY_PATH),
            [Effect::CopyPath("C:\\styles\\carbon.json".into())]
        );
        assert_eq!(invoke(OPEN_WITH), [Effect::OpenWith(path)]);
        assert_eq!(
            invoke(SAVE_AS_QUICKLINK),
            [Effect::SaveAsQuicklink {
                name: "carbon.json".into(),
                link: "C:\\styles\\carbon.json".into(),
            }]
        );
        assert_eq!(
            invoke(EXCLUDE),
            [Effect::Exclude(file("C:\\styles\\carbon.json"))]
        );
        assert_eq!(invoke(REBUILD), [Effect::Rebuild]);

        // A row that is not shown does nothing.
        assert!(
            session
                .event(Event::ActionInvoked {
                    item: Some(file("C:\\elsewhere.txt").id()),
                    action: OPEN.into(),
                })
                .is_empty()
        );
    }

    /// Two rows of one name cannot be told apart by it: each says which
    /// folder it is in, in any case. A name of its own says nothing more.
    #[test]
    fn rows_of_the_same_name_are_told_apart_by_their_folder() {
        let shown = [
            file("C:\\config\\styles\\carbon.json"),
            file("C:\\shots\\Screenshot.png"),
            file("C:\\config\\themes\\Carbon.JSON"),
            folder("C:\\carbon.json"),
        ];
        let shown: Vec<&Found> = shown.iter().collect();
        assert_eq!(
            prefixes(&shown),
            [
                Some("styles/".to_string()),
                None,
                Some("themes/".to_string()),
                Some("C:/".to_string()),
            ]
        );

        let mut session = Session::new(
            vec![
                file("C:\\styles\\carbon.json"),
                file("C:\\themes\\carbon.json"),
                file("C:\\notes.txt"),
            ],
            READY,
        );
        let view = session.view();
        let prefixes: Vec<Option<&str>> = list(&view)
            .items()
            .map(|item| item.prefix.as_deref())
            .collect();
        assert_eq!(prefixes, [Some("styles/"), Some("themes/"), None]);
        // The title stays the name: the prefix is drawn, not matched.
        assert_eq!(titles(&session)[0], "carbon.json");

        // Filtered down to one, it needs no folder.
        filter(&mut session, "documents");
        let view = session.view();
        assert_eq!(list(&view).item(0).unwrap().prefix, None);
    }

    /// A row draws the shell's icon for its entry, with its category's
    /// glyph until that arrives.
    #[test]
    fn a_row_asks_for_its_own_icon_over_its_categorys_glyph() {
        let session = Session::new(recents(), READY);
        let view = session.view();
        let icons: Vec<Icon> = list(&view)
            .items()
            .map(|item| item.icon.clone().unwrap())
            .collect();
        assert_eq!(
            icons,
            [
                Icon::File {
                    path: "C:\\styles\\carbon.json".into(),
                    folder: false,
                    fallback: "script".into(),
                },
                Icon::File {
                    path: "C:\\shots\\Screenshot.png".into(),
                    folder: false,
                    fallback: "image".into(),
                },
                Icon::File {
                    path: "C:\\projects".into(),
                    folder: true,
                    fallback: "folder".into(),
                },
            ]
        );
    }

    #[test]
    fn an_excluded_row_leaves_at_once() {
        let mut session = Session::new(recents(), READY);
        session.excluded(Path::new("C:\\projects"));
        assert_eq!(titles(&session), ["carbon.json", "Screenshot.png"]);
    }

    fn empty_title(session: &Session) -> String {
        list(&session.view()).empty.clone().unwrap().title
    }

    #[test]
    fn each_empty_list_says_why() {
        let none = Session::new(
            Vec::new(),
            Status {
                scopes: false,
                indexing: false,
            },
        );
        assert_eq!(empty_title(&none), "Nothing is indexed");
        let description = list(&none.view()).empty.clone().unwrap().description;
        assert!(description.unwrap().contains("files.scopes"));

        let mut session = Session::new(Vec::new(), READY);
        let asked = generation(&query(&mut session, "zzz"));
        assert_eq!(empty_title(&session), "", "not answered yet");
        session.found(asked, Vec::new());
        assert_eq!(empty_title(&session), "No files match");
    }

    #[test]
    fn while_the_index_is_built_the_view_says_so() {
        let indexing = Status {
            scopes: true,
            indexing: true,
        };
        let empty = Session::new(Vec::new(), indexing);
        assert_eq!(empty_title(&empty), "Indexing\u{2026}");

        let mut session = Session::new(recents(), indexing);
        let view = session.view();
        assert_eq!(
            list(&view).sections[0].subtitle.as_deref(),
            Some("Indexing\u{2026}")
        );

        // Finished: said no more, and what is typed is asked again.
        query(&mut session, "plan");
        let effects = session.set_status(READY);
        assert!(matches!(effects[0], Effect::Render(_)));
        assert!(matches!(effects[1], Effect::Search { .. }));
    }

    fn drive(root: &str, label: &str) -> Drive {
        Drive {
            root: root.into(),
            label: label.into(),
        }
    }

    fn offered() -> Vec<Drive> {
        vec![
            drive("D:\\", "Data"),
            drive("E:\\", ""),
            drive("F:\\", "Games"),
        ]
    }

    fn invoke(session: &mut Session, item: Option<String>, action: &str) -> Vec<Effect> {
        session.event(Event::ActionInvoked {
            item,
            action: action.into(),
        })
    }

    fn row_id(session: &Session, title: &str) -> String {
        list(&session.view())
            .items()
            .find(|item| item.title == title)
            .unwrap_or_else(|| panic!("no row {title:?} in {:?}", titles(session)))
            .id
            .clone()
    }

    const OFFER_D: &str = "Files on D:\\ (Data) aren't searched";
    const OFFER_E: &str = "Files on E:\\ aren't searched";

    /// Above *Recently Used*, at most two, each with *Choose folders...*
    /// first and *Not now*.
    #[test]
    fn an_uncovered_drive_is_offered_above_the_recents() {
        let session = Session::new(recents(), READY).with_drives(offered());
        let view = session.view();
        let list = list(&view);
        assert_eq!(list.sections.len(), 2);
        assert_eq!(list.sections[1].title.as_deref(), Some("Recently Used"));
        let offers: Vec<&str> = list.sections[0]
            .items
            .iter()
            .map(|item| item.title.as_str())
            .collect();
        assert_eq!(offers, [OFFER_D, OFFER_E]);
        let actions: Vec<&str> = list.sections[0].items[0]
            .actions
            .iter()
            .map(|action| action.title.as_str())
            .collect();
        assert_eq!(actions, ["Choose folders\u{2026}", "Not now"]);
    }

    #[test]
    fn no_offer_while_typing_or_with_every_drive_covered() {
        let mut session = Session::new(recents(), READY).with_drives(offered());
        let asked = generation(&query(&mut session, "plan"));
        session.found(asked, vec![file("C:\\plan.md")]);
        assert_eq!(titles(&session), ["plan.md"]);

        let session = Session::new(recents(), READY);
        let view = session.view();
        assert_eq!(list(&view).sections.len(), 1);
        assert_eq!(titles(&session).len(), 3);
    }

    /// The picker opens at the drive; the folder chosen is added, and the
    /// drive is offered no more.
    #[test]
    fn choosing_folders_picks_at_the_drive_and_adds_the_one_chosen() {
        let mut session = Session::new(recents(), READY).with_drives(offered());
        let id = row_id(&session, OFFER_D);
        assert_eq!(
            invoke(&mut session, Some(id), CHOOSE_FOLDERS),
            [Effect::PickFolder("D:\\".into())]
        );
        assert!(session.picked(None).is_empty(), "cancelled");
        assert_eq!(
            session.picked(Some("D:\\Work".into())),
            [Effect::AddScope("D:\\Work".into())]
        );
        let effects = session.added(Path::new("D:\\Work"));
        assert!(matches!(effects[0], Effect::Render(_)));
        let titles = titles(&session);
        assert!(!titles.iter().any(|title| title == OFFER_D), "{titles:?}");
        // The next drive takes its place.
        assert_eq!(titles[0], OFFER_E);
        assert_eq!(titles[1], "Files on F:\\ (Games) aren't searched");
    }

    #[test]
    fn not_now_declines_the_drive_and_removes_its_row() {
        let mut session = Session::new(recents(), READY).with_drives(offered());
        let id = row_id(&session, OFFER_E);
        let effects = invoke(&mut session, Some(id), NOT_NOW);
        assert_eq!(effects[0], Effect::DeclineDrive("E:\\".into()));
        assert!(matches!(effects[1], Effect::Render(_)));
        assert!(!titles(&session).iter().any(|title| title == OFFER_E));
    }

    /// A drive's root is asked about before it is added.
    #[test]
    fn a_whole_drive_picked_is_asked_about_first() {
        let mut session = Session::new(recents(), READY).with_drives(offered());
        let effects = session.picked(Some("D:\\".into()));
        assert!(matches!(effects[..], [Effect::Render(_)]));
        let warning = "D:\\ is the whole drive: a larger index, slower to build, mostly of files you won't look for.";
        let view = session.view();
        let first = list(&view).item(0).unwrap();
        assert_eq!(first.title, warning);
        let actions: Vec<&str> = first.actions.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(actions, ["Add anyway", "Cancel"]);

        let id = first.id.clone();
        assert_eq!(
            invoke(&mut session, Some(id.clone()), ADD_ANYWAY),
            [Effect::AddScope("D:\\".into())]
        );
        assert!(!titles(&session).iter().any(|title| title == warning));

        session.picked(Some("E:\\".into()));
        let id = row_id(
            &session,
            "E:\\ is the whole drive: a larger index, slower to build, mostly of files you won't look for.",
        );
        let effects = invoke(&mut session, Some(id), CANCEL);
        assert!(matches!(effects[..], [Effect::Render(_)]));
        assert!(
            !titles(&session)
                .iter()
                .any(|title| title.contains("whole drive"))
        );
    }

    /// Nothing found, and a drive is not searched: the list says so, and
    /// offers the picker at it.
    #[test]
    fn an_empty_result_names_a_drive_that_is_not_searched() {
        let mut session = Session::new(Vec::new(), READY).with_drives(offered());
        let asked = generation(&query(&mut session, "zzz"));
        session.found(asked, Vec::new());
        let view = session.view();
        let hint = list(&view).item(0).expect("a hint row");
        assert_eq!(hint.title, "No files match. Files on D:\\ aren't searched.");
        let actions: Vec<&str> = hint.actions.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(actions, ["Choose folders\u{2026}"]);
        assert_eq!(list(&view).len(), 1);
        let id = hint.id.clone();
        assert_eq!(
            invoke(&mut session, Some(id), CHOOSE_FOLDERS),
            [Effect::PickFolder("D:\\".into())]
        );

        // Every drive covered: the empty state, as before.
        let mut session = Session::new(Vec::new(), READY);
        let asked = generation(&query(&mut session, "zzz"));
        session.found(asked, Vec::new());
        assert!(list(&session.view()).is_empty());
        assert_eq!(empty_title(&session), "No files match");
    }

    /// The whole loop against a real index: what the session asks, the
    /// index answers, filtered inside the scan.
    #[test]
    fn a_built_index_answers_the_session_in_its_filter() {
        let tree = Tree::new("session-index");
        tree.file("plans/plan.md")
            .file("plans/plan.rs")
            .file("plans/plan.png");
        let index = FileIndex::build(&tree.path(".files.db"), &tree.rules()).unwrap();
        let mut session = Session::new(Vec::new(), READY);

        let answer = |session: &mut Session, effects: Vec<Effect>| {
            let Some(Effect::Search {
                generation,
                query,
                filter,
            }) = effects.into_iter().next()
            else {
                panic!("expected a search");
            };
            let found = super::super::search(&index, &query, filter, &|| false);
            session.found(generation, found);
        };

        let asked = query(&mut session, "plan");
        answer(&mut session, asked);
        let mut all = titles(&session);
        all.sort();
        assert_eq!(all, ["plan.md", "plan.png", "plan.rs", "plans"]);

        let asked = filter(&mut session, "code");
        answer(&mut session, asked);
        assert_eq!(titles(&session), ["plan.rs"]);

        let asked = filter(&mut session, "folders");
        answer(&mut session, asked);
        assert_eq!(titles(&session), ["plans"]);
    }

    /// The second phase the session asks for, as `(generation, query,
    /// filter, limit)`.
    fn inside_asked(effects: &[Effect]) -> Option<(u64, String, Option<Category>, usize)> {
        effects.iter().find_map(|effect| match effect {
            Effect::SearchInside {
                generation,
                query,
                filter,
                limit,
            } => Some((*generation, query.clone(), *filter, *limit)),
            _ => None,
        })
    }

    fn subtitle(session: &Session) -> Option<String> {
        list(&session.view())
            .sections
            .last()
            .and_then(|section| section.subtitle.clone())
    }

    /// The first phase is drawn as it arrives, saying more is coming; the
    /// second is appended below it, without what is already shown.
    #[test]
    fn the_first_phase_is_shown_at_once_and_the_second_appended_below() {
        let mut session = Session::new(Vec::new(), READY);
        let asked = generation(&query(&mut session, "port"));
        let effects = session.found_first(asked, vec![file("C:\\report.txt"), file("C:\\port.md")]);
        let Effect::Render(view) = &effects[0] else {
            panic!("the first phase is drawn first");
        };
        let shown: Vec<&str> = list(view).items().map(|item| item.title.as_str()).collect();
        assert_eq!(shown, ["report.txt", "port.md"]);
        assert_eq!(
            list(view).sections[0].subtitle.as_deref(),
            Some("Searching\u{2026}")
        );
        assert_eq!(
            inside_asked(&effects),
            Some((asked, "port".into(), None, super::super::LIMIT - 2))
        );

        let effects = session.found_inside(
            asked,
            vec![file("C:\\port.md"), file("C:\\deep\\myreport.txt")],
        );
        assert!(matches!(effects[0], Effect::Render(_)));
        assert_eq!(titles(&session), ["report.txt", "port.md", "myreport.txt"]);
        assert_eq!(subtitle(&session), None);
    }

    #[test]
    fn a_second_phase_for_an_older_query_is_dropped() {
        let mut session = Session::new(Vec::new(), READY);
        let first = generation(&query(&mut session, "port"));
        session.found_first(first, vec![file("C:\\report.txt")]);
        let second = generation(&query(&mut session, "porta"));
        assert!(
            session
                .found_inside(first, vec![file("C:\\deep\\myreport.txt")])
                .is_empty()
        );
        assert_eq!(titles(&session), ["report.txt"]);
        session.found(second, vec![file("C:\\portable.exe")]);
        assert!(
            session
                .found_inside(first, vec![file("C:\\x.txt")])
                .is_empty()
        );
        assert_eq!(titles(&session), ["portable.exe"]);
        assert_eq!(subtitle(&session), None);
    }

    /// A whole answer, or a first phase already at the limit, asks for
    /// nothing more; the two phases together never pass the limit.
    #[test]
    fn the_limit_counts_both_phases() {
        let limit = super::super::LIMIT;
        let many = |prefix: &str, count: usize| {
            (0..count)
                .map(|n| file(&format!("C:\\{prefix}{n}.txt")))
                .collect::<Vec<_>>()
        };
        let mut session = Session::new(Vec::new(), READY);
        let asked = generation(&query(&mut session, "port"));
        assert_eq!(inside_asked(&session.found(asked, many("a", 3))), None);

        let asked = generation(&query(&mut session, "ports"));
        let effects = session.found_first(asked, many("a", limit));
        assert_eq!(inside_asked(&effects), None);
        assert_eq!(subtitle(&session), None);

        let asked = generation(&query(&mut session, "portsx"));
        let effects = session.found_first(asked, many("a", 3));
        assert_eq!(inside_asked(&effects).unwrap().3, limit - 3);
        session.found_inside(asked, many("b", limit));
        let shown = titles(&session);
        assert_eq!(shown.len(), limit);
        assert_eq!(shown[..4], ["a0.txt", "a1.txt", "a2.txt", "b0.txt"]);
    }

    /// Nothing yet from the first phase is not "No files match" while the
    /// second runs.
    #[test]
    fn an_empty_first_phase_says_nothing_until_the_second_answers() {
        let mut session = Session::new(Vec::new(), READY).with_drives(offered());
        let asked = generation(&query(&mut session, "zqxj"));
        session.found_first(asked, Vec::new());
        assert_eq!(empty_title(&session), "");
        assert!(list(&session.view()).is_empty(), "no hint yet");

        session.found_inside(asked, Vec::new());
        assert_eq!(
            titles(&session),
            ["No files match. Files on D:\\ aren't searched."]
        );
    }

    /// A keystroke that changes nothing -- a space trimmed away -- still
    /// cancelled the search under way, so one still pending is asked again;
    /// one already answered is not.
    #[test]
    fn a_search_cut_short_by_a_keystroke_that_changed_nothing_is_asked_again() {
        let mut session = Session::new(Vec::new(), READY);
        let first = generation(&query(&mut session, "port"));
        let again = generation(&query(&mut session, "port "));
        assert!(again > first);

        session.found_first(again, vec![file("C:\\report.txt")]);
        let effects = filter(&mut session, ALL_TYPES);
        assert!(matches!(effects[..], [Effect::Search { .. }]));
        let asked = generation(&effects);

        session.found_first(asked, vec![file("C:\\report.txt")]);
        session.found_inside(asked, Vec::new());
        assert!(query(&mut session, " port").is_empty());
        assert!(filter(&mut session, ALL_TYPES).is_empty());
    }

    /// The whole loop against an index larger than its hot layer: the first
    /// phase finds by word start, the second inside a word, both in the
    /// filter.
    #[test]
    fn a_built_index_finds_inside_a_word_outside_the_hot_layer() {
        let tree = Tree::new("session-inside");
        tree.file("a/b/plan.rs")
            .file("a/b/myplan.rs")
            .file("a/b/myplan.md");
        let path = tree.path(".files.db");
        drop(FileIndex::build(&path, &tree.rules()).unwrap());
        let store = crate::file_index::Store::open(&path).unwrap();
        let index = FileIndex::load_within(store, &[], 0).unwrap();
        assert!(!index.hot_is_complete());
        let mut session = Session::new(Vec::new(), READY);

        let answer = |session: &mut Session, effects: Vec<Effect>| {
            let Some(Effect::Search {
                generation,
                query,
                filter,
            }) = effects
                .into_iter()
                .find(|e| matches!(e, Effect::Search { .. }))
            else {
                panic!("expected a search");
            };
            let found = super::super::search(&index, &query, filter, &|| false);
            let effects = if super::super::searches_inside(&index, &query) {
                session.found_first(generation, found)
            } else {
                session.found(generation, found)
            };
            if let Some((generation, query, filter, limit)) = inside_asked(&effects) {
                let found = super::super::search_inside(&index, &query, filter, limit, &|| false);
                session.found_inside(generation, found);
            }
        };

        let asked = query(&mut session, "plan");
        answer(&mut session, asked);
        assert_eq!(titles(&session), ["plan.rs", "myplan.md", "myplan.rs"]);

        let asked = filter(&mut session, "code");
        answer(&mut session, asked);
        assert_eq!(titles(&session), ["plan.rs", "myplan.rs"]);

        // Under three letters, word starts only.
        let asked = query(&mut session, "pl");
        answer(&mut session, asked);
        assert_eq!(titles(&session), ["plan.rs"]);
    }

    /// No eight-row cap: the view shows as many as the index finds, to
    /// the limit.
    #[test]
    fn a_query_is_not_capped_at_roots_eight_rows() {
        let tree = Tree::new("session-many");
        for n in 0..30 {
            tree.file(&format!("note-{n:02}.txt"));
        }
        let index = FileIndex::build(&tree.path(".files.db"), &tree.rules()).unwrap();
        let found = super::super::search(&index, "no", None, &|| false);
        assert_eq!(found.len(), 30);
    }

    /// The generation of the folder listing asked for in `effects`.
    fn browsed(effects: &[Effect]) -> u64 {
        effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Browse { generation, .. } => Some(*generation),
                _ => None,
            })
            .expect("a listing was asked")
    }

    fn at_home() -> Session {
        Session::new(recents(), READY).with_home(Some(r"C:\Users\me".into()))
    }

    #[test]
    fn a_typed_path_lists_its_folder_instead_of_asking_the_index() {
        let mut session = at_home();
        let effects = query(&mut session, r" ~\Doc");
        assert_eq!(
            effects,
            [Effect::Browse {
                generation: 1,
                typed: browse::parse(r"~\Doc", Path::new(r"C:\Users\me")).unwrap(),
                filter: None,
            }]
        );
        let effects = filter(&mut session, "folders");
        assert!(matches!(
            effects[..],
            [Effect::Browse {
                generation: 2,
                filter: Some(Category::Folders),
                ..
            }]
        ));

        // A path is not one without a home to expand `~` to; a drive's is.
        let mut session = Session::new(Vec::new(), READY);
        assert!(matches!(
            query(&mut session, r"~\Doc")[..],
            [Effect::Search { .. }]
        ));
        assert!(matches!(
            query(&mut session, "d:/src/")[..],
            [Effect::Browse { .. }]
        ));
    }

    #[test]
    fn a_listing_for_an_older_query_is_dropped() {
        let mut session = at_home();
        let first = browsed(&query(&mut session, r"C:\Users\"));
        let second = browsed(&query(&mut session, r"C:\Users\m"));
        assert!(
            session
                .found(first, vec![folder(r"C:\Users\Public")])
                .is_empty()
        );
        session.found(second, vec![folder(r"C:\Users\me")]);
        assert_eq!(titles(&session), ["me"]);
    }

    #[test]
    fn a_listing_is_shown_in_the_type_asked_for() {
        let mut session = at_home();
        filter(&mut session, "folders");
        let asked = browsed(&query(&mut session, r"C:\src\"));
        session.found(
            asked,
            vec![folder(r"C:\src\lumen"), file(r"C:\src\notes.txt")],
        );
        assert_eq!(titles(&session), ["lumen"]);
    }

    /// The disk was read whole: nothing inside a word is left to find.
    #[test]
    fn a_listing_has_no_second_phase() {
        let mut session = at_home();
        let asked = browsed(&query(&mut session, r"C:\src\lu"));
        let effects = session.found_first(asked, vec![folder(r"C:\src\lumen")]);
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::SearchInside { .. })),
            "{effects:?}"
        );
        assert_eq!(titles(&session), ["lumen"]);
        let view = session.view();
        assert_eq!(list(&view).sections[0].subtitle, None);
        assert!(session.found_inside(asked, vec![file(r"C:\x")]).is_empty());
    }

    #[test]
    fn tab_completes_a_typed_path_to_the_selected_row() {
        let mut session = at_home();
        let asked = browsed(&query(&mut session, "~/Doc"));
        session.found(
            asked,
            vec![
                folder(r"C:\Users\me\Documents"),
                file(r"C:\Users\me\Doc.txt"),
            ],
        );
        let documents = folder(r"C:\Users\me\Documents").id();
        assert_eq!(
            session.completion(&documents).as_deref(),
            Some("~/Documents/")
        );
        assert_eq!(
            session
                .completion(&file(r"C:\Users\me\Doc.txt").id())
                .as_deref(),
            Some("~/Doc.txt")
        );
        // A row not shown, or no path typed: Tab is the Type filter's.
        assert_eq!(session.completion(&folder(r"C:\elsewhere").id()), None);
        let asked = generation(&query(&mut session, "doc"));
        session.found(asked, vec![folder(r"C:\Users\me\Documents")]);
        assert_eq!(session.completion(&documents), None);
    }

    /// What the host holds to decide, on its own thread, whether `Tab` is
    /// the path's: every shown row's completion, for the query it answers.
    #[test]
    fn completions_are_every_shown_rows_for_the_query_they_answer() {
        let mut session = at_home();
        let asked = browsed(&query(&mut session, "~/Doc "));
        session.found(
            asked,
            vec![
                folder(r"C:\Users\me\Documents"),
                file(r"C:\Users\me\Doc.txt"),
            ],
        );
        let completions = session.completions();
        assert_eq!(session.query(), "~/Doc");
        assert_eq!(completions.len(), 2);
        assert_eq!(
            completions
                .get(&folder(r"C:\Users\me\Documents").id())
                .map(String::as_str),
            Some("~/Documents/")
        );

        // Another folder typed, its listing not yet in: none.
        browsed(&query(&mut session, r"C:\"));
        assert!(session.completions().is_empty());
        // No path: none.
        let asked = generation(&query(&mut session, "doc"));
        session.found(asked, vec![folder(r"C:\Users\me\Documents")]);
        assert!(session.completions().is_empty());
    }

    fn contents_asked(effects: &[Effect]) -> Option<(u64, String, Option<Category>)> {
        effects.iter().find_map(|effect| match effect {
            Effect::SearchContents {
                generation,
                words,
                filter,
            } => Some((*generation, words.clone(), *filter)),
            _ => None,
        })
    }

    fn with_contents() -> Session {
        Session::new(Vec::new(), READY).with_contents(true)
    }

    /// From three letters, once the names have answered, Windows Search is
    /// asked for the contents; *Searching...* is said until it answers.
    #[test]
    fn contents_are_asked_after_the_names_from_three_letters() {
        let mut session = with_contents();
        let asked = generation(&query(&mut session, "rep"));
        let effects = session.found(asked, vec![file(r"C:\report.txt")]);
        assert_eq!(contents_asked(&effects), Some((asked, "rep".into(), None)));
        assert_eq!(subtitle(&session), Some("Searching\u{2026}".into()));

        // After a second phase, not before it.
        let asked = generation(&query(&mut session, "repo"));
        let effects = session.found_first(asked, vec![file(r"C:\report.txt")]);
        assert_eq!(contents_asked(&effects), None);
        let effects = session.found_inside(asked, Vec::new());
        assert_eq!(contents_asked(&effects), Some((asked, "repo".into(), None)));
        assert_eq!(subtitle(&session), Some("Searching\u{2026}".into()));

        session.found_contents(asked, vec![file(r"C:\notes\minutes.txt")]);
        assert_eq!(titles(&session), ["report.txt", "minutes.txt"]);
        assert_eq!(subtitle(&session), None);
    }

    #[test]
    fn contents_are_not_asked_for_two_letters_a_path_or_when_turned_off() {
        let mut session = with_contents();
        let asked = generation(&query(&mut session, "re"));
        assert_eq!(contents_asked(&session.found(asked, Vec::new())), None);
        assert_eq!(subtitle(&session), None);

        let mut session = Session::new(Vec::new(), READY).with_contents(false);
        let asked = generation(&query(&mut session, "report"));
        assert_eq!(contents_asked(&session.found(asked, Vec::new())), None);

        let mut session = at_home().with_contents(true);
        let asked = browsed(&query(&mut session, r"C:\src\lumen"));
        let effects = session.found(asked, vec![folder(r"C:\src\lumen")]);
        assert_eq!(contents_asked(&effects), None);

        // A list already at the limit has no room for them.
        let mut session = with_contents();
        let asked = generation(&query(&mut session, "report"));
        let full = (0..super::super::LIMIT)
            .map(|n| file(&format!(r"C:\{n}.txt")))
            .collect();
        assert_eq!(contents_asked(&session.found(asked, full)), None);
    }

    /// Appended below the names, without a row already shown, in the type
    /// asked for and to the limit; an answer to an older query is dropped.
    #[test]
    fn contents_are_appended_once_in_the_filter_and_stale_ones_dropped() {
        let mut session = with_contents();
        filter(&mut session, "documents");
        let first = generation(&query(&mut session, "budget"));
        session.found(first, vec![file(r"C:\budget.txt")]);
        let second = generation(&query(&mut session, "budgets"));
        assert!(
            session
                .found_contents(first, vec![file(r"C:\stale.txt")])
                .is_empty()
        );
        session.found(second, vec![file(r"C:\budgets.txt")]);
        let effects = session.found_contents(
            second,
            vec![
                file(r"C:\budgets.txt"),
                file(r"C:\photo.png"),
                file(r"C:\plan.txt"),
            ],
        );
        assert!(matches!(effects[0], Effect::Render(_)));
        assert_eq!(titles(&session), ["budgets.txt", "plan.txt"]);
        assert!(
            session
                .found_contents(second, vec![file(r"C:\again.txt")])
                .is_empty(),
            "answered once"
        );

        let mut session = with_contents();
        let asked = generation(&query(&mut session, "report"));
        let limit = super::super::LIMIT;
        let names = (0..limit - 1)
            .map(|n| file(&format!(r"C:\a{n}.txt")))
            .collect();
        session.found(asked, names);
        session.found_contents(asked, vec![file(r"C:\b0.txt"), file(r"C:\b1.txt")]);
        assert_eq!(titles(&session).len(), limit);
    }

    /// A query waiting for its contents is not wholly answered: it says no
    /// "No files match" yet, and a keystroke that changes nothing asks again.
    #[test]
    fn a_search_waiting_for_its_contents_is_pending() {
        let mut session = with_contents();
        let asked = generation(&query(&mut session, "report"));
        session.found(asked, Vec::new());
        assert_eq!(empty_title(&session), "");
        assert!(matches!(
            query(&mut session, "report ")[..],
            [Effect::Search { .. }]
        ));
    }
}
