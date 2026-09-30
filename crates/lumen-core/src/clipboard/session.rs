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

use std::path::{Path, PathBuf};

use jiff::{Timestamp, Zoned};

use super::images;
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
/// A picture's address instead of the picture (F12): where it lives online,
/// or Lumen's copy of it, for a program that reads a path -- a terminal.
pub const PASTE_ADDRESS: &str = "paste-address";
pub const COPY_ADDRESS: &str = "copy-address";

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
    /// Paste this text in the entry's place, then move the entry to the top.
    PasteText { id: i64, text: String },
    /// Put this text on the clipboard, then move the entry to the top.
    CopyText { id: i64, text: String },
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
    /// Where the pictures are, to name their files in the rows and the pane.
    images: Option<PathBuf>,
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
                    // A picture's text is its hash, and a hash contains
                    // every short query sooner or later.
                    let haystack = match kind {
                        Kind::Image => "image".to_string(),
                        _ => entry.text.to_lowercase(),
                    };
                    (entry, kind, haystack)
                })
                .collect(),
            error,
            words: Vec::new(),
            filter: None,
            now,
            images: None,
        }
    }

    /// Where the pictures are kept, from [`super::History::images_dir`].
    pub fn with_images(mut self, dir: Option<PathBuf>) -> Self {
        self.images = dir;
        self
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
                    OPEN => self
                        .opens(entry, *kind)
                        .map(Effect::Open)
                        .into_iter()
                        .collect(),
                    DELETE => vec![Effect::Delete(entry.id)],
                    PASTE_ADDRESS | COPY_ADDRESS => {
                        let Some(text) = self.address(entry) else {
                            return Vec::new();
                        };
                        let id = entry.id;
                        if action == PASTE_ADDRESS {
                            vec![Effect::PasteText { id, text }]
                        } else {
                            vec![Effect::CopyText { id, text }]
                        }
                    }
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
            Kind::Image if self.images.is_some() => {
                // The address a terminal can read, after the picture itself.
                let (paste, copy) = match entry.origin {
                    Some(_) => ("Paste URL", "Copy URL"),
                    None => ("Paste File Path", "Copy File Path"),
                };
                actions.push(action(PASTE_ADDRESS, paste, None));
                actions.push(action(COPY_ADDRESS, copy, None));
                actions.push(action(OPEN, "Open Image", None));
            }
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
            icon: Some(self.icon(entry, kind)),
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
            Kind::Image => {
                if let Some(image) = entry.image {
                    metadata.push(row(
                        "Dimensions",
                        &format!("{}\u{d7}{}", image.width, image.height),
                    ));
                    metadata.push(row("Size", &images::size_label(image.bytes)));
                }
                if let Some(origin) = &entry.origin {
                    metadata.push(row("URL", origin));
                }
            }
            Kind::Files if files.len() > 1 => {
                metadata.push(row("Count", &files.len().to_string()));
            }
            Kind::Link | Kind::Files => {}
        }
        if let Some(at) = copied_at(entry) {
            metadata.push(row("Copied", &crate::dates::moment(at, &self.now)));
        }

        // A picture is shown, not its hash.
        let (text, image) = match kind {
            Kind::Image => (None, self.picture(entry, images::original)),
            _ => (Some(entry.text.clone()), None),
        };
        ItemDetail {
            markdown: String::new(),
            text,
            image,
            metadata,
        }
    }

    /// What *Open* opens, when there is one thing to open.
    fn opens(&self, entry: &Entry, kind: Kind) -> Option<String> {
        match kind {
            Kind::Link => super::kind::link(&entry.text),
            Kind::Files => match entry.files().as_slice() {
                [file] => Some(file.to_string_lossy().into_owned()),
                _ => None,
            },
            Kind::Image => self
                .picture(entry, images::original)
                .map(|path| path.to_string_lossy().into_owned()),
            Kind::Text | Kind::Color(_) => None,
        }
    }

    fn icon(&self, entry: &Entry, kind: Kind) -> Icon {
        match kind {
            Kind::Text => glyph("text"),
            Kind::Link => glyph("link"),
            Kind::Files => glyph("file"),
            Kind::Color(color) => Icon::Swatch { color: color.hex() },
            Kind::Image => match self.picture(entry, images::thumbnail) {
                Some(path) => Icon::Image { path },
                None => glyph("image"),
            },
        }
    }

    /// Where a picture is, as text: online when it was copied from a page,
    /// and otherwise Lumen's copy -- a screenshot has no other.
    fn address(&self, entry: &Entry) -> Option<String> {
        if entry.payload != super::Payload::Image {
            return None;
        }
        entry.origin.clone().or_else(|| {
            self.picture(entry, images::original)
                .map(|path| path.to_string_lossy().into_owned())
        })
    }

    /// One of a picture's files, when the view knows where they are.
    fn picture(&self, entry: &Entry, file: fn(&Path, &str) -> PathBuf) -> Option<PathBuf> {
        self.images.as_deref().map(|dir| file(dir, &entry.text))
    }
}

