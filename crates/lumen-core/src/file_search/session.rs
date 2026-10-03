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
//! The pane is the same: reading a file's first bytes and its times is
//! disk work, so a row's detail is asked for ([`Effect::Describe`]) when the
//! selection lands on it, and until it arrives the row shows what is known
//! without the disk.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use lumen_platform::Drive;

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
    /// Build the pane for this entry off the UI thread
    /// ([`preview::describe`]) and hand it to [`Session::described`].
    Describe(Found),
    /// Open it in its application, or a folder in the file manager, and
    /// count it as used.
    Open(Found),
    /// Show it selected in its folder.
    ShowInFolder(PathBuf),
    /// Put this path on the clipboard.
    CopyPath(String),
    /// Ask the system which application should open it.
    OpenWith(PathBuf),
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
    /// Bumped for every search asked, so an older answer is known as one.
    generation: u64,
    /// Each pane built, by path, and the paths asked for.
    details: HashMap<PathBuf, ItemDetail>,
    asked: HashSet<PathBuf>,
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
            generation: 0,
            details: HashMap::new(),
            asked: HashSet::new(),
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
                if query == self.query {
                    return Vec::new();
                }
                self.query = query;
                self.ask()
            }
            Event::ScopeChanged { scope, option } if scope == TYPE_SCOPE => {
                let filter = Category::from_id(&option);
                if filter == self.filter {
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
                    EXCLUDE => vec![Effect::Exclude(found)],
                    REBUILD => vec![Effect::Rebuild],
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    /// The index's answer to the search of `generation`. Dropped when a
    /// later one was asked: it answers a query nobody is looking at.
    pub fn found(&mut self, generation: u64, found: Vec<Found>) -> Vec<Effect> {
        if generation != self.generation {
            return Vec::new();
        }
        self.results = found;
        self.answered = Some((self.query.clone(), self.filter));
        self.shown_again()
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
        if self.query.is_empty() {
            self.results.clear();
            self.answered = None;
            return self.shown_again();
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
        self.asked
            .insert(found.path.clone())
            .then_some(Effect::Describe(found))
    }

    /// The rows, in order: the recents in the filter for an empty query,
    /// otherwise the latest answer.
    fn shown(&self) -> impl Iterator<Item = &Found> {
        let recents = self.query.is_empty();
        let rows = if recents {
            &self.recents
        } else {
            &self.results
        };
        rows.iter().filter(move |found| {
            !self.gone.contains(&found.path)
                && (!recents
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
                subtitle: self.status.indexing.then(|| "Indexing\u{2026}".into()),
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
            && !self.status.indexing
            && !self.query.is_empty()
            && self.results.is_empty()
            && self.answered.as_ref() == Some(&(self.query.clone(), self.filter))
    }

    fn empty(&self) -> EmptyState {
        let (title, description) = if !self.status.scopes {
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
        } else if self.answered.as_ref() != Some(&(self.query.clone(), self.filter)) {
            // Asked, and not answered yet: a search is milliseconds, and
            // saying "no match" for them would flash.
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
                            you search rather than the whole drive, since everything indexed \
                            is held in memory. **Not now** stops offering it; a folder can \
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
        format!("{root} is the whole drive: everything on it is held in memory."),
        vec![action(ADD_ANYWAY, "Add anyway"), action(CANCEL, "Cancel")],
        format!(
            "Every file on {root} is indexed and kept in Lumen's memory, games and \
             libraries included. Choosing the folders you search instead keeps \
             the index small."
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
            Effect::Describe(file("C:\\styles\\carbon.json"))
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
        assert_eq!(effects[1], Effect::Describe(file("C:\\Invoice.pdf")));
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
        // The same type again asks nothing.
        assert!(filter(&mut session, "code").is_empty());
    }

    #[test]
    fn a_row_is_described_once_when_the_selection_reaches_it() {
        let mut session = Session::new(recents(), READY);
        session.open();
        let id = folder("C:\\projects").id();
        let effects = session.event(Event::SelectionChanged { item: id.clone() });
        assert_eq!(effects, [Effect::Describe(folder("C:\\projects"))]);
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
        let warning = "D:\\ is the whole drive: everything on it is held in memory.";
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
            "E:\\ is the whole drive: everything on it is held in memory.",
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
}
