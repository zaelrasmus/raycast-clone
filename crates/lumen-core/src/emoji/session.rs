//! The *Search Emoji & Symbols* view (F18), as decisions without a window.
//!
//! The shape is *Clipboard History*'s: `lumen-app` pastes and copies, this
//! decides what is shown and what `Enter` does. A grid rather than rows,
//! and a category scope where the clipboard has a type one.
//!
//! The session filters, not the host: the catalogue's search ranks a name
//! above a keyword, which a fuzzy pass over titles cannot know.
//!
//! [`Session::new`] reads the catalogue if nothing has yet (~5 ms), so it is
//! made off the UI thread, as the clipboard's session is.

use super::{Catalogue, Category, CustomKeywords, Entry, catalogue, codepoints};
use crate::view::{
    Action, EmptyState, Event, Filtering, Icon, Item, List, Presentation, Scope, ScopeOption,
    Section, View,
};

/// The id of the category scope.
pub const CATEGORY_SCOPE: &str = "category";
/// The option that shows every category. Empty, because a slug never is.
const ALL_CATEGORIES: &str = "";
/// What `Enter` does, and the rest of the action panel.
pub const PASTE: &str = "paste";
pub const COPY: &str = "copy";
pub const COPY_UNICODE: &str = "copy-unicode";
pub const COPY_NAME: &str = "copy-name";

/// What the daemon is asked to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Draw this.
    Render(View),
    /// Close the launcher and paste this into the window it was opened over.
    Paste(String),
    /// Put this on the clipboard.
    Copy(String),
}

pub struct Session {
    catalogue: &'static Catalogue,
    /// As typed; the catalogue's search folds it.
    query: String,
    /// `None` is every category.
    category: Option<Category>,
    /// The user's own keywords (F18, task 7). Empty until they can be added.
    custom: CustomKeywords,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            catalogue: catalogue(),
            query: String::new(),
            category: None,
            custom: CustomKeywords::new(),
        }
    }

    pub fn event(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::QueryChanged { query } => {
                let query = query.trim().to_string();
                if query == self.query {
                    return Vec::new();
                }
                self.query = query;
                vec![Effect::Render(self.view())]
            }
            Event::ScopeChanged { scope, option } if scope == CATEGORY_SCOPE => {
                let category = Category::from_slug(&option);
                if category == self.category {
                    return Vec::new();
                }
                self.category = category;
                vec![Effect::Render(self.view())]
            }
            Event::ActionInvoked {
                item: Some(item),
                action,
            } => {
                let Some(entry) = self.entry(&item) else {
                    return Vec::new();
                };
                match action.as_str() {
                    PASTE => vec![Effect::Paste(entry.chars.to_string())],
                    COPY => vec![Effect::Copy(entry.chars.to_string())],
                    COPY_UNICODE => vec![Effect::Copy(codepoints(&entry.chars))],
                    COPY_NAME => vec![Effect::Copy(title_case(&entry.name))],
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    /// The entry an item stands for: its id is its characters.
    fn entry(&self, chars: &str) -> Option<&'static Entry> {
        self.catalogue
            .entries()
            .iter()
            .find(|entry| &*entry.chars == chars)
    }

    pub fn view(&self) -> View {
        let sections = if self.query.is_empty() {
            self.browse()
        } else {
            let found = self
                .catalogue
                .search(&self.query, self.category, &self.custom);
            section("Results", found).into_iter().collect()
        };

        View::List(List {
            presentation: Presentation::Grid,
            placeholder: Some("Search Emoji & Symbols\u{2026}".into()),
            filtering: Filtering::Extension,
            scopes: vec![self.category_scope()],
            path: Vec::new(),
            empty: Some(EmptyState {
                title: "No matching emoji or symbols".into(),
                description: None,
                icon: None,
            }),
            showing_detail: false,
            sections,
        })
    }

    /// What is shown before anything is typed: each category in the
    /// dropdown's order, which is Unicode's, or the one chosen.
    /// *Frequently Used* and *Pinned* go above them, here (F18, task 6).
    fn browse(&self) -> Vec<Section> {
        Category::ALL
            .into_iter()
            .filter(|category| self.category.is_none_or(|chosen| chosen == *category))
            .filter_map(|category| {
                let entries = self
                    .catalogue
                    .entries()
                    .iter()
                    .filter(|entry| entry.category == category);
                section(category.label(), entries)
            })
            .collect()
    }

    /// *All Categories*, then each in [`Category::ALL`] order.
    fn category_scope(&self) -> Scope {
        let options = std::iter::once(ScopeOption {
            id: ALL_CATEGORIES.into(),
            title: "All Categories".into(),
            keywords: Vec::new(),
        })
        .chain(Category::ALL.into_iter().map(|category| ScopeOption {
            id: category.slug().into(),
            title: category.label().into(),
            keywords: Vec::new(),
        }))
        .collect();

        Scope {
            id: CATEGORY_SCOPE.into(),
            title: "Category".into(),
            options,
            selected: self
                .category
                .map(|category| category.slug().to_string())
                .unwrap_or_default(),
        }
    }
}

/// A titled run of entries with its count, or nothing when it is empty: a
/// header over no cells says nothing.
fn section<'a>(title: &str, entries: impl IntoIterator<Item = &'a Entry>) -> Option<Section> {
    let items: Vec<Item> = entries.into_iter().map(item).collect();
    (!items.is_empty()).then(|| Section {
        title: Some(title.into()),
        subtitle: Some(items.len().to_string()),
        items,
    })
}

