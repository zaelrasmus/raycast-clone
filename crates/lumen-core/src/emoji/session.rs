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

use super::store::{self, State, join_keywords, parse_keywords};
use super::{Catalogue, Category, Entry, SkinTone, catalogue, codepoints};
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
/// The host's *Paste and Keep Window Open* key runs this: see
/// [`paste_action`](crate::view::paste_action).
pub const PASTE_KEEP_OPEN: &str = crate::view::paste_action::KEEP_OPEN;
pub const COPY: &str = "copy";
pub const COPY_UNICODE: &str = "copy-unicode";
pub const COPY_NAME: &str = "copy-name";
/// The host's favourite keys run these: see
/// [`favorite_action`](crate::view::favorite_action).
pub use crate::view::favorite_action::{MOVE_DOWN, MOVE_UP, PIN, UNPIN};
pub const FORGET: &str = "forget";
/// *Add Custom Keywords*, or *Edit* when the emoji has some: opens the form.
pub const CUSTOM_KEYWORDS: &str = "custom-keywords";
/// Before a [`SkinTone`]'s slug: the action that makes it the tone.
pub const SKIN_TONE: &str = "skin-tone:";

/// The most emoji *Frequently Used* shows: two strips of the default
/// seven-column grid and a little, enough for the handful anyone repeats
/// without pushing the categories below the fold.
const FREQUENT_LIMIT: usize = 16;

/// Before the characters, in the id of an emoji drawn in *Pinned* or
/// *Frequently Used*: it is drawn in its category too, under the bare
/// characters, and ids are unique within a view.
const PINNED_ITEM: &str = "pinned:";
const FREQUENT_ITEM: &str = "frequent:";

/// What the daemon is asked to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Draw this.
    Render(View),
    /// Paste `text` into the window the launcher was opened over: closing
    /// the launcher, or with `keep_open` leaving it as it is, to paste
    /// another.
    Paste { text: String, keep_open: bool },
    /// Put this on the clipboard.
    Copy(String),
    /// Write this to `emoji.json`, off the UI thread.
    Save(State),
    /// Open the custom keywords form over the grid, titled `title`, with
    /// `text` in its field. What is saved comes back to
    /// [`Session::save_keywords`] with `item`; leaving it changes nothing.
    EditKeywords {
        item: String,
        title: String,
        text: String,
    },
}

pub struct Session {
    catalogue: &'static Catalogue,
    /// As typed; the catalogue's search folds it.
    query: String,
    /// `None` is every category.
    category: Option<Category>,
    /// Pins, usage and custom keywords, as `emoji.json` holds them.
    saved: State,
}

impl Default for Session {
    fn default() -> Self {
        Self::new(State::default())
    }
}

