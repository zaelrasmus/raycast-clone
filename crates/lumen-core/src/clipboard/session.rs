//! The *Clipboard History* view (F12), as decisions without a window.
//!
//! `lumen-app` owns the database and the clipboard; this decides what is
//! shown and what `Enter` does, so all of it is tested here. The shape is
//! *Search Snippets*': sections by date, a scope to narrow the list, and the
//! selected entry beside it.
//!
//! The session filters, not the host, for snippets' reason: fuzzy against a
//! paragraph matches nearly any short query. An entry is kept when every word
//! typed appears in it.

use jiff::{Timestamp, Zoned};

use super::kind::{Filter, Kind, kind};
use super::store::Entry;
use crate::view::{
    Action, EmptyState, Event, Filtering, Icon, Item, ItemDetail, List, MetadataRow, Scope,
    ScopeOption, Section, Tone, View,
};

/// The id of the type scope.
pub const TYPE_SCOPE: &str = "type";
/// The option that shows every entry. Empty, because a type never is.
const ALL_TYPES: &str = "";
/// What `Enter` does, and the rest of the action panel.
pub const PASTE: &str = "paste";
pub const COPY: &str = "copy";
pub const OPEN: &str = "open";
pub const DELETE: &str = "delete";

/// How much of an entry is shown in a row.
const PREVIEW_LIMIT: usize = 80;

/// What the daemon is asked to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Draw this.
    Render(View),
    /// Close the launcher and paste the entry into the window it was opened
    /// over, then move the entry to the top.
    Paste(Entry),
    /// Put the entry on the clipboard, then move it to the top.
    Copy(Entry),
    /// Open a link in the browser, or a file in its application.
    Open(String),
    /// Forget the entry with this id, then read the history again and hand
    /// it to [`Session::reload`].
    Delete(i64),
}

pub struct Session {
    /// Newest first, each with its kind and what it is matched against.
    entries: Vec<(Entry, Kind, String)>,
    /// Why the history could not be read, said instead of the list.
    error: Option<String>,
    /// The words typed, in lower case.
    words: Vec<String>,
    /// `None` is every type.
    filter: Option<Filter>,
    /// When the view opened, in the user's time zone: what *Today* means.
    now: Zoned,
}