/// A file's name rather than its path, which is mostly the same folders
/// over and over; several files are the first and a count.
fn title(entry: &Entry, kind: Kind) -> String {
    if kind == Kind::Image {
        return match entry.image {
            Some(image) => format!("Image ({}\u{d7}{})", image.width, image.height),
            None => "Image".into(),
        };
    }
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
            image: None,
            origin: None,
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

    fn picture(id: i64) -> Entry {
        Entry {
            payload: crate::clipboard::Payload::Image,
            text: "ab".repeat(32),
            image: Some(crate::clipboard::ImageInfo {
                width: 937,
                height: 593,
                bytes: 54_321,
            }),
            ..entry(id, "", "2026-09-30T20:00:00-04:00")
        }
    }

    /// Raycast's row and pane: the size in the title, the picture itself,
    /// its dimensions and its size on disk.
    #[test]
    fn a_picture_is_shown_by_its_thumbnail_and_described_by_its_size() {
        let dir = PathBuf::from("C:\\images");
        let mut session = session(vec![picture(1)]).with_images(Some(dir.clone()));
        let hash = "ab".repeat(32);

        let view = session.view();
        let item = list(&view).items().next().unwrap();
        assert_eq!(item.title, "Image (937\u{d7}593)");
        assert_eq!(
            item.icon,
            Some(Icon::Image {
                path: images::thumbnail(&dir, &hash)
            })
        );
        let detail = item.detail.as_ref().unwrap();
        assert_eq!(detail.image, Some(images::original(&dir, &hash)));
        assert_eq!(detail.text, None, "the hash is not shown");
        let rows: Vec<(&str, &str)> = detail
            .metadata
            .iter()
            .map(|row| (row.label.as_str(), row.value.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("Source", "Code"),
                ("Type", "Image"),
                ("Dimensions", "937\u{d7}593"),
                ("Size", "54 kB"),
                ("Copied", "Today, 8:00 PM"),
            ]
        );

        assert_eq!(
            invoke(&mut session, 1, OPEN),
            [Effect::Open(
                images::original(&dir, &hash).to_string_lossy().into_owned()
            )]
        );
    }

    /// Typing a letter must not find every picture through its hash.
    #[test]
    fn a_picture_is_found_as_an_image_not_by_its_hash() {
        let mut session = session(vec![
            picture(2),
            entry(1, "hello", "2026-09-30T20:00:00-04:00"),
        ]);
        session.event(Event::QueryChanged { query: "ab".into() });
        assert!(titles(&session).is_empty());
        session.event(Event::QueryChanged {
            query: "image".into(),
        });
        assert_eq!(titles(&session), ["Image (937\u{d7}593)"]);

        session.event(Event::QueryChanged { query: "".into() });
        session.event(Event::ScopeChanged {
            scope: TYPE_SCOPE.into(),
            option: "image".into(),
        });
        assert_eq!(titles(&session).len(), 1);
    }

    /// A terminal reads a picture by its path: the page's address when it
    /// came from one, and Lumen's copy when it did not.
    #[test]
    fn a_picture_can_be_pasted_as_where_it_lives() {
        let dir = PathBuf::from("C:\\images");
        let hash = "ab".repeat(32);
        let online = Entry {
            origin: Some("https://example.com/cat.png".into()),
            ..picture(2)
        };
        let mut session = session(vec![online, picture(1)]).with_images(Some(dir.clone()));

        let view = session.view();
        let titles: Vec<Vec<String>> = list(&view)
            .items()
            .map(|item| item.actions.iter().map(|a| a.title.clone()).collect())
            .collect();
        assert!(titles[0].contains(&"Copy URL".to_string()));
        assert!(titles[1].contains(&"Copy File Path".to_string()));
        assert_eq!(titles[1][0], "Paste", "Enter still pastes the picture");

        assert_eq!(
            invoke(&mut session, 2, PASTE_ADDRESS),
            [Effect::PasteText {
                id: 2,
                text: "https://example.com/cat.png".into()
            }]
        );
        assert_eq!(
            invoke(&mut session, 1, COPY_ADDRESS),
            [Effect::CopyText {
                id: 1,
                text: images::original(&dir, &hash).to_string_lossy().into_owned()
            }]
        );
        // Text has no address to give.
        assert!(invoke(&mut session, 99, COPY_ADDRESS).is_empty());
    }
}
