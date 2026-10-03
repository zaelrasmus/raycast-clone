//! The UI vocabulary an extension may draw with (F4, AD-06, AD-07, I-13).
//!
//! # This file is the contract
//!
//! AD-06: "The type contract is defined once and is transport-independent. UI
//! tree and event types are Rust structs with `serde`; WIT and JSON Schema are
//! *derived*. The contract is the durable artefact, the transport is
//! reversible."
//!
//! So this is the only place the tree and the events exist (I-13). Nothing here
//! knows what a WebAssembly module is, how a message reaches it, or whether
//! there is a process on the other end at all. When a `.wit` file appears it is
//! generated from these types; if the transport is replaced the contract does
//! not move.
//!
//! # Why the vocabulary is closed
//!
//! AD-07: "Extensions never draw pixels; they emit a declarative tree over
//! fixed primitives. Consistency by construction, not by third-party
//! discipline."
//!
//! There is no `Box`, no `Style`, no colour and no size anywhere in these
//! types. An extension chooses *what* to show, never how it looks. That is the
//! same reasoning as I-11 for themes, one layer further out: the calibrated
//! layout the whole project exists to protect does not survive a third party
//! being able to set a margin.
//!
//! It is also why growth here is deliberately awkward. A new variant is a new
//! thing every renderer, every theme and every extension has to agree about, so
//! the bar for adding one is an interaction that genuinely cannot be expressed
//! with what is already here.
//!
//! # Where this is in the build order
//!
//! The plan is explicit that F4's order is not optional:
//!
//! 1. Hand-write the JSON of a `List` tree with ten items and an action panel.
//!    Render it in GPUI from a static file. No protocol, no processes.
//! 2. Wrap that behind `render_node(node) -> AnyElement`, with list
//!    virtualisation.
//! 3. Only then move the tree across a channel and introduce events.
//! 4. Write the *second* extension before stabilising anything in the protocol.
//!
//! This file is step 1's half of the work. [`Event`] is written down here
//! because the contract is one artefact and splitting it across two commits is
//! how two artefacts start -- but nothing sends one yet, and the shape is not
//! stable until step 4 has happened.

use serde::{Deserialize, Serialize};

/// Everything an extension may put on screen.
///
/// Two variants, and the restraint is the point: a list of things, and one
/// thing shown in detail. Every command worth writing is one of those two, and
/// a third variant would have to earn its place against the cost of every
/// renderer having to handle it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum View {
    List(List),
    Detail(Detail),
}

impl View {
    /// What the search field shows when it is empty.
    pub fn placeholder(&self) -> Option<&str> {
        match self {
            View::List(list) => list.placeholder.as_deref(),
            View::Detail(_) => None,
        }
    }

    /// The actions for the current selection, or the view's own.
    pub fn actions(&self, selected: usize) -> &[Action] {
        match self {
            View::List(list) => list
                .item(selected)
                .map(|item| item.actions.as_slice())
                .unwrap_or_default(),
            View::Detail(detail) => &detail.actions,
        }
    }
}

/// A list of items, optionally in sections.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct List {
    /// Shown in the search field when it is empty. `None` uses the host's.
    pub placeholder: Option<String>,
    /// Whether the host filters as the user types, or the extension does.
    ///
    /// The default is [`Filtering::Host`], and it is the default because it is
    /// the one that cannot stutter: filtering in the host is the same matcher
    /// the root list uses, running in the same frame, with nothing to wait for.
    /// [`Filtering::Extension`] exists for the cases that genuinely need it --
    /// a search that hits a network, or a result set too large to send -- and
    /// it is the caller's problem that it is slower.
    pub filtering: Filtering,
    /// Choices that apply to the whole list, shown beside the search field,
    /// in order.
    pub scopes: Vec<Scope>,
    /// How far into the list's own hierarchy the user has gone, outermost
    /// first: `["Europe", "Spain"]` (F13). Drawn as one pill where the
    /// text starts, so what is typed reads as a search within it.
    ///
    /// The view owns the levels; the host only draws them and says when
    /// one is taken off, with [`Event::Back`] -- `Escape` or `Backspace` in
    /// an empty field, as for the root list's argument pill (F6). Empty, the
    /// default, draws nothing and changes nothing.
    pub path: Vec<String>,
    /// Shown instead of the list when it is empty.
    pub empty: Option<EmptyState>,
    /// Show the selected item's [`Item::detail`] in a pane beside the list
    /// (F10).
    ///
    /// A property of the list, not of each item, so the pane is there or not
    /// for the whole view: an item without a detail leaves it blank rather
    /// than taking it away, and moving the selection never changes the
    /// layout (I-10). The rows keep their height and narrow to the list's
    /// share of the width.
    pub showing_detail: bool,
    pub sections: Vec<Section>,
}

