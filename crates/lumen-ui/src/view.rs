//! Drawing an extension's view (F4, step 2).
//!
//! The plan's build order for F4 is explicit that it is not optional:
//!
//! 1. Hand-write the JSON of a `List` tree with ten items and an action panel.
//!    Render it in GPUI from a static file. No protocol, no processes.
//! 2. Wrap that behind `render_node(node) -> AnyElement`, with list
//!    virtualisation.
//! 3. Only then move the tree across a channel and introduce events.
//! 4. Write the *second* extension before stabilising anything in the protocol.
//!
//! This is step 2. There is no process, no channel and no event anywhere in
//! this file: it takes a [`View`](lumen_core::view::View) that is already in
//! memory and turns it into elements. Where the tree came from is not its
//! business, which is the entire reason for doing it in this order -- the
//! renderer is finished and known-good before anything can be blamed on a
//! transport.
//!
//! # Why `list` and not `uniform_list`
//!
//! The root list uses `uniform_list`, which is cheaper and requires every row
//! to be the same height. A view has section headers, which are shorter than
//! rows on purpose (a header is a label, not an entry). `list` virtualises with
//! variable heights, which is what step 2's "with list virtualisation" needs
//! here: ten items or ten thousand, only what is on screen is built.
//!
//! # What this deliberately shares with the root list
//!
//! Row height, gutter, icon square, selection treatment, type sizes. All of it
//! comes from the same tokens (I-5), so an extension's results cannot look like
//! a different application. That is AD-07 in practice: the extension chose what
//! to show and had no say at all in how it looks.

use gpui::prelude::*;
use gpui::{AnyElement, ListAlignment, ListState, SharedString, Styled, div, list, px, svg};
use lumen_core::layout;
use lumen_core::theme::{Colors, Theme};
use lumen_core::view::{Accessory, Detail, EmptyState, Icon, Item, List, MAX_ACCESSORIES, Tone};

use crate::styling::{color, metrics, type_scale};

/// One line of a rendered list, after sections have been flattened.
///
/// The flattening happens once when the view is set rather than on every frame:
/// a virtualised list asks for row `n` and has to be able to answer without
/// walking the sections to find it.
#[derive(Clone)]
pub enum Row {
    /// A section's title, and its subtitle if it has one.
    Section(SharedString, Option<SharedString>),
    /// An index into the view's flat item order.
    Item(usize),
}

/// The rows of `list`, in display order.
pub fn rows(source: &List) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut index = 0;

    for section in &source.sections {
        if let Some(title) = &section.title {
            rows.push(Row::Section(
                title.clone().into(),
                section.subtitle.clone().map(Into::into),
            ));
        }
        for _ in &section.items {
            rows.push(Row::Item(index));
            index += 1;
        }
    }

    rows
}

/// Fresh list state for `count` rows.
pub fn list_state(count: usize) -> ListState {
    // No overdraw: the rows are cheap, the window is short, and drawing rows
    // nobody can see is exactly the kind of work the frame budget is for.
    ListState::new(count, ListAlignment::Top, px(0.0))
}

/// The height of one row, so the window can be sized before anything is drawn.
pub fn row_height(row: &Row) -> f32 {
    match row {
        Row::Section(..) => layout::view::SECTION_HEIGHT,
        Row::Item(_) => layout::ROW_HEIGHT,
    }
}

/// Draw a list, virtualised.
///
/// `selected` is an index into the flat item order, not into `rows`: the
/// selection moves between items and steps over headers, because a header is
/// not something you can choose.
pub fn render_list(
    source: &List,
    rows: &[Row],
    state: &ListState,
    selected: usize,
    theme: &Theme,
    height: gpui::Pixels,
) -> AnyElement {
    if source.is_empty() {
        return render_empty(source.empty.as_ref(), theme, height);
    }

    // Cloned into the closure because `list` holds its render function for the
    // life of the element and cannot borrow from this frame's `source`. The
    // cost is one clone per rebuild, not per frame: the element is rebuilt only
    // when `Render` runs, and the alternative -- an `Rc` threaded through every
    // caller -- buys nothing at ten items and complicates the ownership at ten
    // thousand.
    let items: Vec<Item> = source.items().cloned().collect();
    let rows = rows.to_vec();
    let colors = theme.colors.clone();

    div()
        .h(height)
        .py(metrics::list_padding())
        .child(
            list(state.clone(), move |index, _window, _cx| {
                match rows.get(index) {
                    Some(Row::Section(title, subtitle)) => {
                        render_section(title.clone(), subtitle.clone(), &colors)
                    }
                    Some(Row::Item(item)) => match items.get(*item) {
                        Some(entry) => render_item(entry, *item == selected, &colors),
                        None => div().into_any_element(),
                    },
                    None => div().into_any_element(),
                }
            })
            .size_full(),
        )
        .into_any_element()
}