impl Session {
    /// `entries` newest first, or why there are none.
    pub fn new(entries: Result<Vec<Entry>, String>, now: Zoned) -> Self {
        let (entries, error) = match entries {
            Ok(entries) => (entries, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        Self {
            entries: entries
                .into_iter()
                .map(|entry| {
                    let kind = kind(&entry);
                    let haystack = entry.text.to_lowercase();
                    (entry, kind, haystack)
                })
                .collect(),
            error,
            words: Vec::new(),
            filter: None,
            now,
        }
    }

    /// The history again, after an entry went: the search and the type are
    /// kept.
    pub fn reload(&mut self, entries: Result<Vec<Entry>, String>) -> Vec<Effect> {
        let fresh = Session::new(entries, self.now.clone());
        self.entries = fresh.entries;
        self.error = fresh.error;
        vec![Effect::Render(self.view())]
    }

    pub fn event(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::QueryChanged { query } => {
                let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
                if words == self.words {
                    return Vec::new();
                }
                self.words = words;
                vec![Effect::Render(self.view())]
            }
            Event::ScopeChanged { scope, option } if scope == TYPE_SCOPE => {
                let filter = Filter::from_id(&option);
                if filter == self.filter {
                    return Vec::new();
                }
                self.filter = filter;
                vec![Effect::Render(self.view())]
            }
            Event::ActionInvoked {
                item: Some(item),
                action,
            } => {
                let Some((entry, kind, _)) = item
                    .parse::<i64>()
                    .ok()
                    .and_then(|id| self.entries.iter().find(|(entry, ..)| entry.id == id))
                else {
                    return Vec::new();
                };
                match action.as_str() {
                    PASTE => vec![Effect::Paste(entry.clone())],
                    COPY => vec![Effect::Copy(entry.clone())],
                    OPEN => opens(entry, *kind).map(Effect::Open).into_iter().collect(),
                    DELETE => vec![Effect::Delete(entry.id)],
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    fn shown(&self, kind: Kind, haystack: &str) -> bool {
        self.filter.is_none_or(|filter| kind.filter() == filter)
            && self
                .words
                .iter()
                .all(|word| haystack.contains(word.as_str()))
    }

    pub fn view(&self) -> View {
        let today = self.now.date();
        let mut sections: Vec<Section> = Vec::new();

        for (entry, kind, haystack) in &self.entries {
            if !self.shown(*kind, haystack) {
                continue;
            }
            let when = copied_at(entry).map(|at| crate::dates::day(at, &self.now));
            let title = crate::dates::period(when, today);
            if sections.last().and_then(|section| section.title.as_deref()) != Some(title) {
                sections.push(Section {
                    title: Some(title.into()),
                    subtitle: None,
                    items: Vec::new(),
                });
            }
            let item = self.item(entry, *kind);
            sections.last_mut().expect("pushed above").items.push(item);
        }

        let empty = if let Some(error) = &self.error {
            EmptyState {
                title: "Clipboard history is unavailable".into(),
                description: Some(error.clone()),
                icon: Some(glyph("clipboard")),
            }
        } else if self.entries.is_empty() {
            EmptyState {
                title: "Nothing copied yet".into(),
                description: Some("What you copy from now on is kept here".into()),
                icon: Some(glyph("clipboard")),
            }
        } else {
            EmptyState {
                title: "No matching entries".into(),
                description: None,
                icon: Some(glyph("clipboard")),
            }
        };

        View::List(List {
            placeholder: Some("Type to filter entries\u{2026}".into()),
            filtering: Filtering::Extension,
            scopes: vec![self.type_scope()],
            empty: Some(empty),
            showing_detail: true,
            sections,
        })
    }

    /// *All Types*, then each type in [`Kind::FILTERS`] order.
    fn type_scope(&self) -> Scope {
        let options = std::iter::once(ScopeOption {
            id: ALL_TYPES.into(),
            title: "All Types".into(),
            keywords: Vec::new(),
        })
        .chain(Kind::FILTERS.into_iter().map(|filter| ScopeOption {
            id: filter.id().into(),
            title: filter.label().into(),
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

    fn item(&self, entry: &Entry, kind: Kind) -> Item {
        let mut actions = vec![
            action(PASTE, "Paste", None),
            action(COPY, "Copy to Clipboard", None),
        ];
        match kind {
            Kind::Link => actions.push(action(OPEN, "Open in Browser", None)),
            Kind::Files if entry.files().len() == 1 => {
                actions.push(action(OPEN, "Open File", None));
            }
            _ => {}
        }
        actions.push(action(DELETE, "Delete Entry", None));

        Item {
            id: entry.id.to_string(),
            title: title(entry, kind),
            subtitle: None,
            icon: Some(icon(kind)),
            accessories: Vec::new(),
            keywords: Vec::new(),
            actions,
            detail: Some(self.detail(entry, kind)),
        }
    }

    fn detail(&self, entry: &Entry, kind: Kind) -> ItemDetail {
        let mut metadata = Vec::new();
        if let Some(source) = entry.source.as_deref() {
            metadata.push(row("Source", source_name(source)));
        }
        let files = entry.files();
        let type_name = if files.len() > 1 {
            "Files"
        } else {
            kind.filter().name()
        };
        metadata.push(row("Type", type_name));
        match kind {
            Kind::Text => {
                metadata.push(row("Characters", &entry.text.chars().count().to_string()));
                metadata.push(row(
                    "Words",
                    &entry.text.split_whitespace().count().to_string(),
                ));
            }
            Kind::Color(color) => metadata.push(row("Hex", &color.hex())),
            Kind::Files if files.len() > 1 => {
                metadata.push(row("Count", &files.len().to_string()));
            }
            Kind::Link | Kind::Files => {}
        }
        if let Some(at) = copied_at(entry) {
            metadata.push(row("Copied", &crate::dates::moment(at, &self.now)));
        }

        ItemDetail {
            markdown: String::new(),
            text: Some(entry.text.clone()),
            metadata,
        }
    }
}

/// What *Open* opens, when there is one thing to open.
fn opens(entry: &Entry, kind: Kind) -> Option<String> {
    match kind {
        Kind::Link => super::kind::link(&entry.text),
        Kind::Files => match entry.files().as_slice() {
            [file] => Some(file.to_string_lossy().into_owned()),
            _ => None,
        },
        Kind::Text | Kind::Color(_) => None,
    }
}

/// A file's name rather than its path, which is mostly the same folders
/// over and over; several files are the first and a count.
fn title(entry: &Entry, kind: Kind) -> String {
    if kind != Kind::Files {
        return entry.preview(PREVIEW_LIMIT);
    }
    let files = entry.files();
    let name = |file: &std::path::Path| {
        file.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.to_string_lossy().into_owned())
    };
    match files.as_slice() {
        [] => entry.preview(PREVIEW_LIMIT),
        [file] => name(file),
        [file, rest @ ..] => format!("{} and {} more", name(file), rest.len()),
    }
}

fn icon(kind: Kind) -> Icon {
    match kind {
        Kind::Text => glyph("text"),
        Kind::Link => glyph("link"),
        Kind::Files => glyph("file"),
        Kind::Color(color) => Icon::Swatch { color: color.hex() },
    }
}

/// `code.exe` as `code`: the executable is what is known, and the extension
/// says nothing.
fn source_name(source: &str) -> &str {
    let cut = source.len().saturating_sub(4);
    match source.get(cut..) {
        Some(extension) if extension.eq_ignore_ascii_case(".exe") => &source[..cut],
        _ => source,
    }
}

fn copied_at(entry: &Entry) -> Option<Timestamp> {
    Timestamp::from_second(entry.copied_at).ok()
}

fn glyph(name: &str) -> Icon {
    Icon::Glyph { name: name.into() }
}

fn action(id: &str, title: &str, shortcut: Option<&str>) -> Action {
    Action {
        id: id.into(),
        title: title.into(),
        shortcut: shortcut.map(Into::into),
    }
}

fn row(label: &str, value: &str) -> MetadataRow {
    MetadataRow {
        label: label.into(),
        value: value.into(),
        tone: Tone::Neutral,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::Payload;

    /// A Wednesday evening, in a zone that is not UTC, so a test that forgot
    /// the zone would be a day off.
    fn now() -> Zoned {
        "2026-09-30T21:00:00-04:00[America/New_York]"
            .parse()
            .unwrap()
    }

    fn at(when: &str) -> i64 {
        when.parse::<Timestamp>().unwrap().as_second()
    }

    fn entry(id: i64, text: &str, copied_at: &str) -> Entry {
        Entry {
            id,
            payload: Payload::Text,
            text: text.into(),
            copied_at: at(copied_at),
            source: Some("Code.exe".into()),
        }
    }

    fn files(id: i64, paths: &[&str]) -> Entry {
        Entry {
            payload: Payload::Files,
            text: paths.join("\n"),
            ..entry(id, "", "2026-09-30T20:00:00-04:00")
        }
    }

    fn session(entries: Vec<Entry>) -> Session {
        Session::new(Ok(entries), now())
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

    fn invoke(session: &mut Session, id: i64, action: &str) -> Vec<Effect> {
        session.event(Event::ActionInvoked {
            item: Some(id.to_string()),
            action: action.into(),
        })
    }

    #[test]
    fn entries_are_in_sections_by_when_they_were_copied() {
        let session = session(vec![
            entry(3, "tonight", "2026-09-30T20:00:00-04:00"),
            entry(2, "last night", "2026-09-29T23:00:00-04:00"),
            entry(1, "long ago", "2026-01-01T12:00:00-04:00"),
        ]);
        let sections: Vec<(String, usize)> = list(&session.view())
            .sections
            .iter()
            .map(|section| (section.title.clone().unwrap(), section.items.len()))
            .collect();
        assert_eq!(
            sections,
            [
                ("Today".into(), 1),
                ("Yesterday".into(), 1),
                ("Earlier".into(), 1)
            ]
        );
    }

    #[test]
    fn each_type_has_its_icon_and_a_colour_is_a_swatch() {
        let session = session(vec![
            entry(4, "#ff8000", "2026-09-30T20:00:00-04:00"),
            entry(3, "https://example.com", "2026-09-30T20:00:00-04:00"),
            entry(2, "hello", "2026-09-30T20:00:00-04:00"),
            files(1, &[r"C:\a.txt"]),
        ]);
        let icons: Vec<Icon> = list(&session.view())
            .items()
            .map(|item| item.icon.clone().unwrap())
            .collect();
        assert_eq!(
            icons,
            [
                Icon::Swatch {
                    color: "#FF8000".into()
                },
                glyph("link"),
                glyph("text"),
                glyph("file"),
            ]
        );
    }

    #[test]
    fn the_type_scope_narrows_the_list() {
        let mut session = session(vec![
            entry(3, "https://example.com", "2026-09-30T20:00:00-04:00"),
            entry(2, "hello", "2026-09-30T20:00:00-04:00"),
            files(1, &[r"C:\a.txt"]),
        ]);
        session.event(Event::ScopeChanged {
            scope: TYPE_SCOPE.into(),
            option: "link".into(),
        });
        assert_eq!(titles(&session), ["https://example.com"]);

        session.event(Event::ScopeChanged {
            scope: TYPE_SCOPE.into(),
            option: ALL_TYPES.into(),
        });
        assert_eq!(titles(&session).len(), 3);
    }

    /// Found by what is in the middle of it, not only by the first line.
    #[test]
    fn every_word_typed_must_appear_somewhere_in_the_entry() {
        let mut session = session(vec![
            entry(
                2,
                "first line\nthe NEEDLE is here",
                "2026-09-30T20:00:00-04:00",
            ),
            entry(1, "needle alone", "2026-09-30T20:00:00-04:00"),
        ]);
        session.event(Event::QueryChanged {
            query: "needle here".into(),
        });
        assert_eq!(titles(&session), ["first line\u{2026}"]);
    }

    #[test]
    fn files_are_shown_by_name_and_counted() {
        let session = session(vec![
            files(2, &[r"C:\docs\a.txt", r"C:\docs\b.txt", r"C:\docs\c.txt"]),
            files(1, &[r"C:\docs\report.pdf"]),
        ]);
        assert_eq!(titles(&session), ["a.txt and 2 more", "report.pdf"]);
    }

    #[test]
    fn the_pane_says_where_it_came_from_what_it_is_and_when() {
        let session = session(vec![entry(1, "two words", "2026-09-30T20:00:00-04:00")]);
        let view = session.view();
        let item = list(&view).items().next().unwrap();
        let detail = item.detail.as_ref().unwrap();
        let rows: Vec<(&str, &str)> = detail
            .metadata
            .iter()
            .map(|row| (row.label.as_str(), row.value.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("Source", "Code"),
                ("Type", "Text"),
                ("Characters", "9"),
                ("Words", "2"),
                ("Copied", "Today, 8:00 PM"),
            ]
        );
        assert_eq!(detail.text.as_deref(), Some("two words"));
    }

    #[test]
    fn enter_pastes_and_the_panel_copies_opens_and_deletes() {
        let link = entry(2, "www.example.com", "2026-09-30T20:00:00-04:00");
        let text = entry(1, "hello", "2026-09-30T20:00:00-04:00");
        let mut session = session(vec![link.clone(), text.clone()]);

        let view = session.view();
        let first = list(&view).items().next().unwrap();
        assert_eq!(first.actions[0].id, PASTE);
        assert!(first.actions.iter().any(|action| action.id == OPEN));

        assert_eq!(
            invoke(&mut session, 2, PASTE),
            [Effect::Paste(link.clone())]
        );
        assert_eq!(invoke(&mut session, 1, COPY), [Effect::Copy(text)]);
        assert_eq!(
            invoke(&mut session, 2, OPEN),
            [Effect::Open("https://www.example.com".into())]
        );
        // Text has nothing to open.
        assert!(invoke(&mut session, 1, OPEN).is_empty());
        assert_eq!(invoke(&mut session, 1, DELETE), [Effect::Delete(1)]);
        // An entry that is not there does nothing.
        assert!(invoke(&mut session, 99, PASTE).is_empty());
    }

    #[test]
    fn a_reload_keeps_the_search_and_the_type() {
        let mut session = session(vec![
            entry(2, "https://example.com", "2026-09-30T20:00:00-04:00"),
            entry(1, "https://example.org", "2026-09-30T20:00:00-04:00"),
        ]);
        session.event(Event::QueryChanged {
            query: "example".into(),
        });
        session.event(Event::ScopeChanged {
            scope: TYPE_SCOPE.into(),
            option: "link".into(),
        });
        session.reload(Ok(vec![entry(
            1,
            "https://example.org",
            "2026-09-30T20:00:00-04:00",
        )]));
        assert_eq!(titles(&session), ["https://example.org"]);
    }

    #[test]
    fn a_history_that_cannot_be_read_says_why() {
        let session = Session::new(Err("disk full".into()), now());
        let view = session.view();
        let empty = list(&view).empty.clone().unwrap();
        assert_eq!(empty.description.as_deref(), Some("disk full"));
    }
}