/// One of a few ways to read the query, chosen beside the search field.
///
/// Raycast's search-bar dropdown, and here for the interaction nothing else
/// can express: changing what the *whole* list means -- which account, which
/// language -- without leaving it and without losing what was typed. An action
/// cannot do it, because actions belong to a row and an empty list has none.
///
/// The host draws each scope as a pill naming its selected option, and opens
/// a picker of the options on its own keys (I-14) or a click. The view is told
/// with [`Event::ScopeChanged`] and answers with a new render. A view that
/// declares no scope is never sent that event, so adding this changed nothing
/// for the extensions written before it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Scope {
    /// Sent back with the option chosen. Opaque to the host.
    pub id: String,
    /// What the choice is about, shown before the selected option: "From".
    pub title: String,
    pub options: Vec<ScopeOption>,
    /// The `id` of the option in force.
    pub selected: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ScopeOption {
    /// Sent back when this option is chosen. Opaque to the host.
    pub id: String,
    pub title: String,
    /// Also matched when the picker is searched, never shown (F9): the
    /// countries that use a currency, so `japan` finds the yen.
    pub keywords: Vec<String>,
}

impl Scope {
    /// The option in force, if `selected` names one.
    pub fn current(&self) -> Option<&ScopeOption> {
        self.options
            .iter()
            .find(|option| option.id == self.selected)
    }

    /// Where the option in force is, or the first when `selected` names none.
    pub fn position(&self) -> usize {
        self.options
            .iter()
            .position(|option| option.id == self.selected)
            .unwrap_or(0)
    }
}

impl List {
    /// Every item, in display order, ignoring section boundaries.
    ///
    /// The selection is an index into this, not a `(section, item)` pair:
    /// pressing down at the end of a section moves to the next one, so a flat
    /// index is what the interaction actually is.
    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.sections.iter().flat_map(|section| &section.items)
    }

    pub fn item(&self, index: usize) -> Option<&Item> {
        self.items().nth(index)
    }

    pub fn len(&self) -> usize {
        self.sections
            .iter()
            .map(|section| section.items.len())
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Who narrows the list as the user types.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Filtering {
    #[default]
    Host,
    Extension,
}

/// A titled group of items.
///
/// Sections are the only structure a list has. They cannot nest, because a
/// nested section is a tree, and a tree in a keyboard-driven list is a
/// navigation model nobody wants to learn.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Section {
    pub title: Option<String>,
    /// Drawn dimmer, after the title, in the same header (F9): the date a
    /// table of rates was published, say. Ignored without a title, since a
    /// header row appears only for one.
    pub subtitle: Option<String>,
    pub items: Vec<Item>,
}

/// One row.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Item {
    /// The extension's own identifier, sent back with every event about this
    /// row. Opaque to the host: it is never parsed, compared for ordering, or
    /// shown.
    pub id: String,
    pub title: String,
    /// Drawn dimmed before the title, as one line with it (F16): what tells
    /// two rows of the same title apart, such as `styles/` before
    /// `carbon.json`. Empty, the default, draws nothing.
    pub prefix: Option<String>,
    pub subtitle: Option<String>,
    pub icon: Option<Icon>,
    /// Shown right-aligned, in order. Bounded because a row is one line and
    /// three accessories is already more than it can hold legibly.
    pub accessories: Vec<Accessory>,
    /// Matched against when [`Filtering::Host`] is in force, never displayed.
    pub keywords: Vec<String>,
    /// Most important first. The first is what `Enter` runs.
    pub actions: Vec<Action>,
    /// What the pane shows while this item is selected, when the list is
    /// [`showing_detail`](List::showing_detail). Ignored otherwise.
    ///
    /// Carried by the item rather than sent when the selection moves: the
    /// pane follows the selection in the same frame, with nothing to wait
    /// for, which is what a row one line tall needs to be readable at all.
    pub detail: Option<ItemDetail>,
}