fn item(entry: &Entry) -> Item {
    Item {
        id: entry.chars.to_string(),
        title: title_case(&entry.name),
        prefix: None,
        subtitle: None,
        icon: Some(Icon::Text {
            text: entry.chars.to_string(),
        }),
        accessories: Vec::new(),
        keywords: Vec::new(),
        actions: vec![
            action(PASTE, "Paste"),
            action(COPY, "Copy to Clipboard"),
            action(COPY_UNICODE, "Copy Unicode"),
            action(COPY_NAME, "Copy Name"),
        ],
        detail: None,
    }
}

fn action(id: &str, title: &str) -> Action {
    Action {
        id: id.into(),
        title: title.into(),
        shortcut: None,
        icon: None,
    }
}

/// CLDR's lowercase name as a title: *grinning face* as *Grinning Face*.
fn title_case(name: &str) -> String {
    let mut title = String::with_capacity(name.len());
    let mut start = true;
    for c in name.chars() {
        if start {
            title.extend(c.to_uppercase());
        } else {
            title.push(c);
        }
        start = c == ' ';
    }
    title
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUN: &str = "\u{2600}\u{FE0F}";

    fn list(view: &View) -> &List {
        match view {
            View::List(list) => list,
            View::Detail(_) => panic!("expected a list"),
        }
    }

    fn invoke(session: &mut Session, item: &str, action: &str) -> Vec<Effect> {
        session.event(Event::ActionInvoked {
            item: Some(item.into()),
            action: action.into(),
        })
    }

    fn query(session: &mut Session, text: &str) {
        session.event(Event::QueryChanged { query: text.into() });
    }

    #[test]
    fn the_view_is_a_grid_with_every_category_to_choose_from() {
        let view = Session::new().view();
        let list = list(&view);
        assert_eq!(list.presentation, Presentation::Grid);
        assert_eq!(list.filtering, Filtering::Extension);
        assert_eq!(
            list.placeholder.as_deref(),
            Some("Search Emoji & Symbols\u{2026}")
        );

        let [scope] = &list.scopes[..] else {
            panic!("one scope")
        };
        assert_eq!(scope.id, CATEGORY_SCOPE);
        assert_eq!(scope.selected, ALL_CATEGORIES);
        let titles: Vec<&str> = scope.options.iter().map(|o| o.title.as_str()).collect();
        assert_eq!(titles.len(), 17);
        assert_eq!(titles[0], "All Categories");
        assert_eq!(titles[1], "Smileys & People");
        assert_eq!(titles[16], "Box Drawing");
        assert_eq!(scope.options[9].id, "arrows");
    }

    #[test]
    fn with_nothing_typed_each_category_is_a_section_with_its_count() {
        let view = Session::new().view();
        let sections = &list(&view).sections;
        assert_eq!(sections.len(), Category::ALL.len());
        for (section, category) in sections.iter().zip(Category::ALL) {
            assert_eq!(section.title.as_deref(), Some(category.label()));
            assert_eq!(
                section.subtitle.as_deref(),
                Some(section.items.len().to_string().as_str())
            );
        }
        assert_eq!(list(&view).items().count(), 2_805);
    }

    #[test]
    fn a_search_is_one_section_best_first() {
        let mut session = Session::new();
        query(&mut session, "sun");
        let view = session.view();
        let list = list(&view);
        let [section] = &list.sections[..] else {
            panic!("one section")
        };
        assert_eq!(section.title.as_deref(), Some("Results"));
        assert_eq!(
            section.subtitle.as_deref(),
            Some(section.items.len().to_string().as_str())
        );
        let first = &section.items[0];
        assert_eq!(first.id, SUN);
        assert_eq!(first.title, "Sun");
        assert_eq!(first.icon, Some(Icon::Text { text: SUN.into() }));
    }

    #[test]
    fn the_category_scope_narrows_the_grid() {
        let mut session = Session::new();
        session.event(Event::ScopeChanged {
            scope: CATEGORY_SCOPE.into(),
            option: "arrows".into(),
        });
        let view = session.view();
        let list = list(&view);
        assert_eq!(list.scopes[0].selected, "arrows");
        assert_eq!(list.sections.len(), 1);
        assert_eq!(list.sections[0].title.as_deref(), Some("Arrows"));
        let arrows: Vec<&str> = list.items().map(|item| item.id.as_str()).collect();
        assert!(arrows.contains(&"\u{2192}"));
        assert!(!arrows.contains(&SUN));

        // A search within the category stays within it.
        query(&mut session, "sun");
        let found = list_ids(&session);
        assert!(!found.contains(&SUN.to_string()));
        let catalogue = catalogue();
        assert!(found.iter().all(|id| {
            catalogue
                .entries()
                .iter()
                .any(|entry| &*entry.chars == id && entry.category == Category::Arrows)
        }));

        session.event(Event::ScopeChanged {
            scope: CATEGORY_SCOPE.into(),
            option: ALL_CATEGORIES.into(),
        });
        assert_eq!(list_ids(&session)[0], SUN);
    }

    fn list_ids(session: &Session) -> Vec<String> {
        list(&session.view())
            .items()
            .map(|item| item.id.clone())
            .collect()
    }

    #[test]
    fn enter_pastes_and_the_panel_copies_the_emoji_its_code_and_its_name() {
        let mut session = Session::new();
        query(&mut session, "sun");
        let view = session.view();
        let first = list(&view).items().next().unwrap();
        let actions: Vec<(&str, &str)> = first
            .actions
            .iter()
            .map(|a| (a.id.as_str(), a.title.as_str()))
            .collect();
        assert_eq!(
            actions,
            [
                (PASTE, "Paste"),
                (COPY, "Copy to Clipboard"),
                (COPY_UNICODE, "Copy Unicode"),
                (COPY_NAME, "Copy Name"),
            ]
        );

        assert_eq!(
            invoke(&mut session, SUN, PASTE),
            [Effect::Paste(SUN.into())]
        );
        assert_eq!(invoke(&mut session, SUN, COPY), [Effect::Copy(SUN.into())]);
        assert_eq!(
            invoke(&mut session, SUN, COPY_UNICODE),
            [Effect::Copy("U+2600 U+FE0F".into())]
        );
        assert_eq!(
            invoke(&mut session, SUN, COPY_NAME),
            [Effect::Copy("Sun".into())]
        );
        // Something that is not in the catalogue does nothing.
        assert!(invoke(&mut session, "not an emoji", PASTE).is_empty());
        assert!(invoke(&mut session, SUN, "unknown").is_empty());
    }

    #[test]
    fn a_change_that_changes_nothing_draws_nothing() {
        let mut session = Session::new();
        assert!(matches!(
            session.event(Event::QueryChanged {
                query: "sun".into()
            })[..],
            [Effect::Render(_)]
        ));
        assert!(
            session
                .event(Event::QueryChanged {
                    query: "sun".into()
                })
                .is_empty()
        );
        assert!(
            session
                .event(Event::ScopeChanged {
                    scope: CATEGORY_SCOPE.into(),
                    option: ALL_CATEGORIES.into(),
                })
                .is_empty()
        );
    }

    #[test]
    fn names_are_shown_in_title_case() {
        assert_eq!(title_case("grinning face"), "Grinning Face");
        assert_eq!(title_case("flag: Spain"), "Flag: Spain");
        assert_eq!(title_case("sun"), "Sun");
    }
}