impl Session {
    pub fn new(saved: State) -> Self {
        Self {
            catalogue: catalogue(),
            query: String::new(),
            category: None,
            saved,
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
                if let Some(tone) = action.strip_prefix(SKIN_TONE) {
                    return self.choose_tone(&item, entry, tone);
                }
                // Kept by its own characters; shown and pasted in the tone.
                let chars = &*entry.chars;
                let toned = entry.toned(self.saved.skin_tone);
                let done = match action.as_str() {
                    PASTE | PASTE_KEEP_OPEN => Effect::Paste {
                        text: toned.to_string(),
                        keep_open: action == PASTE_KEEP_OPEN,
                    },
                    COPY => Effect::Copy(toned.to_string()),
                    COPY_UNICODE => Effect::Copy(codepoints(toned)),
                    COPY_NAME => Effect::Copy(title_case(&entry.name)),
                    CUSTOM_KEYWORDS => {
                        let keywords = self.saved.keywords.get(chars);
                        return vec![Effect::EditKeywords {
                            title: format!(
                                "Custom Keywords for {chars} {}",
                                title_case(&entry.name)
                            ),
                            text: keywords.map(|k| join_keywords(k)).unwrap_or_default(),
                            item,
                        }];
                    }
                    _ => return self.arrange(&item, chars, &action),
                };
                // Saved first: pasting and copying close the view.
                self.saved.record(chars);
                let mut effects = vec![Effect::Save(self.saved.clone()), done];
                // Still open: drawn again, so Frequently Used has it now.
                if action == PASTE_KEEP_OPEN {
                    effects.push(Effect::Render(self.view()));
                }
                effects
            }
            _ => Vec::new(),
        }
    }

    /// The custom keywords form, saved with `text` for the emoji drawn as
    /// `item`: an empty field takes its keywords away.
    ///
    /// The grid is drawn again whether or not anything changed -- the form
    /// is closing, and the emoji is selected where it is drawn now, under
    /// the query and category it was left with.
    pub fn save_keywords(&mut self, item: &str, text: &str) -> Vec<Effect> {
        let Some(entry) = self.entry(item) else {
            return Vec::new();
        };
        let chars = &*entry.chars;
        let mut effects = Vec::new();
        if self.saved.set_keywords(chars, parse_keywords(text)) {
            effects.push(Effect::Save(self.saved.clone()));
        }
        let mut view = self.view();
        if let View::List(list) = &mut view {
            list.select = where_now(list, item, chars).map(Box::new);
        }
        effects.push(Effect::Render(view));
        effects
    }

    /// Make `slug` the tone every emoji is shown and pasted in, chosen on
    /// `entry` as `item`. Pins, use and keywords are kept by the untoned
    /// characters, so nothing else changes; the grid is drawn again in the
    /// tone with the emoji still selected.
    fn choose_tone(&mut self, item: &str, entry: &Entry, slug: &str) -> Vec<Effect> {
        let Some(tone) = SkinTone::from_slug(slug) else {
            return Vec::new();
        };
        if tone == self.saved.skin_tone {
            return Vec::new();
        }
        self.saved.skin_tone = tone;
        let mut view = self.view();
        if let View::List(list) = &mut view {
            list.select = where_now(list, item, &entry.chars).map(Box::new);
        }
        vec![Effect::Save(self.saved.clone()), Effect::Render(view)]
    }

    /// Pin, unpin, move or forget `chars`, acted on as `item`: saved and
    /// drawn again when anything changed, nothing at all when not.
    ///
    /// The view drawn asks the host to select the emoji where it is now,
    /// since its id may have gone: pinned out of *Frequently Used*, it is
    /// `pinned:` and no longer `frequent:`.
    fn arrange(&mut self, item: &str, chars: &str, action: &str) -> Vec<Effect> {
        let changed = match action {
            PIN => self.saved.pin(chars),
            UNPIN => self.saved.unpin(chars),
            MOVE_UP | MOVE_DOWN => {
                let visible = self.pinned();
                let visible: Vec<&str> = visible.iter().map(|entry| &*entry.chars).collect();
                self.saved.move_pinned(chars, action == MOVE_UP, &visible)
            }
            FORGET => self.saved.forget(chars),
            _ => false,
        };
        if !changed {
            return Vec::new();
        }
        let mut view = self.view();
        if let View::List(list) = &mut view {
            list.select = where_now(list, item, chars).map(Box::new);
        }
        vec![Effect::Save(self.saved.clone()), Effect::Render(view)]
    }

    /// The entry an item stands for: its id is its characters, after the
    /// prefix of the section it was drawn in, if any.
    fn entry(&self, id: &str) -> Option<&'static Entry> {
        let chars = [PINNED_ITEM, FREQUENT_ITEM]
            .iter()
            .find_map(|prefix| id.strip_prefix(prefix))
            .unwrap_or(id);
        self.catalogue
            .entries()
            .iter()
            .find(|entry| &*entry.chars == chars)
    }

    /// The pins the catalogue has, once each, in the user's order. One it
    /// lacks stays in the file, unseen.
    fn pinned(&self) -> Vec<&'static Entry> {
        let mut pinned: Vec<&'static Entry> = Vec::new();
        for entry in self
            .saved
            .pinned
            .iter()
            .filter_map(|chars| self.entry(chars))
        {
            if !pinned.iter().any(|kept| kept.chars == entry.chars) {
                pinned.push(entry);
            }
        }
        pinned
    }

    /// The most used that are not pinned, best first, at most
    /// [`FREQUENT_LIMIT`]. Ties keep the catalogue's order.
    fn frequent(&self) -> Vec<&'static Entry> {
        let now = store::now();
        let mut used: Vec<(f64, &'static Entry)> = self
            .catalogue
            .entries()
            .iter()
            .filter(|entry| self.saved.is_used(&entry.chars) && !self.saved.is_pinned(&entry.chars))
            .map(|entry| (self.saved.score(&entry.chars, now), entry))
            .collect();
        used.sort_by(|a, b| b.0.total_cmp(&a.0));
        used.into_iter()
            .take(FREQUENT_LIMIT)
            .map(|(_, entry)| entry)
            .collect()
    }

    pub fn view(&self) -> View {
        let pinned = self.pinned();
        let pinned: Vec<&str> = pinned.iter().map(|entry| &*entry.chars).collect();
        let items = Items {
            saved: &self.saved,
            pinned: &pinned,
        };
        let sections = if self.query.is_empty() {
            self.browse(&items)
        } else {
            let found = self
                .catalogue
                .search(&self.query, self.category, &self.saved.keywords);
            items.section("Results", "", found).into_iter().collect()
        };

        View::List(List {
            presentation: Presentation::Grid,
            select: None,
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
    /// dropdown's order, which is Unicode's, or the one chosen. Over *All
    /// Categories*, *Pinned* and *Frequently Used* go above them.
    fn browse(&self, items: &Items) -> Vec<Section> {
        let mut sections = Vec::new();
        if self.category.is_none() {
            sections.extend(items.section("Pinned", PINNED_ITEM, self.pinned()));
            sections.extend(items.section("Frequently Used", FREQUENT_ITEM, self.frequent()));
        }
        sections.extend(
            Category::ALL
                .into_iter()
                .filter(|category| self.category.is_none_or(|chosen| chosen == *category))
                .filter_map(|category| {
                    let entries = self
                        .catalogue
                        .entries()
                        .iter()
                        .filter(|entry| entry.category == category);
                    items.section(category.label(), "", entries)
                }),
        );
        sections
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

/// What an item's glyph and actions depend on: the tone, whether it is
/// pinned, where, and used.
struct Items<'a> {
    saved: &'a State,
    /// The pins shown, in order.
    pinned: &'a [&'a str],
}

impl Items<'_> {
    /// A titled run of entries with its count, or nothing when it is empty:
    /// a header over no cells says nothing. `prefix` keeps ids unique when an
    /// emoji is drawn twice, pinned or used and in its category.
    fn section<'e>(
        &self,
        title: &str,
        prefix: &str,
        entries: impl IntoIterator<Item = &'e Entry>,
    ) -> Option<Section> {
        let items: Vec<Item> = entries
            .into_iter()
            .map(|entry| self.item(entry, prefix))
            .collect();
        (!items.is_empty()).then(|| Section {
            title: Some(title.into()),
            subtitle: Some(items.len().to_string()),
            items,
        })
    }

    fn item(&self, entry: &Entry, prefix: &str) -> Item {
        let chars = &*entry.chars;
        let mut actions = vec![
            action(PASTE, "Paste"),
            action(PASTE_KEEP_OPEN, "Paste and Keep Window Open"),
            action(COPY, "Copy to Clipboard"),
            action(COPY_UNICODE, "Copy Unicode"),
            action(COPY_NAME, "Copy Name"),
        ];
        match self.pinned.iter().position(|pinned| *pinned == chars) {
            Some(at) => {
                actions.push(action(UNPIN, "Unpin Emoji"));
                if at > 0 {
                    actions.push(action(MOVE_UP, "Move Up in Pinned"));
                }
                if at + 1 < self.pinned.len() {
                    actions.push(action(MOVE_DOWN, "Move Down in Pinned"));
                }
            }
            None => actions.push(action(PIN, "Pin Emoji")),
        }
        if self.saved.is_used(chars) {
            actions.push(action(FORGET, "Remove from Frequently Used"));
        }
        actions.push(action(
            CUSTOM_KEYWORDS,
            if self.saved.keywords.contains_key(chars) {
                "Edit Custom Keywords"
            } else {
                "Add Custom Keywords"
            },
        ));
        // The tones it can be shown in other than the current one, which the
        // grid already shows: a view's actions are one run (ARCHITECTURE
        // 7.44), so these go last, each with the raised hand as its sample.
        if entry.has_tones() {
            actions.extend(
                SkinTone::ALL
                    .into_iter()
                    .filter(|tone| *tone != self.saved.skin_tone)
                    .map(|tone| {
                        let sample: String =
                            std::iter::once('\u{270B}').chain(tone.modifier()).collect();
                        action(
                            &format!("{SKIN_TONE}{}", tone.slug()),
                            &format!("{sample} {}", tone.label()),
                        )
                    }),
            );
        }
        Item {
            id: format!("{prefix}{chars}"),
            title: title_case(&entry.name),
            prefix: None,
            subtitle: None,
            icon: Some(Icon::Text {
                text: entry.toned(self.saved.skin_tone).to_string(),
            }),
            accessories: Vec::new(),
            keywords: Vec::new(),
            actions,
            detail: None,
        }
    }
}