/// An item's detail, shown in the pane beside the list (F10).
///
/// A [`Detail`] without actions: in a list the actions are the selected
/// row's, and a second set in the pane would be two answers to what `Enter`
/// does.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ItemDetail {
    /// The body, in the same restricted Markdown as [`Detail::markdown`].
    pub markdown: String,
    /// Text shown exactly as written, after the Markdown (F11): each line
    /// break kept and nothing read as a marker.
    ///
    /// A snippet is the case: `a * b` is a command, not emphasis, and a
    /// signature's lines are lines, where Markdown would join them into one
    /// paragraph. The subset has no escapes and no hard breaks, and adding
    /// both would still leave the view escaping text to have it unescaped.
    pub text: Option<String>,
    /// Text shown as written in a card, after the Markdown and in place of
    /// [`text`](Self::text)'s plain run (F16): a file's first lines. Drawn
    /// in the palette's monospace family with its lines clipped, not
    /// wrapped, so columns and indentation survive -- unless the user
    /// prefers the interface family for previews, which wraps them. The
    /// card is as tall as its lines up to a bound, so what is under it is
    /// always within reach.
    pub code: Option<String>,
    /// A picture the host wrote, shown first, scaled to fit (F12). The
    /// host's alone, like [`Icon::Image`], and for the same reason.
    #[serde(skip)]
    pub image: Option<std::path::PathBuf>,
    /// Labelled values shown under the body.
    pub metadata: Vec<MetadataRow>,
}

/// The most accessories a row will draw.
///
/// Anything past this is dropped rather than squeezed, because a row that
/// shrinks its text to fit an accessory is a row that no longer lines up with
/// the one above it (I-10).
pub const MAX_ACCESSORIES: usize = 3;

/// A small piece of right-aligned metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Accessory {
    /// Plain text: a date, a count, a branch name.
    Text {
        text: String,
        /// What the row exists to show, drawn as prominently as the title
        /// rather than dimmed (F13): the time in a city, the difference
        /// from here. A meaning, not a style, as a [`Tone`] is. Omitted, it
        /// is plain, so views written before it are unchanged.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        strong: bool,
    },
    /// Text in a pill, for a status with a meaning attached.
    ///
    /// The tone is semantic, never a colour. An extension that could name a
    /// colour would be an extension that clashes with every theme, which is
    /// AD-13 one layer out.
    Tag { text: String, tone: Tone },
}

/// What a tag means, from which the theme decides how it looks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tone {
    #[default]
    Neutral,
    Accent,
    Positive,
    Caution,
    Danger,
}

/// What to draw in a row's icon square.
///
/// No arbitrary images by path from an extension *yet*: a file an extension
/// names is a file the host would read on its behalf, which is AD-09's host
/// RPC to decide, not a field to add. [`Icon::Image`] is the host's own, for
/// files it wrote, and no extension can send it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Icon {
    /// One of the host's bundled glyphs, by name.
    ///
    /// Unknown names draw nothing rather than failing to load the view. An
    /// extension written against a later version of Lumen must still run on
    /// this one -- the same reasoning as I-12 for theme tokens.
    Glyph { name: String },
    /// A single letter or emoji, drawn in the icon square.
    ///
    /// The cheap way to give rows identity without shipping images.
    Text { text: String },
    /// A colour, filling the icon square: what a copied colour looks like
    /// (F12). `#rrggbb` or `#rrggbbaa`; anything else draws nothing.
    ///
    /// The one colour in the vocabulary, and the one exception to I-5 and
    /// AD-13: the colour is the *content* of the row, not a choice about how
    /// the row looks, so there is no theme token it could come from. The
    /// square's size, radius and border are still the theme's and the
    /// layout's.
    Swatch { color: String },
    /// A picture the host wrote, drawn in the icon square: a copied image's
    /// thumbnail (F12). Loaded off the UI thread; the square is reserved
    /// while it loads (I-10).
    ///
    /// The host's alone: skipped by serde, so a view an extension sends
    /// cannot carry one, and I-7 holds -- the host never reads a file because
    /// an extension named it.
    #[serde(skip)]
    Image { path: std::path::PathBuf },
    /// The shell's own icon for a file or folder the host found (F16): a
    /// zip's archiver, a folder's custom icon. Extracted off the UI thread;
    /// the glyph `fallback` names is drawn in the square until it arrives,
    /// and for good if it cannot be had (I-10).
    ///
    /// The host's alone, as [`Icon::Image`] is, and for the same reason.
    #[serde(skip)]
    File {
        path: std::path::PathBuf,
        folder: bool,
        fallback: String,
    },
}

