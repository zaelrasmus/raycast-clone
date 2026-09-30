//! The *Search Snippets* view (F11), as decisions without a window.
//!
//! `lumen-app` owns the file and the clipboard; this decides what is shown and
//! what `Enter` does, so all of it is tested here.
//!
//! # The session filters, not the host
//!
//! A snippet is found by what it says as well as by its name. The host's
//! matcher is fuzzy, and fuzzy against a paragraph matches nearly any short
//! query -- and host filtering keeps the view's order rather than the score's
//! (7.30), so the loose matches would not even sink. So the list asks to filter
//! itself, and a snippet is kept when every word typed appears in it.
//!
//! # Arguments are asked for one at a time
//!
//! A snippet with `{argument}` placeholders is not pasted on `Enter`: the
//! search field is emptied and becomes the first argument's answer, with its
//! options as the list if it has any, and the pane showing the snippet filled
//! in so far. `Enter` moves to the next, and on the last pastes. The view, the
//! field and the pane are the ones already there; Raycast's side-by-side
//! fields would be a new component in the search bar.

use std::collections::HashMap;

use jiff::Zoned;

use super::template::{Argument, Template};
use super::{Broken, Format, Loaded, Snippet};
use crate::view::{
    Accessory, Action, EmptyState, Event, Filtering, Icon, Item, ItemDetail, List, MetadataRow,
    Scope, ScopeOption, Section, Tone, View,
};

/// The id of the tag scope.
pub const TAG_SCOPE: &str = "tag";
/// The option that shows every snippet. Empty, because a tag never is.
const ALL_TAGS: &str = "";
/// What `Enter` does.
pub const PASTE: &str = "paste";
pub const COPY: &str = "copy";
/// While arguments are asked for: take what is typed, or the option chosen.
pub const NEXT: &str = "next";
/// While arguments are asked for: back to the list.
pub const BACK: &str = "back";
/// The item that takes what is typed as the answer.
const ANSWER: &str = "answer";
/// Open the form on the snippet, on a copy of it, or empty.
pub const EDIT: &str = "edit";
pub const DUPLICATE: &str = "duplicate";
pub const CREATE: &str = "create";
/// Ask before deleting, then delete or not.
pub const DELETE: &str = "delete";
pub const CONFIRM_DELETE: &str = "confirm-delete";
pub const CANCEL: &str = "cancel";

/// What the daemon is asked to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Draw this.
    Render(View),
    /// Put this in the search field, as if typed.
    SetQuery(String),
    /// Fill in `text`'s placeholders with `answers` for its arguments, close
    /// the launcher and paste it into the window it was opened over.
    Paste {
        text: String,
        answers: HashMap<String, String>,
        format: Format,
    },
    /// The same, onto the clipboard.
    Copy {
        text: String,
        answers: HashMap<String, String>,
        format: Format,
    },
    /// Show the snippet form on `snippet`, saving over the snippet called
    /// `replacing` when there is one, and as a new snippet otherwise.
    OpenForm {
        snippet: Box<Snippet>,
        replacing: Option<String>,
    },
    /// Delete the snippet called this from the file, then read it again and
    /// hand it to [`Session::reload`].
    Delete(String),
}

/// A snippet's arguments being asked for.
struct Asking {
    /// Which snippet, by its place in the list.
    index: usize,
    template: Template,
    arguments: Vec<Argument>,
    answers: HashMap<String, String>,
    /// The argument being asked for now.
    step: usize,
    /// What is typed in the field, which is the answer being written.
    typed: String,
    /// Whether the end of it is a copy rather than a paste.
    copy: bool,
}

pub struct Session {
    /// Newest first, each with what it is matched against, in lower case.
    snippets: Vec<(Snippet, String)>,
    /// What could not be read, said at the end of the list.
    broken: Vec<Broken>,
    /// Why the file could not be read at all, said instead of the list.
    error: Option<String>,
    /// The search, as typed, to put back when arguments are left.
    query: String,
    /// The words typed, in lower case.
    words: Vec<String>,
    asking: Option<Asking>,
    /// The snippet whose deletion is being confirmed.
    confirming: Option<usize>,
    /// `None` is every tag.
    tag: Option<String>,
    /// When the view opened, in the user's time zone: what *Today* means.
    now: Zoned,
}

impl Session {
    pub fn new(loaded: impl Into<Loaded>, now: Zoned) -> Self {
        let Loaded {
            mut snippets,
            broken,
            error,
        } = loaded.into();
        // Stable, so snippets with the same date, or none, keep the file's
        // order; those with none go last.
        snippets.sort_by_key(|snippet| std::cmp::Reverse(snippet.changed()));
        Self {
            snippets: snippets
                .into_iter()
                .map(|snippet| {
                    let haystack = haystack(&snippet);
                    (snippet, haystack)
                })
                .collect(),
            broken,
            error,
            query: String::new(),
            words: Vec::new(),
            asking: None,
            confirming: None,
            tag: None,
            now,
        }
    }

