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

use super::category::{CATEGORIES, Category};
use super::{Found, preview};
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
        }
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
        let items: Vec<Item> = self.shown().map(|found| self.item(found)).collect();
        let title = if self.query.is_empty() {
            "Recently Used"
        } else {
            "Results"
        };
        let sections = if items.is_empty() {
            Vec::new()
        } else {
            vec![Section {
                title: Some(title.into()),
                subtitle: self.status.indexing.then(|| "Indexing\u{2026}".into()),
                items,
            }]
        };

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

/// A glyph by category: the shell's own icon per file is slice 3b's.
fn icon(found: &Found) -> Icon {
    let name = match Category::of(&found.name(), found.is_dir) {
        Some(Category::Folders) => "folder",
        Some(Category::Images) => "image",
        Some(Category::Code) => "script",
        Some(Category::Documents) => "text",
        _ => "file",
    };
    glyph(name)
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