/// Something the user can do to a row or to the view.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Action {
    /// Sent back to the extension when this action runs. Opaque to the host.
    pub id: String,
    /// The verb shown in the footer and in the action panel.
    pub title: String,
    /// A key combination shown beside it, such as `"Ctrl+Shift+C"`.
    ///
    /// Displayed, not bound. Binding it would let an extension take a key the
    /// host or another extension already uses, and a keymap that depends on
    /// what is installed is a keymap nobody can learn. Binding arrives, if it
    /// arrives, as something the *user* configures.
    pub shortcut: Option<String>,
    /// The icon drawn beside it in the action panel, in the same vocabulary as
    /// a row's.
    ///
    /// Optional, and absent from every view written before it existed: an
    /// action without one still gets its square in the panel, empty, so the
    /// titles line up (I-10). Additive on the wire, which is what I-12 asks
    /// of anything an extension may already be sending.
    pub icon: Option<Icon>,
}

/// What to show when a list has nothing in it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct EmptyState {
    pub title: String,
    pub description: Option<String>,
    pub icon: Option<Icon>,
}

/// One thing, shown at length.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Detail {
    /// The body, as a restricted subset of Markdown.
    ///
    /// Markdown rather than a node tree because prose is not a tree anybody
    /// wants to build by hand, and restricted because the vocabulary is closed:
    /// headings, paragraphs, emphasis, code, lists and rules. No images, no raw
    /// HTML, no links that navigate anywhere the host did not choose.
    pub markdown: String,
    /// Labelled values shown beside the body.
    pub metadata: Vec<MetadataRow>,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct MetadataRow {
    pub label: String,
    pub value: String,
    pub tone: Tone,
}

/// What the host tells an extension has happened.
///
/// Written down with the tree because the contract is one artefact (AD-06) and
/// keeping half of it in another commit is how two artefacts start. Nothing
/// sends one yet: that is step 3, and per step 4 none of this is stable until a
/// second extension has been written against it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Event {
    /// The query changed. Only sent under [`Filtering::Extension`]; under
    /// [`Filtering::Host`] the host filters and the extension is not told,
    /// which is the point of the distinction.
    QueryChanged { query: String },
    /// The selection moved. Sent so a detail pane can follow it.
    SelectionChanged { item: String },
    /// An option of a [`Scope`] was chosen, both by id. Only sent to a list
    /// that has scopes.
    ScopeChanged { scope: String, option: String },
    /// The innermost level of [`List::path`] was taken off (F13). Only sent
    /// to a list with a path.
    Back,
    /// An action ran.
    ActionInvoked {
        item: Option<String>,
        action: String,
    },
    /// The view was closed.
    Dismissed,
}