/// A section's header: the title, and the subtitle after it in the same row
/// and the same size, only dimmer -- so a header with a subtitle is exactly
/// as tall as one without (I-10).
fn render_section(
    title: SharedString,
    subtitle: Option<SharedString>,
    colors: &Colors,
) -> AnyElement {
    div()
        .h(px(layout::view::SECTION_HEIGHT))
        .px(metrics::gutter())
        .flex()
        .items_center()
        .gap(px(layout::view::SECTION_SUBTITLE_GAP))
        .overflow_hidden()
        .text_size(type_scale::footer())
        .text_color(color(colors.text_muted))
        .child(div().flex_none().child(title))
        .when_some(subtitle, |header, subtitle| {
            header.child(
                div()
                    .truncate()
                    .text_color(color(colors.text_placeholder))
                    .child(subtitle),
            )
        })
        .into_any_element()
}

/// One item row.
///
/// Structurally the same as a root-list row and for the same reasons: a fixed
/// height whether or not there is a subtitle, an icon square that exists before
/// the icon does, and the accessories in a column that cannot be pushed around
/// by the text beside them (I-10).
fn render_item(item: &Item, selected: bool, colors: &Colors) -> AnyElement {
    div()
        .h(metrics::row_height())
        .w_full()
        .mx(metrics::row_inset())
        .px(metrics::gutter() - metrics::row_inset())
        .flex()
        .items_center()
        .gap(metrics::icon_gap())
        .rounded(metrics::row_radius())
        .when(selected, |row| {
            row.bg(color(colors.selection_background))
                .border_1()
                .border_color(color(colors.selection_border))
        })
        .when(!selected, |row| {
            row.border_1().border_color(gpui::transparent_black())
        })
        .child(render_icon(item.icon.as_ref(), colors))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .overflow_hidden()
                .child(
                    div()
                        .text_size(type_scale::title())
                        .text_color(color(colors.text_primary))
                        .truncate()
                        .child(item.title.clone()),
                )
                .when_some(item.subtitle.as_ref(), |slot, subtitle| {
                    slot.child(
                        div()
                            .text_size(type_scale::subtitle())
                            .text_color(color(colors.text_muted))
                            .truncate()
                            .child(subtitle.clone()),
                    )
                }),
        )
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(layout::view::ACCESSORY_GAP))
                // Dropped rather than squeezed past the cap: a row that shrinks
                // its text to fit a fourth accessory no longer lines up with
                // the row above it.
                .children(
                    item.accessories
                        .iter()
                        .take(MAX_ACCESSORIES)
                        .map(|accessory| render_accessory(accessory, colors)),
                ),
        )
        .into_any_element()
}

fn render_icon(icon: Option<&Icon>, colors: &Colors) -> AnyElement {
    let square = div()
        .size(metrics::icon_size())
        .flex_none()
        .flex()
        .items_center()
        .justify_center();

    match icon {
        // An unknown glyph name draws an empty square rather than failing the
        // view: an extension written for a later Lumen still runs on this one,
        // which is I-12's reasoning applied to the extension contract.
        Some(Icon::Glyph { name }) => square
            .child(
                svg()
                    .path(format!("icons/{name}.svg"))
                    .size(metrics::icon_size())
                    .text_color(color(colors.icon)),
            )
            .into_any_element(),
        Some(Icon::Text { text }) => square
            .text_size(type_scale::title())
            .text_color(color(colors.icon))
            .child(text.clone())
            .into_any_element(),
        None => square.into_any_element(),
    }
}