    /// The file again, after it was written: the search and the tag are
    /// kept.
    pub fn reload(&mut self, loaded: impl Into<Loaded>) -> Vec<Effect> {
        let fresh = Session::new(loaded, self.now.clone());
        self.snippets = fresh.snippets;
        self.broken = fresh.broken;
        self.error = fresh.error;
        self.asking = None;
        self.confirming = None;
        vec![Effect::Render(self.view())]
    }

    pub fn event(&mut self, event: Event) -> Vec<Effect> {
        if self.asking.is_some() {
            return self.asking_event(event);
        }
        if let Some(index) = self.confirming {
            return self.confirming_event(index, event);
        }
        match event {
            Event::QueryChanged { query } => {
                let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
                self.query = query;
                if words == self.words {
                    return Vec::new();
                }
                self.words = words;
                vec![Effect::Render(self.view())]
            }
            Event::ScopeChanged { scope, option } if scope == TAG_SCOPE => {
                let tag = (option != ALL_TAGS).then_some(option);
                if tag == self.tag {
                    return Vec::new();
                }
                self.tag = tag;
                vec![Effect::Render(self.view())]
            }
            Event::ActionInvoked {
                item: Some(item),
                action,
            } => {
                let Some(index) = item
                    .parse::<usize>()
                    .ok()
                    .filter(|i| *i < self.snippets.len())
                else {
                    return Vec::new();
                };
                let copy = match action.as_str() {
                    PASTE => false,
                    COPY => true,
                    EDIT => {
                        let snippet = self.snippets[index].0.clone();
                        let replacing = Some(snippet.name.trim().to_string());
                        return vec![Effect::OpenForm {
                            snippet: Box::new(snippet),
                            replacing,
                        }];
                    }
                    DUPLICATE => {
                        let original = &self.snippets[index].0;
                        let all: Vec<Snippet> =
                            self.snippets.iter().map(|(s, _)| s.clone()).collect();
                        let copy = Snippet {
                            name: super::store::copy_name(&all, &original.name),
                            // A copy's keyword would clash with the original's.
                            keyword: None,
                            created: None,
                            modified: None,
                            ..original.clone()
                        };
                        return vec![Effect::OpenForm {
                            snippet: Box::new(copy),
                            replacing: None,
                        }];
                    }
                    CREATE => {
                        return vec![Effect::OpenForm {
                            snippet: Box::default(),
                            replacing: None,
                        }];
                    }
                    DELETE => {
                        self.confirming = Some(index);
                        return vec![Effect::SetQuery(String::new()), Effect::Render(self.view())];
                    }
                    _ => return Vec::new(),
                };
                let text = self.snippets[index].0.text.clone();
                let format = self.snippets[index].0.format;
                let template = Template::parse(&text);
                let arguments = template.arguments();
                if arguments.is_empty() {
                    return vec![finish(text, HashMap::new(), format, copy)];
                }
                self.asking = Some(Asking {
                    index,
                    template,
                    arguments,
                    answers: HashMap::new(),
                    step: 0,
                    typed: String::new(),
                    copy,
                });
                vec![Effect::SetQuery(String::new()), Effect::Render(self.view())]
            }
            _ => Vec::new(),
        }
    }

    fn confirming_event(&mut self, index: usize, event: Event) -> Vec<Effect> {
        let Event::ActionInvoked { action, .. } = event else {
            // Nothing typed means anything while the question is up.
            return Vec::new();
        };
        match action.as_str() {
            CONFIRM_DELETE => {
                self.confirming = None;
                let name = self.snippets[index].0.name.trim().to_string();
                vec![Effect::Delete(name)]
            }
            CANCEL => {
                self.confirming = None;
                vec![
                    Effect::SetQuery(self.query.clone()),
                    Effect::Render(self.view()),
                ]
            }
            _ => Vec::new(),
        }
    }

    /// Two rows: delete, or keep. The pane shows what would go.
    fn confirming_view(&self, index: usize) -> View {
        let snippet = &self.snippets[index].0;
        let row = |id: &str, title: &str| Item {
            id: id.into(),
            title: title.into(),
            subtitle: None,
            icon: Some(glyph()),
            accessories: Vec::new(),
            keywords: Vec::new(),
            actions: vec![Action {
                id: id.into(),
                title: title.into(),
                shortcut: None,
            }],
            detail: Some(self.detail(snippet)),
        };
        View::List(List {
            placeholder: Some(String::new()),
            filtering: Filtering::Extension,
            scopes: Vec::new(),
            empty: None,
            showing_detail: true,
            sections: vec![Section {
                title: Some(format!("Delete \u{201c}{}\u{201d}?", snippet.name.trim())),
                subtitle: Some("it is removed from snippets.json".into()),
                items: vec![
                    row(CONFIRM_DELETE, "Delete Snippet"),
                    row(CANCEL, "Keep It"),
                ],
            }],
        })
    }