/// The id `chars` has in `list` after a change made to it as `item`: in
/// the section it was acted on, if it is still there, else pinned, else
/// frequently used, else in its category or the results.
fn where_now(list: &List, item: &str, chars: &str) -> Option<String> {
    let acted_in = [PINNED_ITEM, FREQUENT_ITEM]
        .into_iter()
        .find(|prefix| item.starts_with(prefix))
        .unwrap_or("");
    [acted_in, PINNED_ITEM, FREQUENT_ITEM, ""]
        .into_iter()
        .map(|prefix| format!("{prefix}{chars}"))
        .find(|id| list.items().any(|item| item.id == *id))
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

    /// What *Paste* asks for: the launcher closes.
    fn pasted(text: &str) -> Effect {
        Effect::Paste {
            text: text.into(),
            keep_open: false,
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
        let view = Session::default().view();
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
        let view = Session::default().view();
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
        let mut session = Session::default();
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
        let mut session = Session::default();
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
        let mut session = Session::default();
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
                (PASTE_KEEP_OPEN, "Paste and Keep Window Open"),
                (COPY, "Copy to Clipboard"),
                (COPY_UNICODE, "Copy Unicode"),
                (COPY_NAME, "Copy Name"),
                (PIN, "Pin Emoji"),
                (CUSTOM_KEYWORDS, "Add Custom Keywords"),
            ]
        );

        // Each after the save that records the use.
        let mut last = |action: &str| invoke(&mut session, SUN, action).pop();
        assert_eq!(last(PASTE), Some(pasted(SUN)));
        assert_eq!(last(COPY), Some(Effect::Copy(SUN.into())));
        assert_eq!(
            last(COPY_UNICODE),
            Some(Effect::Copy("U+2600 U+FE0F".into()))
        );
        assert_eq!(last(COPY_NAME), Some(Effect::Copy("Sun".into())));
        // Something that is not in the catalogue does nothing.
        assert!(invoke(&mut session, "not an emoji", PASTE).is_empty());
        assert!(invoke(&mut session, SUN, "unknown").is_empty());
    }

    #[test]
    fn a_change_that_changes_nothing_draws_nothing() {
        let mut session = Session::default();
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

    // Pinned and Frequently Used (F18, task 6).

    const GRIN: &str = "\u{1F600}";

    fn titles(session: &Session) -> Vec<String> {
        list(&session.view())
            .sections
            .iter()
            .map(|section| section.title.clone().unwrap_or_default())
            .collect()
    }

    fn section_ids(session: &Session, title: &str) -> Vec<String> {
        list(&session.view())
            .sections
            .iter()
            .find(|section| section.title.as_deref() == Some(title))
            .map(|section| section.items.iter().map(|item| item.id.clone()).collect())
            .unwrap_or_default()
    }

    fn action_ids(session: &Session, id: &str) -> Vec<String> {
        list(&session.view())
            .items()
            .find(|item| item.id == id)
            .unwrap_or_else(|| panic!("no item {id}"))
            .actions
            .iter()
            .map(|action| action.id.clone())
            .collect()
    }

    fn saved(effects: &[Effect]) -> State {
        effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Save(state) => Some(state.clone()),
                _ => None,
            })
            .expect("a save")
    }

    fn the_first_emoji(count: usize) -> Vec<String> {
        catalogue()
            .entries()
            .iter()
            .take(count)
            .map(|entry| entry.chars.to_string())
            .collect()
    }

    #[test]
    fn an_empty_store_shows_only_the_categories() {
        let labels: Vec<String> = Category::ALL
            .iter()
            .map(|category| category.label().to_string())
            .collect();
        assert_eq!(titles(&Session::default()), labels);
    }

    #[test]
    fn a_pasted_emoji_is_frequently_used_when_the_view_opens_again() {
        let dir = std::env::temp_dir().join(format!("lumen-emoji-session-{}", std::process::id()));
        let path = dir.join("emoji.json");
        let mut session = Session::new(State::load(&path));

        let effects = invoke(&mut session, SUN, PASTE);
        assert_eq!(effects.last(), Some(&pasted(SUN)));
        assert!(
            matches!(effects[0], Effect::Save(_)),
            "saved before closing"
        );
        saved(&effects).save(&path).unwrap();

        let session = Session::new(State::load(&path));
        assert_eq!(titles(&session)[0], "Frequently Used");
        let view = session.view();
        let frequent = &list(&view).sections[0];
        assert_eq!(frequent.subtitle.as_deref(), Some("1"));
        assert_eq!(frequent.items[0].id, format!("frequent:{SUN}"));
        assert_eq!(frequent.items[0].title, "Sun");
        // Still in its category, under its own id.
        assert!(list(&view).items().any(|item| item.id == SUN));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// *Paste and Keep Window Open* pastes the same emoji as *Paste*, keeps
    /// the launcher open, counts as a use, and draws the grid again so
    /// *Frequently Used* shows it at once.
    #[test]
    fn paste_and_keep_window_open_counts_the_use_and_redraws() {
        let mut session = in_tone(SkinTone::Medium);

        let effects = invoke(&mut session, THUMBS, PASTE_KEEP_OPEN);
        let [Effect::Save(state), paste, Effect::Render(view)] = &effects[..] else {
            panic!("expected a save, a paste and a render, got {effects:?}");
        };
        assert_eq!(state.usage[THUMBS].count, 1);
        assert_eq!(
            *paste,
            Effect::Paste {
                text: THUMBS_MEDIUM.into(),
                keep_open: true,
            }
        );
        assert_eq!(
            list(view).sections[0].title.as_deref(),
            Some("Frequently Used")
        );
        // Still in its category, under its own id, which keeps it selected.
        assert!(list(view).items().any(|item| item.id == THUMBS));

        // A second one adds to the count.
        invoke(&mut session, THUMBS, PASTE_KEEP_OPEN);
        assert_eq!(session.saved.usage[THUMBS].count, 2);
    }

    #[test]
    fn every_copy_counts_as_a_use_and_the_most_used_come_first() {
        let mut session = Session::default();
        for action in [COPY, COPY_UNICODE, COPY_NAME] {
            assert!(matches!(
                invoke(&mut session, GRIN, action)[..],
                [Effect::Save(_), Effect::Copy(_)]
            ));
        }
        invoke(&mut session, SUN, COPY);
        assert_eq!(session.saved.usage[GRIN].count, 3);
        assert_eq!(
            section_ids(&session, "Frequently Used"),
            [format!("frequent:{GRIN}"), format!("frequent:{SUN}")]
        );
    }

    #[test]
    fn frequently_used_is_bounded() {
        let mut session = Session::default();
        for chars in the_first_emoji(FREQUENT_LIMIT + 4) {
            invoke(&mut session, &chars, COPY);
        }
        assert_eq!(
            section_ids(&session, "Frequently Used").len(),
            FREQUENT_LIMIT
        );
    }

    #[test]
    fn pinning_moves_an_emoji_from_frequently_used_to_pinned() {
        let mut session = Session::default();
        invoke(&mut session, SUN, PASTE);
        let frequent = format!("frequent:{SUN}");
        assert!(action_ids(&session, &frequent).contains(&PIN.to_string()));

        // From its prefixed id.
        let effects = invoke(&mut session, &frequent, PIN);
        assert_eq!(saved(&effects).pinned, [SUN]);
        assert!(matches!(effects.last(), Some(Effect::Render(_))));
        assert_eq!(titles(&session)[0], "Pinned");
        assert_eq!(titles(&session)[1], Category::ALL[0].label());
        assert_eq!(section_ids(&session, "Pinned"), [format!("pinned:{SUN}")]);

        let actions = action_ids(&session, &format!("pinned:{SUN}"));
        assert_eq!(
            &actions[..5],
            [PASTE, PASTE_KEEP_OPEN, COPY, COPY_UNICODE, COPY_NAME]
        );
        assert!(actions.contains(&UNPIN.to_string()));
        assert!(!actions.contains(&PIN.to_string()));
        // Used, so it can be forgotten; alone, so it cannot move.
        assert!(actions.contains(&FORGET.to_string()));
        assert!(!actions.contains(&MOVE_UP.to_string()));
        assert!(!actions.contains(&MOVE_DOWN.to_string()));
        // The same panel from its category.
        assert_eq!(action_ids(&session, SUN), actions);
    }

    #[test]
    fn pins_keep_the_users_order_and_move_up_and_down() {
        let mut session = Session::default();
        let [a, b, c] = <[String; 3]>::try_from(the_first_emoji(3)).unwrap();
        for chars in [&a, &b, &c] {
            invoke(&mut session, chars, PIN);
        }
        let pinned = |chars: &str| format!("pinned:{chars}");
        assert_eq!(
            section_ids(&session, "Pinned"),
            [pinned(&a), pinned(&b), pinned(&c)]
        );
        let moves = |session: &Session, id: &str| -> Vec<String> {
            action_ids(session, id)
                .into_iter()
                .filter(|id| id == MOVE_UP || id == MOVE_DOWN)
                .collect()
        };
        assert_eq!(moves(&session, &pinned(&a)), [MOVE_DOWN]);
        assert_eq!(moves(&session, &pinned(&b)), [MOVE_UP, MOVE_DOWN]);
        assert_eq!(moves(&session, &pinned(&c)), [MOVE_UP]);

        let effects = invoke(&mut session, &pinned(&c), MOVE_UP);
        assert_eq!(saved(&effects).pinned, [a.clone(), c.clone(), b.clone()]);
        invoke(&mut session, &a, MOVE_DOWN);
        assert_eq!(
            section_ids(&session, "Pinned"),
            [pinned(&c), pinned(&a), pinned(&b)]
        );
        // At the top, up does nothing and writes nothing.
        assert!(invoke(&mut session, &pinned(&c), MOVE_UP).is_empty());
    }

    #[test]
    fn unpinning_puts_a_used_emoji_back_in_frequently_used() {
        let mut session = Session::default();
        invoke(&mut session, SUN, COPY);
        invoke(&mut session, SUN, PIN);
        assert!(section_ids(&session, "Frequently Used").is_empty());

        let effects = invoke(&mut session, &format!("pinned:{SUN}"), UNPIN);
        assert!(saved(&effects).pinned.is_empty());
        assert!(!titles(&session).contains(&"Pinned".to_string()));
        assert_eq!(
            section_ids(&session, "Frequently Used"),
            [format!("frequent:{SUN}")]
        );
        assert!(invoke(&mut session, SUN, UNPIN).is_empty());
    }

    #[test]
    fn an_emoji_can_be_removed_from_frequently_used() {
        let mut session = Session::default();
        assert!(!action_ids(&session, SUN).contains(&FORGET.to_string()));
        invoke(&mut session, SUN, COPY);
        let effects = invoke(&mut session, &format!("frequent:{SUN}"), FORGET);
        assert!(saved(&effects).usage.is_empty());
        assert!(!titles(&session).contains(&"Frequently Used".to_string()));
        assert!(!action_ids(&session, SUN).contains(&FORGET.to_string()));
    }

    /// The item the render after `effects` asks the host to select.
    fn selected(effects: &[Effect]) -> Option<String> {
        effects.iter().find_map(|effect| match effect {
            Effect::Render(view) => list(view).select.as_deref().cloned(),
            _ => None,
        })
    }

    #[test]
    fn the_selection_follows_an_emoji_to_where_it_is_drawn_now() {
        let mut session = Session::default();
        invoke(&mut session, SUN, COPY);
        let effects = invoke(&mut session, &format!("frequent:{SUN}"), PIN);
        assert_eq!(selected(&effects), Some(format!("pinned:{SUN}")));

        // Used, so it goes back to Frequently Used.
        let effects = invoke(&mut session, &format!("pinned:{SUN}"), UNPIN);
        assert_eq!(selected(&effects), Some(format!("frequent:{SUN}")));

        let effects = invoke(&mut session, &format!("frequent:{SUN}"), FORGET);
        assert_eq!(selected(&effects), Some(SUN.to_string()));

        // Never used, so only its category is left.
        invoke(&mut session, GRIN, PIN);
        let effects = invoke(&mut session, &format!("pinned:{GRIN}"), UNPIN);
        assert_eq!(selected(&effects), Some(GRIN.to_string()));
    }

    #[test]
    fn a_moved_or_category_pin_stays_where_it_was_acted_on() {
        let mut session = Session::default();
        invoke(&mut session, GRIN, PIN);
        let effects = invoke(&mut session, SUN, PIN);
        assert_eq!(selected(&effects), Some(SUN.to_string()));
        let effects = invoke(&mut session, &format!("pinned:{SUN}"), MOVE_UP);
        assert_eq!(selected(&effects), Some(format!("pinned:{SUN}")));
    }

    #[test]
    fn only_a_rearrangement_asks_for_a_selection() {
        let mut session = Session::default();
        invoke(&mut session, SUN, PIN);
        assert_eq!(list(&session.view()).select, None);
        let effects = session.event(Event::QueryChanged {
            query: "sun".into(),
        });
        assert_eq!(selected(&effects), None);
    }

    #[test]
    fn ids_are_unique_across_sections() {
        let mut session = Session::default();
        for chars in the_first_emoji(5) {
            invoke(&mut session, &chars, COPY);
        }
        invoke(&mut session, SUN, PIN);
        invoke(&mut session, GRIN, PIN);
        let ids = list_ids(&session);
        let unique: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
        // The first emoji is the grin: two pinned, four used and not pinned.
        assert_eq!(the_first_emoji(1), [GRIN]);
        assert_eq!(ids.len(), 2_805 + 2 + 4);
    }

    #[test]
    fn a_query_or_a_category_hides_pinned_and_frequently_used() {
        let mut session = Session::default();
        invoke(&mut session, SUN, COPY);
        invoke(&mut session, GRIN, PIN);
        query(&mut session, "sun");
        assert_eq!(titles(&session), ["Results"]);
        assert!(list_ids(&session).iter().all(|id| !id.contains(':')));

        query(&mut session, "");
        assert_eq!(titles(&session)[..2], ["Pinned", "Frequently Used"]);
        session.event(Event::ScopeChanged {
            scope: CATEGORY_SCOPE.into(),
            option: Category::ALL[0].slug().into(),
        });
        assert_eq!(titles(&session), [Category::ALL[0].label()]);
    }

    // Custom keywords (F18, task 7).

    fn action_title(session: &Session, id: &str, action: &str) -> Option<String> {
        list(&session.view())
            .items()
            .find(|item| item.id == id)?
            .actions
            .iter()
            .find(|each| each.id == action)
            .map(|each| each.title.clone())
    }

    fn edit(session: &mut Session, item: &str) -> (String, String, String) {
        match &invoke(session, item, CUSTOM_KEYWORDS)[..] {
            [Effect::EditKeywords { item, title, text }] => {
                (item.clone(), title.clone(), text.clone())
            }
            other => panic!("expected the form, got {other:?}"),
        }
    }

    #[test]
    fn the_keywords_action_adds_then_edits() {
        let mut session = Session::default();
        assert_eq!(
            action_title(&session, SUN, CUSTOM_KEYWORDS).as_deref(),
            Some("Add Custom Keywords")
        );
        assert_eq!(
            edit(&mut session, SUN),
            (
                SUN.into(),
                format!("Custom Keywords for {SUN} Sun"),
                String::new()
            )
        );

        session.save_keywords(SUN, "Sunny, warm , ,SUNNY");
        assert_eq!(
            action_title(&session, SUN, CUSTOM_KEYWORDS).as_deref(),
            Some("Edit Custom Keywords")
        );
        let (_, _, text) = edit(&mut session, SUN);
        assert_eq!(text, "sunny, warm");
    }

    #[test]
    fn saving_keywords_stores_them_cleaned_and_empty_removes_them() {
        let mut session = Session::default();
        let effects = session.save_keywords(SUN, "Sunny, warm , ,SUNNY");
        assert_eq!(saved(&effects).keywords[SUN], ["sunny", "warm"]);
        // The same again writes nothing, but still draws the grid.
        let again = session.save_keywords(SUN, "sunny,warm");
        assert!(again.iter().all(|e| !matches!(e, Effect::Save(_))));
        assert!(matches!(again[..], [Effect::Render(_)]));

        let effects = session.save_keywords(SUN, "  ");
        assert!(saved(&effects).keywords.is_empty());
        assert_eq!(
            action_title(&session, SUN, CUSTOM_KEYWORDS).as_deref(),
            Some("Add Custom Keywords")
        );
        // Not an emoji: nothing.
        assert!(session.save_keywords("not an emoji", "x").is_empty());
    }

    #[test]
    fn a_custom_keyword_finds_its_emoji_first() {
        let mut session = Session::default();
        session.save_keywords(SUN, "Sunny");
        query(&mut session, "sunny");
        assert_eq!(list_ids(&session)[0], SUN);

        // From the file, in a new session.
        let session = Session::new(session.saved.clone());
        let mut session = session;
        query(&mut session, "sunny");
        assert_eq!(list_ids(&session)[0], SUN);
    }

    #[test]
    fn after_saving_the_grid_keeps_its_query_and_selects_the_emoji() {
        let mut session = Session::default();
        invoke(&mut session, SUN, PIN);
        query(&mut session, "sun");
        let effects = session.save_keywords(SUN, "sunny");
        assert_eq!(selected(&effects), Some(SUN.to_string()));
        assert_eq!(titles(&session), ["Results"]);

        query(&mut session, "");
        let pinned = format!("pinned:{SUN}");
        let (item, _, _) = edit(&mut session, &pinned);
        assert_eq!(item, pinned);
        let effects = session.save_keywords(&pinned, "");
        assert_eq!(selected(&effects), Some(pinned));
    }

    // Skin tone (F18, task 8).

    const THUMBS: &str = "\u{1F44D}";
    const THUMBS_MEDIUM: &str = "\u{1F44D}\u{1F3FD}";

    fn in_tone(tone: SkinTone) -> Session {
        Session::new(State {
            skin_tone: tone,
            ..State::default()
        })
    }

    fn item(session: &Session, id: &str) -> Item {
        list(&session.view())
            .items()
            .find(|item| item.id == id)
            .unwrap_or_else(|| panic!("no item {id}"))
            .clone()
    }

    fn glyph(session: &Session, id: &str) -> String {
        match item(session, id).icon {
            Some(Icon::Text { text }) => text,
            other => panic!("expected a glyph, got {other:?}"),
        }
    }

    fn tone_actions(session: &Session, id: &str) -> Vec<(String, String)> {
        item(session, id)
            .actions
            .into_iter()
            .filter(|action| action.id.starts_with(SKIN_TONE))
            .map(|action| (action.id, action.title))
            .collect()
    }

    #[test]
    fn in_a_tone_an_emoji_is_shown_pasted_and_copied_toned_and_kept_by_its_own_id() {
        let mut session = in_tone(SkinTone::Medium);
        query(&mut session, "thumbs up");
        let thumbs = item(&session, THUMBS);
        assert_eq!(thumbs.title, "Thumbs Up");
        assert_eq!(glyph(&session, THUMBS), THUMBS_MEDIUM);

        let mut last = |action: &str| invoke(&mut session, THUMBS, action).pop();
        assert_eq!(last(PASTE), Some(pasted(THUMBS_MEDIUM)));
        assert_eq!(last(COPY), Some(Effect::Copy(THUMBS_MEDIUM.into())));
        assert_eq!(
            last(COPY_UNICODE),
            Some(Effect::Copy("U+1F44D U+1F3FD".into()))
        );
        assert_eq!(last(COPY_NAME), Some(Effect::Copy("Thumbs Up".into())));
        // Its use is the untoned emoji's.
        assert_eq!(session.saved.usage[THUMBS].count, 4);
        assert_eq!(session.saved.usage.len(), 1);
    }

    #[test]
    fn an_emoji_without_tones_is_the_same_in_any_tone() {
        let mut session = in_tone(SkinTone::Dark);
        query(&mut session, "sun");
        assert_eq!(glyph(&session, SUN), SUN);
        assert!(tone_actions(&session, SUN).is_empty());
        assert_eq!(invoke(&mut session, SUN, PASTE).pop(), Some(pasted(SUN)));
    }

    #[test]
    fn an_emoji_that_takes_a_tone_offers_the_others() {
        let session = Session::default();
        assert_eq!(
            tone_actions(&session, THUMBS),
            [
                ("skin-tone:light", "\u{270B}\u{1F3FB} Light"),
                ("skin-tone:medium-light", "\u{270B}\u{1F3FC} Medium-Light"),
                ("skin-tone:medium", "\u{270B}\u{1F3FD} Medium"),
                ("skin-tone:medium-dark", "\u{270B}\u{1F3FE} Medium-Dark"),
                ("skin-tone:dark", "\u{270B}\u{1F3FF} Dark"),
            ]
            .map(|(id, title)| (id.to_string(), title.to_string()))
        );
        // Last, after everything the emoji itself does.
        let actions = action_ids(&session, THUMBS);
        assert!(actions.last().unwrap().starts_with(SKIN_TONE));

        let toned = tone_actions(&in_tone(SkinTone::Medium), THUMBS);
        assert_eq!(toned.len(), 5);
        assert_eq!(
            toned[0],
            (
                "skin-tone:default".to_string(),
                "\u{270B} Default".to_string()
            )
        );
        assert!(toned.iter().all(|(id, _)| id != "skin-tone:medium"));
    }

    #[test]
    fn choosing_a_tone_redraws_every_emoji_in_it_and_keeps_pins_use_and_selection() {
        let mut session = Session::default();
        invoke(&mut session, THUMBS, PIN);
        invoke(&mut session, SUN, COPY);
        let pinned = format!("pinned:{THUMBS}");
        assert_eq!(glyph(&session, &pinned), THUMBS);

        let effects = invoke(&mut session, &pinned, "skin-tone:medium");
        let state = saved(&effects);
        assert_eq!(state.skin_tone, SkinTone::Medium);
        assert_eq!(state.pinned, [THUMBS]);
        assert!(state.is_used(SUN));
        assert_eq!(selected(&effects), Some(pinned.clone()));
        let Some(Effect::Render(view)) = effects.last() else {
            panic!("drawn again")
        };
        let drawn = |id: &str| {
            list(view)
                .items()
                .find(|item| item.id == id)
                .and_then(|item| item.icon.clone())
        };
        let text = |text: &str| Some(Icon::Text { text: text.into() });
        assert_eq!(drawn(&pinned), text(THUMBS_MEDIUM));
        assert_eq!(drawn(THUMBS), text(THUMBS_MEDIUM));
        assert_eq!(drawn(SUN), text(SUN));

        // From its category, the selection stays there.
        let effects = invoke(&mut session, THUMBS, "skin-tone:default");
        assert_eq!(selected(&effects), Some(THUMBS.to_string()));
        assert_eq!(saved(&effects).skin_tone, SkinTone::Default);
        // The tone it already has, or one that does not exist: nothing.
        assert!(invoke(&mut session, THUMBS, "skin-tone:default").is_empty());
        assert!(invoke(&mut session, THUMBS, "skin-tone:purple").is_empty());
    }

    #[test]
    fn a_toned_emoji_typed_finds_the_emoji() {
        let mut session = in_tone(SkinTone::Light);
        query(&mut session, THUMBS_MEDIUM);
        assert_eq!(list_ids(&session)[0], THUMBS);
    }

    #[test]
    fn a_corrupt_file_starts_empty() {
        let dir = std::env::temp_dir().join(format!("lumen-emoji-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("emoji.json");
        std::fs::write(&path, "{ \"pinned\": 3").unwrap();
        let session = Session::new(State::load(&path));
        assert_eq!(titles(&session).len(), Category::ALL.len());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