fn render_accessory(accessory: &Accessory, colors: &Colors) -> AnyElement {
    match accessory {
        Accessory::Text { text } => div()
            .text_size(type_scale::footer())
            .text_color(color(colors.text_muted))
            .child(text.clone())
            .into_any_element(),
        Accessory::Tag { text, tone } => div()
            .px(px(layout::view::TAG_PADDING_X))
            .py(px(layout::view::TAG_PADDING_Y))
            .rounded(px(layout::view::TAG_RADIUS))
            .bg(color(tone_background(*tone, colors)))
            .text_size(type_scale::footer())
            .text_color(color(tone_foreground(*tone, colors)))
            .child(text.clone())
            .into_any_element(),
    }
}

/// What a tone looks like, decided here and never by the extension.
///
/// The five tones map onto tokens the theme already has rather than onto five
/// new ones. A theme author should not have to learn an extension vocabulary to
/// make their theme work, and a tone that resolved to a token most themes never
/// set would be a tone that looks broken in most themes (I-12).
fn tone_foreground(tone: Tone, colors: &Colors) -> lumen_core::theme::Color {
    match tone {
        Tone::Neutral => colors.text_muted,
        Tone::Accent | Tone::Positive | Tone::Caution | Tone::Danger => colors.text_primary,
    }
}

fn tone_background(tone: Tone, colors: &Colors) -> lumen_core::theme::Color {
    match tone {
        Tone::Neutral => colors.selection_background,
        Tone::Accent => colors.accent.with_alpha(TAG_ALPHA),
        // Positive, caution and danger have no tokens of their own yet, and
        // inventing three would mean every existing theme is missing three. The
        // accent carries them at different weights until a theme file can say
        // otherwise -- which is a change to the theme schema, not to this file.
        Tone::Positive => colors.accent.with_alpha(TAG_ALPHA_STRONG),
        Tone::Caution => colors.match_highlight.with_alpha(TAG_ALPHA),
        Tone::Danger => colors.border_strong,
    }
}

/// How much of the accent a tag's background keeps.
///
/// A pill filled with the full accent reads as a button -- something to press
/// rather than something to notice.
const TAG_ALPHA: u8 = 0x30;
const TAG_ALPHA_STRONG: u8 = 0x48;

fn render_empty(empty: Option<&EmptyState>, theme: &Theme, height: gpui::Pixels) -> AnyElement {
    let colors = &theme.colors;
    let Some(empty) = empty else {
        // A list with nothing in it and nothing to say about it is blank, not
        // decorated with an apology the extension did not write.
        return div().h(height).into_any_element();
    };

    div()
        .h(height)
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(layout::view::EMPTY_GAP))
        .when_some(empty.icon.as_ref(), |slot, icon| match icon {
            Icon::Glyph { name } => slot.child(
                svg()
                    .path(format!("icons/{name}.svg"))
                    .size(px(layout::view::EMPTY_ICON_SIZE))
                    .text_color(color(colors.text_placeholder)),
            ),
            Icon::Text { text } => slot.child(
                div()
                    .text_size(px(layout::view::EMPTY_ICON_SIZE))
                    .child(text.clone()),
            ),
        })
        .child(
            div()
                .text_size(type_scale::title())
                .text_color(color(colors.text_primary))
                .child(empty.title.clone()),
        )
        .when_some(empty.description.as_ref(), |slot, description| {
            slot.child(
                div()
                    .text_size(type_scale::subtitle())
                    .text_color(color(colors.text_muted))
                    .child(description.clone()),
            )
        })
        .into_any_element()
}

/// Draw a view that has been asked for but has not arrived.
///
/// A state, not a spinner. The launcher is already open, already focused and
/// already taking keystrokes; what is missing is the content, and saying so
/// plainly is better than animating to fill a gap that is usually gone in a
/// few tens of milliseconds. An animation here would draw the eye to the one
/// moment the interface is not ready.
pub fn render_loading(theme: &Theme, height: gpui::Pixels) -> AnyElement {
    div()
        .h(height)
        .flex()
        .items_center()
        .justify_center()
        .text_size(type_scale::subtitle())
        .text_color(color(theme.colors.text_placeholder))
        .child("Starting…")
        .into_any_element()
}