    fn asking_event(&mut self, event: Event) -> Vec<Effect> {
        let Some(asking) = self.asking.as_mut() else {
            return Vec::new();
        };
        match event {
            Event::QueryChanged { query } => {
                if query == asking.typed {
                    return Vec::new();
                }
                asking.typed = query;
                vec![Effect::Render(self.view())]
            }
            Event::ActionInvoked { action, .. } if action == BACK => {
                self.asking = None;
                vec![
                    Effect::SetQuery(self.query.clone()),
                    Effect::Render(self.view()),
                ]
            }
            Event::ActionInvoked {
                item: Some(item),
                action,
            } if action == NEXT => {
                let argument = &asking.arguments[asking.step];
                let answer = match item.strip_prefix("option:") {
                    Some(option) => option
                        .parse::<usize>()
                        .ok()
                        .and_then(|option| argument.options.get(option).cloned()),
                    None => argument.resolve(&asking.typed),
                };
                // A required argument with nothing typed, or a choice that is
                // not one of the options, is not an answer yet.
                let Some(answer) = answer else {
                    return Vec::new();
                };
                asking.answers.insert(argument.key.clone(), answer);
                asking.step += 1;
                asking.typed.clear();
                if asking.step < asking.arguments.len() {
                    return vec![Effect::SetQuery(String::new()), Effect::Render(self.view())];
                }
                let asking = self.asking.take().expect("asking, above");
                let snippet = &self.snippets[asking.index].0;
                vec![finish(
                    snippet.text.clone(),
                    asking.answers,
                    snippet.format,
                    asking.copy,
                )]
            }
            _ => Vec::new(),
        }
    }

    /// The field is the answer to the argument asked for now; the list is
    /// that answer, or the options it may be; the pane is the snippet filled
    /// in so far.
    fn asking_view(&self, asking: &Asking) -> View {
        let snippet = &self.snippets[asking.index].0;
        let argument = &asking.arguments[asking.step];
        let last = asking.step + 1 == asking.arguments.len();
        let verb = match (last, asking.copy) {
            (false, _) => "Next",
            (true, false) => "Paste",
            (true, true) => "Copy to Clipboard",
        };
        let actions = vec![
            Action {
                id: NEXT.into(),
                title: verb.into(),
                shortcut: None,
            },
            Action {
                id: BACK.into(),
                title: "Back to Snippets".into(),
                shortcut: None,
            },
        ];
        // What the pane shows if `answer` is given for this argument.
        let preview = |answer: Option<&str>| {
            let mut answers = asking.answers.clone();
            if let Some(answer) = answer {
                answers.insert(argument.key.clone(), answer.to_string());
            }
            let text = asking.template.preview(&answers);
            match snippet.format {
                Format::Markdown => ItemDetail {
                    markdown: text,
                    text: None,
                    image: None,
                    metadata: Vec::new(),
                },
                Format::Text | Format::Unknown => ItemDetail {
                    markdown: String::new(),
                    text: Some(text),
                    image: None,
                    metadata: Vec::new(),
                },
            }
        };

        let items: Vec<Item> = if argument.options.is_empty() {
            let answer = argument.resolve(&asking.typed);
            let (title, subtitle) = match (&answer, asking.typed.trim().is_empty()) {
                (Some(answer), true) => (answer.clone(), Some("the default".to_string())),
                (Some(answer), false) => (answer.clone(), None),
                (None, _) => (format!("Type the {}", argument.label()), None),
            };
            vec![Item {
                id: ANSWER.into(),
                title,
                subtitle,
                icon: Some(glyph()),
                accessories: Vec::new(),
                keywords: Vec::new(),
                actions: actions.clone(),
                detail: Some(preview(answer.as_deref())),
            }]
        } else {
            let typed = asking.typed.trim().to_lowercase();
            argument
                .options
                .iter()
                .enumerate()
                .filter(|(_, option)| option.to_lowercase().contains(&typed))
                .map(|(index, option)| Item {
                    id: format!("option:{index}"),
                    title: option.clone(),
                    subtitle: (argument.default.as_deref() == Some(option.as_str()))
                        .then(|| "the default".to_string()),
                    icon: Some(glyph()),
                    accessories: Vec::new(),
                    keywords: Vec::new(),
                    actions: actions.clone(),
                    detail: Some(preview(Some(option))),
                })
                .collect()
        };

        let placeholder = match &argument.default {
            Some(default) => format!("{} ({default})", argument.label()),
            None => argument.label(),
        };
        View::List(List {
            placeholder: Some(placeholder),
            filtering: Filtering::Extension,
            scopes: Vec::new(),
            empty: Some(EmptyState {
                title: format!("No {} matches", argument.label()),
                description: Some(argument.options.join(", ")),
                icon: Some(glyph()),
            }),
            showing_detail: true,
            sections: vec![Section {
                title: Some(snippet.name.trim().to_string()),
                subtitle: Some(format!("{} of {}", asking.step + 1, asking.arguments.len())),
                items,
            }],
        })
    }