/// Read a view from JSON.
///
/// Its own function rather than `serde_json::from_str` at the call site so that
/// what a malformed view does is decided in one place: it is reported and the
/// view does not open. Unlike a theme (I-12), there is no sensible partial
/// result -- half a list is not a list.
pub fn from_json(source: &str) -> Result<View, serde_json::Error> {
    serde_json::from_str(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An action written before actions had icons still loads, with none; one
    /// that names a glyph keeps it. A picture by path is refused here as it
    /// is on a row (I-7).
    #[test]
    fn an_actions_icon_is_optional_on_the_wire() {
        let old: Action = serde_json::from_str(r#"{ "id": "open", "title": "Open" }"#).unwrap();
        assert_eq!(old.icon, None);

        let new: Action = serde_json::from_str(
            r#"{ "id": "open", "title": "Open", "icon": { "type": "glyph", "name": "open" } }"#,
        )
        .unwrap();
        assert_eq!(
            new.icon,
            Some(Icon::Glyph {
                name: "open".into()
            })
        );

        let picture: Result<Action, _> = serde_json::from_str(
            r#"{ "id": "x", "icon": { "type": "image", "path": "C:\\secret.png" } }"#,
        );
        assert!(picture.is_err());
    }

    /// I-7: a picture by path is the host's alone. An extension that names
    /// one is refused, and one it puts in a pane is dropped, so the host
    /// never reads a file because an extension said so.
    #[test]
    fn an_extension_cannot_name_a_picture_for_the_host_to_read() {
        let icon: Result<Icon, _> =
            serde_json::from_str(r#"{ "type": "image", "path": "C:\\secret.png" }"#);
        assert!(icon.is_err());

        let detail: ItemDetail =
            serde_json::from_str(r#"{ "markdown": "x", "image": "C:\\secret.png" }"#).unwrap();
        assert_eq!(detail.image, None);
    }

    fn item(id: &str, title: &str) -> Item {
        Item {
            id: id.into(),
            title: title.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_list_indexes_across_section_boundaries() {
        let list = List {
            sections: vec![
                Section {
                    title: Some("First".into()),
                    subtitle: None,
                    items: vec![item("a", "A"), item("b", "B")],
                },
                Section {
                    title: Some("Second".into()),
                    subtitle: None,
                    items: vec![item("c", "C")],
                },
            ],
            ..Default::default()
        };

        assert_eq!(list.len(), 3);
        assert_eq!(list.item(2).unwrap().id, "c");
        assert!(list.item(3).is_none());
    }

    /// A view written for a later Lumen must still load on this one, which
    /// means every optional field has to be genuinely optional.
    #[test]
    fn the_smallest_possible_list_is_valid() {
        let view = from_json(r#"{ "type": "list" }"#).unwrap();
        let View::List(list) = view else {
            panic!("expected a list")
        };
        assert!(list.is_empty());
        assert_eq!(list.filtering, Filtering::Host);
    }

    #[test]
    fn an_item_needs_only_a_title_and_an_id() {
        let view = from_json(
            r#"{ "type": "list", "sections": [{ "items": [{ "id": "x", "title": "X" }] }] }"#,
        )
        .unwrap();
        let View::List(list) = view else {
            panic!("expected a list")
        };
        assert_eq!(list.item(0).unwrap().title, "X");
        assert!(list.item(0).unwrap().actions.is_empty());
    }

    /// Host filtering is what the budget can promise, so it has to be what you
    /// get by saying nothing.
    #[test]
    fn filtering_defaults_to_the_host() {
        assert_eq!(Filtering::default(), Filtering::Host);
    }

    #[test]
    fn the_tree_round_trips_through_json() {
        let view = View::List(List {
            placeholder: Some("Search issues".into()),
            filtering: Filtering::Extension,
            scopes: vec![scope(&["mine", "all"], "mine")],
            path: Vec::new(),
            empty: Some(EmptyState {
                title: "Nothing open".into(),
                description: Some("Every issue is closed.".into()),
                icon: Some(Icon::Glyph {
                    name: "command".into(),
                }),
            }),
            showing_detail: true,
            sections: vec![Section {
                title: Some("Assigned".into()),
                subtitle: None,
                items: vec![Item {
                    detail: Some(ItemDetail {
                        markdown: "# Fix the seam\n\nThe pane *follows*.".into(),
                        text: None,
                        code: None,
                        image: None,
                        metadata: vec![MetadataRow {
                            label: "Opened".into(),
                            value: "2 days ago".into(),
                            tone: Tone::Neutral,
                        }],
                    }),
                    id: "1".into(),
                    title: "Fix the seam".into(),
                    prefix: None,
                    subtitle: Some("lumen#12".into()),
                    icon: Some(Icon::Text {
                        text: "🐛".into()
                    }),
                    accessories: vec![
                        Accessory::Text {
                            text: "2 days".into(),
                            strong: false,
                        },
                        Accessory::Tag {
                            text: "open".into(),
                            tone: Tone::Positive,
                        },
                    ],
                    keywords: vec!["tiling".into()],
                    actions: vec![Action {
                        id: "open".into(),
                        title: "Open in Browser".into(),
                        shortcut: Some("Ctrl+O".into()),
                        icon: Some(Icon::Glyph {
                            name: "open".into(),
                        }),
                    }],
                }],
            }],
        });

        let json = serde_json::to_string(&view).unwrap();
        assert_eq!(from_json(&json).unwrap(), view);
    }

    /// The tags are semantic. A tone that names a colour would clash with every
    /// theme, which is AD-13 one layer further out.
    #[test]
    fn a_tone_is_a_meaning_and_not_a_colour() {
        let json = serde_json::to_string(&Tone::Caution).unwrap();
        assert_eq!(json, "\"caution\"");
    }

    #[test]
    fn an_unreadable_view_is_an_error_rather_than_an_empty_one() {
        assert!(from_json("{ nonsense").is_err());
        assert!(from_json(r#"{ "type": "carousel" }"#).is_err());
    }

    fn scope(ids: &[&str], selected: &str) -> Scope {
        Scope {
            id: "filter".into(),
            title: "Show".into(),
            options: ids
                .iter()
                .map(|id| ScopeOption {
                    id: (*id).into(),
                    title: id.to_uppercase(),
                    keywords: Vec::new(),
                })
                .collect(),
            selected: selected.into(),
        }
    }

    #[test]
    fn a_scope_knows_its_current_option() {
        let scope = scope(&["a", "b", "c"], "b");
        assert_eq!(scope.current().unwrap().title, "B");
        assert_eq!(scope.position(), 1);
    }

    #[test]
    fn a_scope_naming_no_option_starts_at_the_first() {
        let scope = scope(&["a", "b"], "gone");
        assert!(scope.current().is_none());
        assert_eq!(scope.position(), 0);
    }

    /// Lists written before the pane existed load without one, and an item
    /// may carry a detail with only a body.
    #[test]
    fn the_detail_pane_is_opt_in() {
        let View::List(list) = from_json(r#"{ "type": "list" }"#).unwrap() else {
            panic!("expected a list")
        };
        assert!(!list.showing_detail);

        let view = from_json(
            r##"{ "type": "list", "showing-detail": true, "sections": [{ "items": [
                { "id": "x", "title": "X", "detail": { "markdown": "# X" } }
            ] }] }"##,
        )
        .unwrap();
        let View::List(list) = view else {
            panic!("expected a list")
        };
        assert!(list.showing_detail);
        let detail = list.item(0).unwrap().detail.as_ref().unwrap();
        assert_eq!(detail.markdown, "# X");
        assert!(detail.metadata.is_empty());
    }

    /// Lists written before scopes existed must still load, with none.
    #[test]
    fn a_list_without_scopes_has_none() {
        let View::List(list) = from_json(r#"{ "type": "list" }"#).unwrap() else {
            panic!("expected a list")
        };
        assert!(list.scopes.is_empty());
    }

    #[test]
    fn a_scope_change_names_the_scope_and_the_option() {
        let json = serde_json::to_string(&Event::ScopeChanged {
            scope: "to".into(),
            option: "lang-fr".into(),
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"type":"scope-changed","scope":"to","option":"lang-fr"}"#
        );
    }

    #[test]
    fn actions_come_from_the_selected_item() {
        let view = View::List(List {
            sections: vec![Section {
                title: None,
                subtitle: None,
                items: vec![
                    Item {
                        actions: vec![Action {
                            id: "first".into(),
                            ..Default::default()
                        }],
                        ..item("a", "A")
                    },
                    item("b", "B"),
                ],
            }],
            ..Default::default()
        });

        assert_eq!(view.actions(0)[0].id, "first");
        assert!(view.actions(1).is_empty());
        assert!(view.actions(99).is_empty());
    }
}