/// Draw a detail view.
///
/// The Markdown is drawn as plain paragraphs for now, which is honest about
/// where this is: the vocabulary says "a restricted subset of Markdown" and
/// rendering that subset properly is its own piece of work. What matters at
/// step 2 is that `Detail` has a place in the renderer and the caller does not
/// have to care which variant it holds.
pub fn render_detail(detail: &Detail, theme: &Theme, height: gpui::Pixels) -> AnyElement {
    let colors = &theme.colors;

    div()
        .h(height)
        .px(metrics::gutter())
        .py(metrics::list_padding())
        .flex()
        .flex_col()
        .gap(px(layout::view::EMPTY_GAP))
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .text_size(type_scale::title())
                .text_color(color(colors.text_primary))
                .child(detail.markdown.clone()),
        )
        .children(detail.metadata.iter().map(|row| {
            div()
                .flex()
                .justify_between()
                .text_size(type_scale::footer())
                .child(
                    div()
                        .text_color(color(colors.text_muted))
                        .child(row.label.clone()),
                )
                .child(
                    div()
                        .text_color(color(tone_foreground(row.tone, colors)))
                        .child(row.value.clone()),
                )
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_core::view::{Section, View};

    fn item(id: &str) -> Item {
        Item {
            id: id.into(),
            title: id.into(),
            ..Default::default()
        }
    }

    fn example() -> List {
        List {
            sections: vec![
                Section {
                    title: Some("First".into()),
                    subtitle: None,
                    items: vec![item("a"), item("b")],
                },
                Section {
                    title: None,
                    subtitle: None,
                    items: vec![item("c")],
                },
            ],
            ..Default::default()
        }
    }

    /// A section without a title contributes no header. The alternative is a
    /// blank band of nothing where a label should be.
    #[test]
    fn only_titled_sections_produce_a_header() {
        let rows = rows(&example());
        assert_eq!(rows.len(), 4);
        assert!(matches!(rows[0], Row::Section(..)));
        assert!(matches!(rows[1], Row::Item(0)));
        assert!(matches!(rows[2], Row::Item(1)));
        assert!(matches!(rows[3], Row::Item(2)));
    }

    /// The flat item index is what the selection moves through, so it has to
    /// keep counting across a section boundary rather than restarting.
    #[test]
    fn item_indices_continue_across_sections() {
        let rows = rows(&example());
        let items: Vec<usize> = rows
            .iter()
            .filter_map(|row| match row {
                Row::Item(index) => Some(*index),
                Row::Section(..) => None,
            })
            .collect();
        assert_eq!(items, [0, 1, 2]);
    }

    #[test]
    fn a_header_is_shorter_than_a_row() {
        assert!(row_height(&Row::Section("x".into(), None)) < row_height(&Row::Item(0)));
    }

    /// The file the plan asks for in step 1, parsed and counted, so that
    /// editing it cannot silently produce something the renderer will not draw.
    #[test]
    fn the_bundled_example_is_a_list_of_ten_items_with_an_action_panel() {
        let source = include_str!("../../../assets/views/example.json");
        let view = lumen_core::view::from_json(source).expect("the example view parses");

        let View::List(list) = view else {
            panic!("the example is a list")
        };

        assert_eq!(list.len(), 10);
        assert_eq!(list.sections.len(), 2);
        assert_eq!(rows(&list).len(), 12);

        // "with an action panel": at least one item has more than the one
        // action `Enter` would run, or there is nothing for the panel to show.
        assert!(list.items().any(|item| item.actions.len() > 1));
        assert!(list.items().all(|item| !item.actions.is_empty()));
    }

    /// Every glyph the example names has to exist, or the view draws blanks.
    #[test]
    fn the_example_names_only_bundled_glyphs() {
        use gpui::AssetSource as _;

        let source = include_str!("../../../assets/views/example.json");
        let View::List(list) = lumen_core::view::from_json(source).unwrap() else {
            panic!("the example is a list")
        };

        for item in list.items() {
            if let Some(Icon::Glyph { name }) = &item.icon {
                let path = format!("icons/{name}.svg");
                assert!(
                    crate::Assets.load(&path).unwrap().is_some(),
                    "{path} is not bundled"
                );
            }
        }
    }
}