    fn shown(&self, snippet: &Snippet, haystack: &str) -> bool {
        let tagged = match &self.tag {
            None => true,
            Some(wanted) => snippet.tags().any(|tag| tag.eq_ignore_ascii_case(wanted)),
        };
        tagged
            && self
                .words
                .iter()
                .all(|word| haystack.contains(word.as_str()))
    }

    pub fn view(&self) -> View {
        if let Some(asking) = &self.asking {
            return self.asking_view(asking);
        }
        if let Some(index) = self.confirming {
            return self.confirming_view(index);
        }
        let today = self.now.date();
        let mut sections: Vec<Section> = Vec::new();

        for (index, (snippet, haystack)) in self.snippets.iter().enumerate() {
            if !self.shown(snippet, haystack) {
                continue;
            }
            let when = snippet.changed().map(|at| crate::dates::day(at, &self.now));
            let title = crate::dates::period(when, today);
            if sections.last().and_then(|section| section.title.as_deref()) != Some(title) {
                sections.push(Section {
                    title: Some(title.into()),
                    subtitle: None,
                    items: Vec::new(),
                });
            }
            let item = self.item(index, snippet);
            sections.last_mut().expect("pushed above").items.push(item);
        }

        // Last, and not filtered: a snippet that cannot be read is found by
        // looking, not by searching for what it might have said.
        if !self.broken.is_empty() {
            sections.push(Section {
                title: Some("Could not read".into()),
                subtitle: Some("fix these in snippets.json".into()),
                items: self.broken.iter().map(broken_item).collect(),
            });
        }

        let empty = if let Some(error) = &self.error {
            EmptyState {
                title: "snippets.json has a mistake in it".into(),
                description: Some(error.clone()),
                icon: Some(glyph()),
            }
        } else if self.snippets.is_empty() {
            EmptyState {
                title: "No snippets yet".into(),
                description: Some("Snippets are kept in snippets.json, in Lumen's folder".into()),
                icon: Some(glyph()),
            }
        } else {
            EmptyState {
                title: "No matching snippets".into(),
                description: None,
                icon: Some(glyph()),
            }
        };

        View::List(List {
            placeholder: Some("Search snippets\u{2026}".into()),
            filtering: Filtering::Extension,
            scopes: vec![self.tag_scope()],
            empty: Some(empty),
            showing_detail: true,
            sections,
        })
    }

    /// *All Tags*, then every tag in use, sorted, each spelled as it was
    /// first met.
    fn tag_scope(&self) -> Scope {
        let mut tags: Vec<&str> = Vec::new();
        for tag in self.snippets.iter().flat_map(|(snippet, _)| snippet.tags()) {
            if !tags.iter().any(|seen| seen.eq_ignore_ascii_case(tag)) {
                tags.push(tag);
            }
        }
        tags.sort_by_key(|tag| tag.to_lowercase());

        let options = std::iter::once(ScopeOption {
            id: ALL_TAGS.into(),
            title: "All Tags".into(),
            keywords: Vec::new(),
        })
        .chain(tags.into_iter().map(|tag| ScopeOption {
            id: tag.into(),
            title: tag.into(),
            keywords: Vec::new(),
        }))
        .collect();

        Scope {
            id: TAG_SCOPE.into(),
            title: "Tag".into(),
            options,
            selected: self.tag.clone().unwrap_or_default(),
        }
    }

    fn item(&self, index: usize, snippet: &Snippet) -> Item {
        Item {
            id: index.to_string(),
            title: snippet.name.trim().into(),
            subtitle: None,
            icon: Some(glyph()),
            accessories: snippet
                .keyword()
                .map(|keyword| Accessory::Tag {
                    text: keyword.into(),
                    tone: Tone::Neutral,
                })
                .into_iter()
                .collect(),
            keywords: Vec::new(),
            actions: vec![
                Action {
                    id: PASTE.into(),
                    title: "Paste".into(),
                    shortcut: None,
                },
                Action {
                    id: COPY.into(),
                    title: "Copy to Clipboard".into(),
                    shortcut: None,
                },
                Action {
                    id: EDIT.into(),
                    title: "Edit Snippet".into(),
                    shortcut: None,
                },
                Action {
                    id: DUPLICATE.into(),
                    title: "Duplicate Snippet".into(),
                    shortcut: None,
                },
                Action {
                    id: CREATE.into(),
                    title: "Create Snippet".into(),
                    shortcut: None,
                },
                Action {
                    id: DELETE.into(),
                    title: "Delete Snippet".into(),
                    shortcut: None,
                },
            ],
            detail: Some(self.detail(snippet)),
        }
    }

    fn detail(&self, snippet: &Snippet) -> ItemDetail {
        let mut metadata = vec![row("Name", snippet.name.trim())];
        if let Some(keyword) = snippet.keyword() {
            metadata.push(row("Keyword", keyword));
        }
        let tags: Vec<&str> = snippet.tags().collect();
        if !tags.is_empty() {
            metadata.push(MetadataRow {
                label: "Tags".into(),
                value: tags.join(", "),
                tone: Tone::Accent,
            });
        }
        metadata.push(row("Content Type", snippet.format.label()));
        if let Some(at) = snippet.modified_at() {
            metadata.push(row("Modified", &crate::dates::moment(at, &self.now)));
        }
        if let Some(at) = snippet.created_at() {
            metadata.push(row("Created", &crate::dates::moment(at, &self.now)));
        }

        match snippet.format {
            Format::Markdown => ItemDetail {
                markdown: snippet.text.clone(),
                text: None,
                image: None,
                metadata,
            },
            Format::Text | Format::Unknown => ItemDetail {
                markdown: String::new(),
                text: Some(snippet.text.clone()),
                image: None,
                metadata,
            },
        }
    }
}

/// A row for a snippet that could not be read: what it is called, or where
/// it is, and why. It has no actions, since there is nothing to paste.
fn broken_item(broken: &Broken) -> Item {
    Item {
        id: format!("broken:{}", broken.position),
        title: broken
            .name
            .clone()
            .unwrap_or_else(|| format!("Snippet {}", broken.position)),
        subtitle: Some(broken.reason.clone()),
        icon: Some(Icon::Glyph {
            name: "snippet".into(),
        }),
        accessories: Vec::new(),
        keywords: Vec::new(),
        actions: Vec::new(),
        detail: Some(ItemDetail {
            markdown: String::new(),
            text: Some(format!(
                "Snippet {} in snippets.json could not be read:

{}",
                broken.position, broken.reason
            )),
            image: None,
            metadata: Vec::new(),
        }),
    }
}

/// The end of choosing a snippet: paste it, or copy it.
fn finish(text: String, answers: HashMap<String, String>, format: Format, copy: bool) -> Effect {
    if copy {
        Effect::Copy {
            text,
            answers,
            format,
        }
    } else {
        Effect::Paste {
            text,
            answers,
            format,
        }
    }
}

/// Everything a word typed may be found in, in lower case.
fn haystack(snippet: &Snippet) -> String {
    let mut haystack = snippet.name.to_lowercase();
    for part in snippet
        .keyword()
        .into_iter()
        .chain(snippet.keywords.iter().map(String::as_str))
        .chain(snippet.tags())
        .chain(std::iter::once(snippet.text.as_str()))
    {
        haystack.push('\n');
        haystack.push_str(&part.to_lowercase());
    }
    haystack
}

fn glyph() -> Icon {
    Icon::Glyph {
        name: "snippet".into(),
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

    /// A Wednesday evening, in a zone that is not UTC, so a test that forgot
    /// the zone would be a day off.
    fn now() -> Zoned {
        "2026-09-30T21:00:00-04:00[America/New_York]"
            .parse()
            .unwrap()
    }

    fn snippet(name: &str, text: &str) -> Snippet {
        Snippet {
            name: name.into(),
            text: text.into(),
            keywords: Vec::new(),
            keyword: None,
            tags: Vec::new(),
            created: None,
            modified: None,
            format: Format::Text,
            extra: Default::default(),
        }
    }

    fn dated(name: &str, modified: &str) -> Snippet {
        Snippet {
            modified: Some(modified.into()),
            ..snippet(name, "text")
        }
    }

    fn tagged(name: &str, tags: &[&str]) -> Snippet {
        Snippet {
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            ..snippet(name, "text")
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

    fn sections(session: &Session) -> Vec<(String, usize)> {
        list(&session.view())
            .sections
            .iter()
            .map(|section| (section.title.clone().unwrap(), section.items.len()))
            .collect()
    }

    fn typed(session: &mut Session, query: &str) {
        session.event(Event::QueryChanged {
            query: query.into(),
        });
    }

    #[test]
    fn newest_first_in_sections_by_when_they_changed() {
        let session = Session::new(
            vec![
                snippet("Undated", "x"),
                // 01:30 UTC on the 30th is still the 29th in New York.
                dated("Last night", "2026-09-30T01:30:00Z"),
                dated("This morning", "2026-09-30T13:00:00Z"),
                dated("Monday", "2026-09-28T15:00:00Z"),
                dated("Earlier this month", "2026-09-02T15:00:00Z"),
                dated("August", "2026-08-31T15:00:00Z"),
            ],
            now(),
        );

        assert_eq!(
            titles(&session),
            [
                "This morning",
                "Last night",
                "Monday",
                "Earlier this month",
                "August",
                "Undated"
            ]
        );
        assert_eq!(
            sections(&session),
            [
                ("Today".to_string(), 1),
                ("Yesterday".to_string(), 1),
                ("This Week".to_string(), 1),
                ("This Month".to_string(), 1),
                ("Earlier".to_string(), 2),
            ]
        );
    }

    #[test]
    fn every_word_typed_must_appear_somewhere_in_the_snippet() {
        let mut session = Session::new(
            vec![
                snippet("Address", "1 Example Street\nSpringfield"),
                Snippet {
                    keyword: Some(";sig".into()),
                    ..snippet("Signature", "Best,\nYour Name")
                },
                tagged("Standup", &["Work"]),
            ],
            now(),
        );

        typed(&mut session, "spring");
        assert_eq!(titles(&session), ["Address"]);

        typed(&mut session, "  EXAMPLE   street ");
        assert_eq!(titles(&session), ["Address"]);

        typed(&mut session, ";sig");
        assert_eq!(titles(&session), ["Signature"]);

        typed(&mut session, "work");
        assert_eq!(titles(&session), ["Standup"]);

        // Loose letters are not a match, as they would be to a fuzzy matcher.
        typed(&mut session, "adt");
        assert!(titles(&session).is_empty());
    }

    #[test]
    fn the_tag_scope_lists_each_tag_once_and_narrows_the_list() {
        let mut session = Session::new(
            vec![
                tagged("One", &["work", "email"]),
                tagged("Two", &["Work"]),
                tagged("Three", &[]),
            ],
            now(),
        );

        let view = session.view();
        let scope = &list(&view).scopes[0];
        let options: Vec<&str> = scope.options.iter().map(|o| o.title.as_str()).collect();
        assert_eq!(options, ["All Tags", "email", "work"]);
        assert_eq!(scope.current().unwrap().title, "All Tags");

        let effects = session.event(Event::ScopeChanged {
            scope: TAG_SCOPE.into(),
            option: "work".into(),
        });
        assert_eq!(effects.len(), 1);
        // Tags are compared without case: "Work" is "work".
        assert_eq!(titles(&session), ["One", "Two"]);

        session.event(Event::ScopeChanged {
            scope: TAG_SCOPE.into(),
            option: ALL_TAGS.into(),
        });
        assert_eq!(titles(&session).len(), 3);
    }

    #[test]
    fn enter_pastes_the_snippet_it_was_pressed_on_and_the_panel_copies_it() {
        let mut session = Session::new(
            vec![
                dated("Old", "2026-01-01T00:00:00Z"),
                Snippet {
                    text: "new text".into(),
                    ..dated("New", "2026-09-30T13:00:00Z")
                },
            ],
            now(),
        );
        let view = session.view();
        let item = list(&view).item(0).unwrap();
        assert_eq!(item.actions[0].id, PASTE);
        let id = item.id.clone();

        let pasted = session.event(Event::ActionInvoked {
            item: Some(id.clone()),
            action: PASTE.into(),
        });
        assert_eq!(
            pasted,
            [Effect::Paste {
                text: "new text".into(),
                answers: HashMap::new(),
                format: Format::Text,
            }]
        );

        let copied = session.event(Event::ActionInvoked {
            item: Some(id),
            action: COPY.into(),
        });
        assert_eq!(
            copied,
            [Effect::Copy {
                text: "new text".into(),
                answers: HashMap::new(),
                format: Format::Text,
            }]
        );
    }

    #[test]
    fn an_unknown_item_or_action_does_nothing() {
        let mut session = Session::new(vec![snippet("A", "a")], now());
        for (item, action) in [("9", PASTE), ("x", COPY), ("0", "rename")] {
            let effects = session.event(Event::ActionInvoked {
                item: Some(item.into()),
                action: action.into(),
            });
            assert!(effects.is_empty(), "{item} {action}");
        }
    }

    #[test]
    fn plain_text_is_shown_as_written_and_markdown_as_markdown() {
        let session = Session::new(
            vec![
                snippet("Plain", "a * b\nc"),
                Snippet {
                    format: Format::Markdown,
                    ..snippet("Rich", "**bold**")
                },
            ],
            now(),
        );
        let view = session.view();
        let items: Vec<&Item> = list(&view).items().collect();

        let plain = items[0].detail.as_ref().unwrap();
        assert_eq!(plain.text.as_deref(), Some("a * b\nc"));
        assert!(plain.markdown.is_empty());

        let rich = items[1].detail.as_ref().unwrap();
        assert_eq!(rich.markdown, "**bold**");
        assert_eq!(rich.text, None);
    }

    #[test]
    fn the_pane_names_the_dates_in_the_users_time_zone() {
        let session = Session::new(
            vec![Snippet {
                keyword: Some("tks".into()),
                tags: vec!["lasd".into()],
                created: Some("2026-09-03T01:00:19Z".into()),
                modified: Some("2026-09-30T01:00:19Z".into()),
                ..snippet("Test Snippet", "This is some test")
            }],
            now(),
        );
        let view = session.view();
        let item = list(&view).item(0).unwrap();
        let rows: Vec<(&str, &str)> = item
            .detail
            .as_ref()
            .unwrap()
            .metadata
            .iter()
            .map(|row| (row.label.as_str(), row.value.as_str()))
            .collect();

        assert_eq!(
            rows,
            [
                ("Name", "Test Snippet"),
                ("Keyword", "tks"),
                ("Tags", "lasd"),
                ("Content Type", "Text"),
                ("Modified", "Yesterday, 9:00 PM"),
                ("Created", "Sep 2, 2026, 9:00 PM"),
            ]
        );
        assert_eq!(
            item.accessories,
            [Accessory::Tag {
                text: "tks".into(),
                tone: Tone::Neutral
            }]
        );
    }

    #[test]
    fn an_empty_file_and_an_empty_search_say_different_things() {
        let none = Session::new(Vec::new(), now());
        assert_eq!(
            list(&none.view()).empty.as_ref().unwrap().title,
            "No snippets yet"
        );

        let mut some = Session::new(vec![snippet("A", "a")], now());
        typed(&mut some, "zzz");
        assert_eq!(
            list(&some.view()).empty.as_ref().unwrap().title,
            "No matching snippets"
        );
    }

    #[test]
    fn a_file_that_does_not_parse_says_so_instead_of_seeming_empty() {
        let session = Session::new(
            Loaded {
                error: Some("expected `,` at line 4 column 5".into()),
                ..Default::default()
            },
            now(),
        );
        let view = session.view();
        let empty = list(&view).empty.as_ref().unwrap();
        assert_eq!(empty.title, "snippets.json has a mistake in it");
        assert_eq!(
            empty.description.as_deref(),
            Some("expected `,` at line 4 column 5")
        );
    }

    #[test]
    fn snippets_that_could_not_be_read_are_listed_last_whatever_is_typed() {
        let mut session = Session::new(
            Loaded {
                snippets: vec![snippet("Good", "yes")],
                broken: vec![Broken {
                    name: Some("Tagged wrong".into()),
                    position: 2,
                    reason: "invalid type: string \"work\", expected a sequence".into(),
                }],
                error: None,
            },
            now(),
        );
        typed(&mut session, "zzz");

        let view = session.view();
        let last = list(&view).sections.last().unwrap();
        assert_eq!(last.title.as_deref(), Some("Could not read"));
        assert_eq!(last.items[0].title, "Tagged wrong");
        assert!(last.items[0].actions.is_empty());
    }

    fn invoke(session: &mut Session, item: &str, action: &str) -> Vec<Effect> {
        session.event(Event::ActionInvoked {
            item: Some(item.into()),
            action: action.into(),
        })
    }

    fn first_item(session: &Session) -> Item {
        list(&session.view()).item(0).unwrap().clone()
    }

    fn asking_session() -> Session {
        Session::new(
            vec![snippet(
                "Reply",
                "Dear {argument name=who}, I am {argument name=mood options=\"happy, sad\" default=sad}.{cursor}",
            )],
            now(),
        )
    }

    #[test]
    fn a_snippet_with_arguments_asks_for_them_one_at_a_time() {
        let mut session = asking_session();
        typed(&mut session, "reply");

        let effects = invoke(&mut session, "0", PASTE);
        assert_eq!(effects[0], Effect::SetQuery(String::new()));
        let view = session.view();
        let asked = list(&view);
        assert_eq!(asked.placeholder.as_deref(), Some("who"));
        assert_eq!(asked.sections[0].subtitle.as_deref(), Some("1 of 2"));
        // The field is the answer; nothing typed yet, and none by default.
        assert_eq!(first_item(&session).title, "Type the who");
        assert!(invoke(&mut session, ANSWER, NEXT).is_empty());

        typed(&mut session, "Ana");
        let item = first_item(&session);
        assert_eq!(item.title, "Ana");
        assert_eq!(
            item.detail.unwrap().text.as_deref(),
            Some("Dear Ana, I am {mood}.")
        );

        let effects = invoke(&mut session, ANSWER, NEXT);
        assert_eq!(effects[0], Effect::SetQuery(String::new()));
        // The second is a choice: its options are the list, filtered as typed.
        let view = session.view();
        assert_eq!(list(&view).placeholder.as_deref(), Some("mood (sad)"));
        assert_eq!(list(&view).len(), 2);
        typed(&mut session, "HAP");
        let item = first_item(&session);
        assert_eq!(item.title, "happy");
        assert_eq!(item.actions[0].title, "Paste");

        let effects = invoke(&mut session, &item.id, NEXT);
        assert_eq!(
            effects,
            [Effect::Paste {
                text: session.snippets[0].0.text.clone(),
                answers: HashMap::from([
                    ("who".to_string(), "Ana".to_string()),
                    ("mood".to_string(), "happy".to_string())
                ]),
                format: Format::Text,
            }]
        );
    }

    #[test]
    fn back_returns_to_the_list_with_the_search_as_it_was() {
        let mut session = asking_session();
        typed(&mut session, "reply");
        invoke(&mut session, "0", PASTE);
        typed(&mut session, "half an answer");

        let effects = invoke(&mut session, ANSWER, BACK);
        assert_eq!(effects[0], Effect::SetQuery("reply".into()));
        assert_eq!(titles(&session), ["Reply"]);
        // The search field was put back, which the host reports as typed.
        typed(&mut session, "reply");
        assert_eq!(titles(&session), ["Reply"]);
    }

    #[test]
    fn copy_asks_too_and_ends_in_a_copy() {
        let mut session = Session::new(vec![snippet("One", "{argument default=x}")], now());
        invoke(&mut session, "0", COPY);
        // Enter with nothing typed takes the default.
        let effects = invoke(&mut session, ANSWER, NEXT);
        assert_eq!(
            effects,
            [Effect::Copy {
                text: "{argument default=x}".into(),
                answers: HashMap::from([("#1".to_string(), "x".to_string())]),
                format: Format::Text,
            }]
        );
    }

    #[test]
    fn edit_duplicate_and_create_open_the_form() {
        let mut session = Session::new(
            vec![Snippet {
                keyword: Some(";sig".into()),
                created: Some("2026-01-01T00:00:00Z".into()),
                ..snippet("Sig", "Best")
            }],
            now(),
        );

        let [Effect::OpenForm { snippet, replacing }] = &invoke(&mut session, "0", EDIT)[..] else {
            panic!("expected the form");
        };
        assert_eq!(snippet.name, "Sig");
        assert_eq!(replacing.as_deref(), Some("Sig"));

        let [Effect::OpenForm { snippet, replacing }] = &invoke(&mut session, "0", DUPLICATE)[..]
        else {
            panic!("expected the form");
        };
        assert_eq!(snippet.name, "Sig copy");
        assert_eq!(snippet.text, "Best");
        assert_eq!(snippet.keyword, None);
        assert_eq!(snippet.created, None);
        assert_eq!(*replacing, None);

        let [Effect::OpenForm { snippet, replacing }] = &invoke(&mut session, "0", CREATE)[..]
        else {
            panic!("expected the form");
        };
        assert_eq!(**snippet, Snippet::default());
        assert_eq!(*replacing, None);
    }

    #[test]
    fn delete_asks_first_and_reload_keeps_the_search() {
        let mut session = Session::new(vec![snippet("Keep", "k"), snippet("Gone", "g")], now());
        typed(&mut session, "g");
        let id = first_item(&session).id;

        invoke(&mut session, &id, DELETE);
        let view = session.view();
        assert_eq!(
            list(&view).sections[0].title.as_deref(),
            Some("Delete \u{201c}Gone\u{201d}?")
        );
        // Keeping it goes back to the list, the search put back.
        let effects = invoke(&mut session, CANCEL, CANCEL);
        assert_eq!(effects[0], Effect::SetQuery("g".into()));
        assert_eq!(titles(&session), ["Gone"]);

        invoke(&mut session, &id, DELETE);
        let effects = invoke(&mut session, CONFIRM_DELETE, CONFIRM_DELETE);
        assert_eq!(effects, [Effect::Delete("Gone".into())]);

        // The daemon deleted it and read the file again.
        session.reload(vec![snippet("Keep", "k")]);
        assert!(titles(&session).is_empty());
        typed(&mut session, "");
        assert_eq!(titles(&session), ["Keep"]);
    }

    #[test]
    fn the_same_query_again_draws_nothing_new() {
        let mut session = Session::new(vec![snippet("A", "a")], now());
        typed(&mut session, "a");
        assert!(
            session
                .event(Event::QueryChanged {
                    query: " a ".into()
                })
                .is_empty()
        );
    }
}
