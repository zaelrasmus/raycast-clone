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

use std::hash::{DefaultHasher, Hash, Hasher};

use gpui::prelude::*;
use gpui::{
    AnyElement, App, FontStyle, FontWeight, HighlightStyle, ListAlignment, ListState, SharedString,
    Styled, StyledText, div, img, list, px, relative, svg,
};
use lumen_core::config::PreviewFont;
use lumen_core::layout;
use lumen_core::markdown::{self, Block, Span};
use lumen_core::style::{Metrics, RowSubtitle};
use lumen_core::theme::{Colors, Theme};
use lumen_core::view::{
    Accessory, Detail, EmptyState, Icon, Item, ItemDetail, List, MAX_ACCESSORIES, MetadataRow,
    Presentation, Tone,
};

use crate::icons::IconCache;
use crate::images::{Fit, ImageCache, PREVIEW_PX};
use crate::styling::{Geometry, color, type_scale};

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
    /// A grid's strip: the flat indices of its cells, left to right (F18).
    Cells(Vec<usize>),
}

impl Row {
    /// The items this row shows: one for an item, none for a header, every
    /// cell of a strip.
    pub fn items(&self) -> &[usize] {
        match self {
            Row::Section(..) => &[],
            Row::Item(index) => std::slice::from_ref(index),
            Row::Cells(cells) => cells,
        }
    }
}

/// The rows of `list`, in display order, a grid's cut into strips of
/// `columns` cells. A list of rows ignores `columns`.
pub fn rows(source: &List, columns: usize) -> Vec<Row> {
    flatten(source, columns, |_| true, true)
}

/// The rows of `list` that show an item `keep` accepts, by flat index.
///
/// A header with nothing left under it is dropped: a heading about nothing.
/// A grid packs what survives into full strips, whatever the gaps.
pub fn rows_where(source: &List, columns: usize, keep: impl Fn(usize) -> bool) -> Vec<Row> {
    flatten(source, columns, keep, false)
}

fn flatten(
    source: &List,
    columns: usize,
    keep: impl Fn(usize) -> bool,
    empty_headers: bool,
) -> Vec<Row> {
    let grid = source.presentation == Presentation::Grid;
    let columns = columns.max(1);
    let mut rows = Vec::new();
    let mut index = 0;

    for section in &source.sections {
        let shown: Vec<usize> = (index..index + section.items.len())
            .filter(|&item| keep(item))
            .collect();
        index += section.items.len();

        if let Some(title) = &section.title
            && (empty_headers || !shown.is_empty())
        {
            rows.push(Row::Section(
                title.clone().into(),
                section.subtitle.clone().map(Into::into),
            ));
        }
        if grid {
            rows.extend(
                shown
                    .chunks(columns)
                    .map(|strip| Row::Cells(strip.to_vec())),
            );
        } else {
            rows.extend(shown.into_iter().map(Row::Item));
        }
    }

    rows
}

/// The row that shows the item at flat index `item`, if any does.
pub fn row_of(rows: &[Row], item: usize) -> Option<usize> {
    rows.iter().position(|row| row.items().contains(&item))
}

/// How many columns `list` has in a window `width` wide: those that fit
/// between the gutters, in the list's share of the window beside a detail
/// pane. A list of rows has one.
pub fn grid_columns(list: &List, metrics: &Metrics, width: f32) -> usize {
    if list.presentation != Presentation::Grid {
        return 1;
    }
    let width = if list.showing_detail {
        width * layout::view::DETAIL_LIST_SHARE
    } else {
        width
    };
    layout::view::grid_columns(
        width - 2.0 * metrics.gutter,
        metrics.grid_cell,
        metrics.grid_gap,
    )
}

/// Fresh list state for `count` rows.
pub fn list_state(count: usize) -> ListState {
    // No overdraw: the rows are cheap, the window is short, and drawing rows
    // nobody can see is exactly the kind of work the frame budget is for.
    ListState::new(count, ListAlignment::Top, px(0.0))
}

/// The height of one row, so the window can be sized before anything is drawn.
pub fn row_height(row: &Row, metrics: &Metrics) -> f32 {
    match row {
        Row::Section(..) => metrics.section_height,
        Row::Item(_) => metrics.row_height,
        // Every strip, the short last one too (I-10).
        Row::Cells(_) => metrics.grid_cell + metrics.grid_gap,
    }
}

/// What a view's body is, as far as its height goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// A list on its own: as tall as its rows.
    List,
    /// A list with the detail pane beside it (F10).
    Pane,
    /// A detail view, which has no rows at all.
    Detail,
}

/// How tall a view's rows are, rounded down to a whole row.
///
/// `cap` is how tall the list may grow, `Config::list_height`, the root
/// list's own; `full` is how tall a full root list is,
/// `Config::full_list_height`. A list ends on whole rows within `cap`, as the
/// root list does (7.29). A view whose height does not follow its rows -- a
/// pane, while its rows fit, a detail view, and a view with no rows, loading
/// or empty -- is as tall as a full root list, so a style's height reaches
/// views the way it reaches the root list (7.48).
///
/// Not `total.min(cap)`. A view's rows are not all the same height -- a
/// section header is shorter than an item -- so capping the sum lands
/// wherever it lands, which for the bundled example is two thirds of the
/// way through the last row it shows. Accumulating whole rows until the
/// next one would not fit is what keeps I-10's promise here: a partial row
/// is never left clipped at the bottom edge.
pub fn rows_height(rows: &[Row], shape: Shape, metrics: &Metrics, cap: f32, full: f32) -> f32 {
    if shape == Shape::Detail {
        return full;
    }
    // A list with nothing in it still has something to say, and needs room
    // to say it: the whole list's, not a few rows'. A view is empty while it
    // loads and when its search matches nothing, and anything shorter would
    // make the window grow when the rows arrive and shrink as the user types.
    if rows.is_empty() {
        return full;
    }

    let mut used = 0.0;
    let mut all_fit = true;
    for row in rows {
        let height = row_height(row, metrics);
        if used + height > cap {
            all_fit = false;
            break;
        }
        used += height;
    }

    // A list with a detail pane is as tall as the window allows, however
    // few rows it has (F10): the pane holds sentences, and sizing the
    // window to two senses would leave it no room to show either. As tall
    // as a full root list, not the cap itself, which need not be a whole
    // number of rows; taller only when headers make the rows that fit so.
    // When the rows overflow, they still end on a whole row.
    if all_fit && shape == Shape::Pane {
        return used.max(full);
    }

    // A single row taller than the whole cap is shown anyway: an empty
    // window is worse than one row that does not quite fit.
    if used == 0.0 {
        return row_height(&rows[0], metrics);
    }

    used
}

/// Where a list's scroll should start so that `row` is fully visible in a
/// viewport `viewport` tall, given where it starts now, as (row, offset in it).
///
/// Worked out from the rows' heights, which are known exactly (I-10), rather
/// than left to GPUI's `scroll_to_reveal_item`: that one adds up the heights
/// it has *measured*, and a row below the fold has never been drawn, so it
/// counts as nothing and the list never scrolls down to it (F10).
///
/// Moving up to a section's first item reveals the section's header too:
/// an item scrolled to the top edge with its heading just out of sight
/// reads as belonging to the section above.
pub fn reveal(
    rows: &[Row],
    metrics: &Metrics,
    top: (usize, f32),
    row: usize,
    viewport: f32,
) -> (usize, f32) {
    let row_height = |row: &Row| row_height(row, metrics);
    let offset = |index: usize| -> f32 { rows.iter().take(index).map(row_height).sum() };
    let Some(target) = rows.get(row) else {
        return top;
    };

    let current = offset(top.0.min(rows.len())) + top.1;
    let row_top = offset(row);
    let row_bottom = row_top + row_height(target);

    let new_top = if row_top < current {
        match row.checked_sub(1).and_then(|above| rows.get(above)) {
            Some(Row::Section(..)) => row_top - metrics.section_height,
            _ => row_top,
        }
    } else if row_bottom > current + viewport {
        row_bottom - viewport
    } else {
        return top;
    };

    // Back from pixels to a row and an offset into it.
    let mut start = 0.0;
    for (index, each) in rows.iter().enumerate() {
        let height = row_height(each);
        if new_top < start + height {
            return (index, (new_top - start).max(0.0));
        }
        start += height;
    }
    (rows.len(), 0.0)
}

/// Draw a list, virtualised.
///
/// `selected` is an index into the flat item order, not into `rows`: the
/// selection moves between items and steps over headers, because a header is
/// not something you can choose.
#[allow(clippy::too_many_arguments)]
pub fn render_list(
    source: &List,
    rows: &[Row],
    state: &ListState,
    selected: usize,
    theme: &Theme,
    height: gpui::Pixels,
    images: &ImageCache,
    icons: &IconCache,
    preview_font: PreviewFont,
    words: &[String],
    cx: &mut App,
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
    let detail = source
        .showing_detail
        .then(|| items.get(selected).and_then(|item| item.detail.clone()));
    let rows = rows.to_vec();
    let colors = theme.colors.clone();
    let row_images = images.clone();
    let row_icons = icons.clone();
    let geometry = Geometry::of(theme);

    let list = div()
        .h(height)
        .pt(geometry.list_padding())
        .pb(geometry.list_padding_bottom())
        .child(
            list(state.clone(), move |index, _window, cx| {
                match rows.get(index) {
                    Some(Row::Section(title, subtitle)) => {
                        render_section(title.clone(), subtitle.clone(), &colors, &geometry)
                    }
                    Some(Row::Item(item)) => match items.get(*item) {
                        Some(entry) => render_item(
                            entry,
                            *item == selected,
                            &colors,
                            &geometry,
                            &row_images,
                            &row_icons,
                            cx,
                        ),
                        None => div().into_any_element(),
                    },
                    Some(Row::Cells(cells)) => render_strip(
                        cells,
                        &items,
                        selected,
                        &colors,
                        &geometry,
                        &row_images,
                        &row_icons,
                        cx,
                    ),
                    None => div().into_any_element(),
                }
            })
            .size_full(),
        );

    let Some(detail) = detail else {
        return list.into_any_element();
    };

    // The list keeps its share whether or not the selected item has a
    // detail: a pane that came and went with the selection would move every
    // row sideways (I-10).
    div()
        .h(height)
        .flex()
        .child(
            list.w(relative(layout::view::DETAIL_LIST_SHARE))
                .flex_none(),
        )
        .child(render_pane(
            detail.as_ref(),
            selected,
            theme,
            height,
            images,
            preview_font,
            words,
            cx,
        ))
        .into_any_element()
}

/// The pane beside a list, showing the selected item's detail (F10).
///
/// It scrolls with the wheel when the body is longer than the window; the
/// keys stay the list's, so arrowing moves between items as it always does.
/// With a preview card the card and what follows it scroll apart, each
/// under the wheel over it: GPUI hands a wheel event to every scrolling
/// element under the pointer, so the pane itself does not scroll then.
#[allow(clippy::too_many_arguments)]
fn render_pane(
    detail: Option<&ItemDetail>,
    selected: usize,
    theme: &Theme,
    height: gpui::Pixels,
    images: &ImageCache,
    preview_font: PreviewFont,
    words: &[String],
    cx: &mut App,
) -> AnyElement {
    let colors = &theme.colors;
    // The picture, when there is one, above the body (F12). Its height is
    // absolute: `img` takes the picture's aspect ratio, and with a relative
    // height it keeps the width and lets the height follow -- a tall picture
    // then runs down the pane, under the metadata. Clipped as well, so no
    // shape of picture can leave its box.
    let picture = detail
        .and_then(|detail| detail.image.as_deref())
        .map(|path| {
            let height = px(layout::view::DETAIL_IMAGE_HEIGHT);
            div()
                .w_full()
                .h(height)
                .flex_none()
                .overflow_hidden()
                .mb(px(layout::view::DETAIL_BLOCK_GAP))
                .when_some(
                    images.get(path, PREVIEW_PX, Fit::Within, cx),
                    |slot, image| slot.child(img(image).w_full().h(height)),
                )
        });

    let pane = div()
        .h(height)
        .flex_1()
        .min_w(px(0.0))
        .border_l_1()
        .border_color(color(colors.border_subtle))
        .px(Geometry::of(theme).gutter())
        .py(px(layout::view::DETAIL_PADDING_Y));

    if let Some((detail, code)) = detail.and_then(|detail| Some((detail, detail.code.as_deref()?)))
    {
        return pane
            .flex()
            .flex_col()
            .children(picture)
            .child(render_card(code, selected, theme, preview_font, words))
            .child(
                div()
                    .id(("detail-rest", selected))
                    .flex_1()
                    .min_h(px(0.0))
                    .mt(px(layout::view::DETAIL_BLOCK_GAP))
                    .overflow_y_scroll()
                    .child(render_body(
                        &detail.markdown,
                        detail.text.as_deref(),
                        &detail.metadata,
                        colors,
                    )),
            )
            .into_any_element();
    }

    pane.id("detail-pane")
        .overflow_y_scroll()
        .children(picture)
        .when_some(detail, |pane, detail| {
            pane.child(render_body(
                &detail.markdown,
                detail.text.as_deref(),
                &detail.metadata,
                colors,
            ))
        })
        .into_any_element()
}

/// The family a preview card is drawn in (F16): the palette's monospace
/// family or -- when the user prefers the interface font for previews --
/// the family inherited.
fn card_font(theme: &Theme, preview_font: PreviewFont) -> Option<&str> {
    match preview_font {
        PreviewFont::Monospace => Some(theme.font_mono.as_str()),
        PreviewFont::Interface => None,
    }
}

/// Text as written, in a card on a raised surface (F16): a file's first
/// lines. As tall as its lines up to a bound, past which they scroll inside
/// the card, so the metadata under it stays in view. Lines wrap at the
/// card's edge in either family: a paragraph of prose is one line, and a
/// line cut at the edge hides the rest of it.
///
/// `words`, the query's, are drawn as [`card_match`] wherever they are, and
/// the card starts a few lines above the first when it is further down
/// (slice 6b), so the match is in view before any scrolling. Its scroll
/// position is kept under [`card_scroll_key`], for this preview only.
fn render_card(
    code: &str,
    selected: usize,
    theme: &Theme,
    preview_font: PreviewFont,
    words: &[String],
) -> AnyElement {
    let colors = &theme.colors;
    let family = card_font(theme, preview_font);
    let shown = lumen_core::file_search::highlight::shown(code, words);
    let matched = card_match(colors);
    let key = card_scroll_key(selected, &code[shown.start..]);
    let text = StyledText::new(code[shown.start..].to_string())
        .with_highlights(shown.ranges.into_iter().map(|range| (range, matched)));
    div()
        .id(("detail-card", key))
        .w_full()
        .max_h(px(layout::view::DETAIL_CARD_MAX_HEIGHT))
        .flex_none()
        .overflow_hidden()
        .overflow_y_scroll()
        .px(px(layout::view::DETAIL_CARD_PADDING_X))
        .py(px(layout::view::DETAIL_CARD_PADDING_Y))
        .rounded(px(layout::view::DETAIL_CARD_RADIUS))
        .bg(color(colors.surface_elevated))
        .border_1()
        .border_color(color(colors.border_subtle))
        .text_size(type_scale::code())
        .text_color(color(colors.text_primary))
        .when_some(family, |card, family| {
            card.font_family(SharedString::from(family.to_string()))
        })
        .child(text)
        .into_any_element()
}

/// How a match in a preview card is drawn: the match colour, in bold, on a
/// tint of itself. Only the card's; a list row's match keeps the colour.
fn card_match(colors: &Colors) -> HighlightStyle {
    HighlightStyle {
        color: Some(color(colors.match_highlight)),
        background_color: Some(color(colors.match_highlight.with_alpha(MATCH_ALPHA))),
        font_weight: Some(FontWeight::BOLD),
        ..Default::default()
    }
}

/// The key a preview card's scroll position is kept under.
///
/// GPUI keeps a scrolled element's offset under its id, so an id that names
/// the selection and the text shown keeps the offset across renders of one
/// preview and starts a new one at the top.
fn card_scroll_key(selected: usize, shown: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    selected.hash(&mut hasher);
    shown.hash(&mut hasher);
    hasher.finish()
}

/// A detail's body and its metadata, stacked.
///
/// `text` is drawn as written after the Markdown (F11): the text system lays
/// out each line break as a line, and nothing in it is read as a marker.
/// A preview card, when there is one, is drawn above it by [`render_pane`].
fn render_body(
    source: &str,
    text: Option<&str>,
    metadata: &[MetadataRow],
    colors: &Colors,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(layout::view::DETAIL_BLOCK_GAP))
        .children(
            markdown::parse(source)
                .iter()
                .map(|block| render_block(block, colors)),
        )
        .when_some(text, |body, text| {
            body.child(
                div()
                    .text_size(type_scale::title())
                    .text_color(color(colors.text_primary))
                    .child(text.to_string()),
            )
        })
        .when(!metadata.is_empty(), |body| {
            body.child(
                div()
                    .pt(px(layout::view::DETAIL_BLOCK_GAP))
                    .border_t_1()
                    .border_color(color(colors.border_subtle))
                    .flex()
                    .flex_col()
                    .gap(px(layout::view::DETAIL_BULLET_GAP))
                    .children(metadata.iter().map(|row| render_metadata(row, colors))),
            )
        })
        .into_any_element()
}

fn render_block(block: &Block, colors: &Colors) -> AnyElement {
    match block {
        Block::Heading(level, spans) => div()
            .text_size(if *level == 1 {
                type_scale::heading()
            } else {
                type_scale::title()
            })
            .font_weight(FontWeight::BOLD)
            .text_color(color(if *level == 3 {
                colors.text_muted
            } else {
                colors.text_primary
            }))
            .child(styled(spans, colors))
            .into_any_element(),
        Block::Paragraph(spans) => div()
            .text_size(type_scale::title())
            .text_color(color(colors.text_primary))
            .child(styled(spans, colors))
            .into_any_element(),
        Block::Bullet(spans) => div()
            .flex()
            .gap(px(layout::view::DETAIL_BULLET_GAP))
            .text_size(type_scale::title())
            .text_color(color(colors.text_primary))
            .child(
                div()
                    .flex_none()
                    .text_color(color(colors.text_muted))
                    .child("\u{2022}"),
            )
            .child(div().flex_1().min_w(px(0.0)).child(styled(spans, colors)))
            .into_any_element(),
        Block::Rule => div()
            .border_t_1()
            .border_color(color(colors.border_subtle))
            .into_any_element(),
    }
}

/// One run of text with its spans styled in place, so a line wraps as one
/// line rather than as a row of separate pieces.
fn styled(spans: &[Span], colors: &Colors) -> StyledText {
    let mut text = String::new();
    let mut highlights = Vec::new();
    for span in spans {
        let start = text.len();
        text.push_str(&span.text);
        if !(span.strong || span.emphasis || span.code) {
            continue;
        }
        let style = HighlightStyle {
            font_weight: span.strong.then_some(FontWeight::BOLD),
            font_style: span.emphasis.then_some(FontStyle::Italic),
            // Emphasis is also dimmed: it is how an example reads apart from
            // the definition above it, in every theme's own muted colour.
            color: (span.emphasis || span.code).then(|| color(colors.text_muted)),
            background_color: span.code.then(|| color(colors.selection_background)),
            ..Default::default()
        };
        highlights.push((start..text.len(), style));
    }
    StyledText::new(text).with_highlights(highlights)
}

fn render_metadata(row: &MetadataRow, colors: &Colors) -> AnyElement {
    div()
        .flex()
        .gap(px(layout::view::DETAIL_METADATA_GAP))
        .text_size(type_scale::footer())
        .child(
            div()
                .flex_none()
                .text_color(color(colors.text_muted))
                .child(row.label.clone()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_color(color(tone_foreground(row.tone, colors)))
                .child(row.value.clone()),
        )
        .into_any_element()
}

/// A section's header: the title, and the subtitle after it in the same row
/// and the same size, only dimmer -- so a header with a subtitle is exactly
/// as tall as one without (I-10).
pub(crate) fn render_section(
    title: SharedString,
    subtitle: Option<SharedString>,
    colors: &Colors,
    geometry: &Geometry,
) -> AnyElement {
    div()
        .h(geometry.section_height())
        .px(geometry.gutter())
        .flex()
        .items_center()
        .gap(px(layout::view::SECTION_SUBTITLE_GAP))
        .overflow_hidden()
        .text_size(geometry.text_section())
        .font_weight(geometry.weight_section())
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

/// A row's title and subtitle, placed as the style says: one under the
/// other, or side by side with the subtitle muted after the title (F15).
///
/// Shared by the root list and a view's list, which must not look like two
/// applications (AD-07). The title arrives drawn, so the root list can light
/// the characters that matched. Inline, the title keeps its width and the
/// subtitle is what gives way; stacked, each truncates on its own line.
pub fn row_text(
    title: AnyElement,
    subtitle: Option<SharedString>,
    colors: &Colors,
    geometry: &Geometry,
) -> AnyElement {
    let subtitle = subtitle.map(|subtitle| {
        div()
            .text_size(geometry.text_subtitle())
            .text_color(color(colors.text_muted))
            // A file's subtitle is a path and can be longer than the window.
            // Ending it in an ellipsis is the difference between a row that
            // is too long and a row cut through the middle of a letter.
            .truncate()
            .child(subtitle)
    });
    let text = div().flex().flex_1().overflow_hidden();
    match geometry.variants().row_subtitle {
        RowSubtitle::Stacked => text.flex_col().child(title).children(subtitle),
        // The title at its own width and the subtitle in what is left; a
        // title longer than the row shrinks into its ellipsis, since the
        // subtitle's basis is nothing and is never what gives way first.
        // `flex_none` here cut a long title through a letter (F16).
        RowSubtitle::Inline => text
            .items_baseline()
            .gap(geometry.subtitle_gap())
            // `truncate`, not `overflow_hidden` alone: the ellipsis is a
            // text style, inherited by the drawn title inside.
            .child(div().min_w(px(0.0)).truncate().child(title))
            .children(subtitle.map(|subtitle| subtitle.flex_1().min_w(px(0.0)))),
    }
    .into_any_element()
}

/// One item row.
///
/// Structurally the same as a root-list row and for the same reasons: a fixed
/// height whether or not there is a subtitle, an icon square that exists before
/// the icon does, and the accessories in a column that cannot be pushed around
/// by the text beside them (I-10).
fn render_item(
    item: &Item,
    selected: bool,
    colors: &Colors,
    geometry: &Geometry,
    images: &ImageCache,
    icons: &IconCache,
    cx: &mut App,
) -> AnyElement {
    // Inset by a wrapper's padding: `list` ignores an item's margins.
    div()
        .px(geometry.row_inset())
        .child(
            div()
                .h(geometry.row_height())
                .w_full()
                .px(geometry.gutter() - geometry.row_inset())
                .flex()
                .items_center()
                .gap(geometry.icon_gap())
                .rounded(geometry.row_radius())
                .when(selected, |row| {
                    row.bg(color(colors.selection_background))
                        .border_1()
                        .border_color(geometry.selection_border(colors))
                })
                .when(!selected, |row| {
                    row.border_1().border_color(gpui::transparent_black())
                })
                .child(render_icon(
                    item.icon.as_ref(),
                    geometry.icon_size(),
                    colors,
                    geometry,
                    images,
                    icons,
                    cx,
                ))
                .child(row_text(
                    div()
                        .text_size(geometry.text_title())
                        .font_weight(geometry.weight_title())
                        .text_color(color(colors.text_primary))
                        .truncate()
                        .child(title(item, colors))
                        .into_any_element(),
                    item.subtitle.clone().map(Into::into),
                    colors,
                    geometry,
                ))
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
                                .map(|accessory| render_accessory(accessory, colors, geometry)),
                        ),
                ),
        )
        .into_any_element()
}

/// One strip of a grid (F18): square cells a gap apart from the gutter on,
/// with the gap below them, so every strip is the same height (I-10).
#[allow(clippy::too_many_arguments)]
fn render_strip(
    cells: &[usize],
    items: &[Item],
    selected: usize,
    colors: &Colors,
    geometry: &Geometry,
    images: &ImageCache,
    icons: &IconCache,
    cx: &mut App,
) -> AnyElement {
    div()
        .h(geometry.grid_cell() + geometry.grid_gap())
        .px(geometry.gutter())
        .flex()
        .gap(geometry.grid_gap())
        .children(cells.iter().map(|&index| {
            let selected = index == selected;
            div()
                .size(geometry.grid_cell())
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(geometry.grid_radius())
                .border_1()
                .when(selected, |cell| {
                    cell.bg(color(colors.selection_background))
                        .border_color(geometry.selection_border(colors))
                })
                .when(!selected, |cell| {
                    cell.bg(color(colors.surface_elevated))
                        .border_color(gpui::transparent_black())
                })
                .child(render_glyph(
                    items.get(index).and_then(|item| item.icon.as_ref()),
                    colors,
                    geometry,
                    images,
                    icons,
                    cx,
                ))
        }))
        .into_any_element()
}

/// A cell's glyph, centred at its share of the cell. On its own so a
/// picture of the emoji can take the text's place later.
fn render_glyph(
    icon: Option<&Icon>,
    colors: &Colors,
    geometry: &Geometry,
    images: &ImageCache,
    icons: &IconCache,
    cx: &mut App,
) -> AnyElement {
    let size = px(layout::view::grid_glyph(geometry.metrics().grid_cell));
    match icon {
        Some(Icon::Text { text }) => div()
            .size(size)
            .flex()
            .items_center()
            .justify_center()
            .text_size(size)
            .line_height(size)
            .text_color(color(colors.text_primary))
            .child(text.clone())
            .into_any_element(),
        icon => render_icon(icon, size, colors, geometry, images, icons, cx),
    }
}

/// A row's title, after its prefix dimmed when it has one (F16): one run of
/// text, so the two truncate together into one ellipsis.
fn title(item: &Item, colors: &Colors) -> StyledText {
    let Some(prefix) = item.prefix.as_deref().filter(|prefix| !prefix.is_empty()) else {
        return StyledText::new(item.title.clone());
    };
    let dimmed = HighlightStyle {
        color: Some(color(colors.text_muted)),
        ..Default::default()
    };
    StyledText::new(format!("{prefix}{}", item.title)).with_highlights([(0..prefix.len(), dimmed)])
}

/// An icon in a square of `size`, which is reserved whether or not there is
/// one (I-10). A row's is the style's icon size; the action panel's is its own.
pub(crate) fn render_icon(
    icon: Option<&Icon>,
    size: gpui::Pixels,
    colors: &Colors,
    geometry: &Geometry,
    images: &ImageCache,
    icons: &IconCache,
    cx: &mut App,
) -> AnyElement {
    let square = div()
        .size(size)
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
                    .size(size)
                    .text_color(color(colors.icon)),
            )
            .into_any_element(),
        Some(Icon::Text { text }) => square
            .text_size(geometry.text_title())
            .text_color(color(colors.icon))
            .child(text.clone())
            .into_any_element(),
        Some(Icon::Swatch { color: value }) => square
            .child(swatch(value, px(layout::view::SWATCH_SIZE), colors))
            .into_any_element(),
        // The square stays empty while the thumbnail loads, like an
        // application's icon (I-10).
        Some(Icon::Image { path }) => square
            .when_some(
                images.get(path, geometry.metrics().icon_resolution(), Fit::Square, cx),
                |square, image| {
                    square.child(
                        img(image)
                            .size(size)
                            .rounded(px(layout::view::THUMBNAIL_RADIUS)),
                    )
                },
            )
            .into_any_element(),
        // The category's glyph until the shell's icon arrives, and for good
        // when it cannot: a file always has something in its square.
        Some(Icon::File {
            path,
            folder,
            fallback,
        }) => match icons.file(path, *folder, cx) {
            Some(image) => square.child(img(image).size(size)),
            None => square.child(
                svg()
                    .path(format!("icons/{fallback}.svg"))
                    .size(size)
                    .text_color(color(colors.icon)),
            ),
        }
        .into_any_element(),
        None => square.into_any_element(),
    }
}

/// A copied colour, drawn as itself (F12). The one colour that is data rather
/// than a theme token: the I-5 exception `Icon::Swatch` documents. The
/// border is the theme's, so a colour close to the background still shows
/// where it ends. A value that does not parse draws nothing.
fn swatch(value: &str, size: gpui::Pixels, colors: &Colors) -> gpui::Div {
    let square = div().size(size).rounded(px(layout::view::SWATCH_RADIUS));
    match lumen_core::theme::Color::parse(value) {
        Ok(fill) => square
            .bg(color(fill))
            .border_1()
            .border_color(color(colors.border_strong)),
        Err(_) => square,
    }
}

fn render_accessory(accessory: &Accessory, colors: &Colors, geometry: &Geometry) -> AnyElement {
    match accessory {
        // What the row exists to show reads like its title (F13); the rest
        // is dimmed beside it.
        Accessory::Text { text, strong } => div()
            .text_size(if *strong {
                geometry.text_title()
            } else {
                geometry.text_accessory()
            })
            .text_color(color(if *strong {
                colors.text_primary
            } else {
                colors.text_muted
            }))
            .child(text.clone())
            .into_any_element(),
        Accessory::Tag { text, tone } => render_tag(text.clone().into(), *tone, colors, geometry),
    }
}

/// Text in a pill: a view's tag, and a root row's alias (F17).
pub(crate) fn render_tag(
    text: SharedString,
    tone: Tone,
    colors: &Colors,
    geometry: &Geometry,
) -> AnyElement {
    div()
        .px(px(layout::view::TAG_PADDING_X))
        .py(px(layout::view::TAG_PADDING_Y))
        .rounded(px(layout::view::TAG_RADIUS))
        .bg(color(tone_background(tone, colors)))
        .text_size(geometry.text_accessory())
        .text_color(color(tone_foreground(tone, colors)))
        .child(text)
        .into_any_element()
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

/// How much of the match colour the tint behind a preview card's match
/// keeps: enough to find the match at a glance, little enough that the
/// match colour on it still reads.
const MATCH_ALPHA: u8 = 0x30;

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
            Icon::Swatch { color: value } => {
                slot.child(swatch(value, px(layout::view::EMPTY_ICON_SIZE), colors))
            }
            // No empty state is a picture; a host view that tried gets none.
            Icon::Image { .. } | Icon::File { .. } => slot,
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

/// Draw a detail view: its Markdown and metadata, as the pane beside a list
/// draws an item's (F10), over the whole width.
pub fn render_detail(detail: &Detail, theme: &Theme, height: gpui::Pixels) -> AnyElement {
    div()
        .id("detail")
        .h(height)
        .px(Geometry::of(theme).gutter())
        .py(px(layout::view::DETAIL_PADDING_Y))
        .overflow_y_scroll()
        .child(render_body(
            &detail.markdown,
            None,
            &detail.metadata,
            &theme.colors,
        ))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_core::view::{Presentation, Section, View};

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

    /// A grid of five titled sections, 0, 1, 8, 9 and 17 items long.
    fn grid() -> List {
        let mut next = 0;
        let sections = [0, 1, 8, 9, 17]
            .into_iter()
            .map(|count| Section {
                title: Some(format!("{count}")),
                subtitle: Some(format!("{count}")),
                items: (0..count)
                    .map(|_| {
                        next += 1;
                        item(&next.to_string())
                    })
                    .collect(),
            })
            .collect();
        List {
            sections,
            presentation: Presentation::Grid,
            ..Default::default()
        }
    }

    /// What a row holds, as a short string a test can compare: `H` for a
    /// header, `i3` for an item, `[8 9 10]` for a strip of cells.
    fn shape(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Section(..) => "H".to_string(),
                Row::Item(index) => format!("i{index}"),
                Row::Cells(cells) => format!(
                    "[{}]",
                    cells
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
            })
            .collect()
    }

    /// A grid's section is cut into strips of as many cells as there are
    /// columns, the last one short; a strip never runs into the next section.
    #[test]
    fn a_grid_section_is_cut_into_strips_of_the_column_count() {
        let rows = rows(&grid(), 8);

        assert_eq!(
            shape(&rows),
            [
                "H",
                "H",
                "[0]",
                "H",
                "[1 2 3 4 5 6 7 8]",
                "H",
                "[9 10 11 12 13 14 15 16]",
                "[17]",
                "H",
                "[18 19 20 21 22 23 24 25]",
                "[26 27 28 29 30 31 32 33]",
                "[34]",
            ]
        );
    }

    /// The header keeps the subtitle the view gave it: the section's count.
    #[test]
    fn a_grid_header_keeps_its_subtitle() {
        let rows = rows(&grid(), 8);
        let Row::Section(title, subtitle) = &rows[3] else {
            panic!("a header")
        };
        assert_eq!(title.as_ref(), "8");
        assert_eq!(subtitle.as_ref().map(AsRef::as_ref), Some("8"));
    }

    /// A list of rows ignores the column count: one item, one row, as before.
    #[test]
    fn a_list_of_rows_is_not_cut_into_strips() {
        let mut list = grid();
        list.presentation = Presentation::Rows;

        for columns in [1, 8] {
            let rows = rows(&list, columns);
            assert_eq!(rows.len(), 5 + 35);
            assert!(rows.iter().all(|row| !matches!(row, Row::Cells(_))));
        }
        assert_eq!(shape(&rows(&example(), 8)), ["H", "i0", "i1", "i2"]);
    }

    /// A filtered grid packs what survives into strips, whatever the gaps
    /// in the flat order, and drops a header with nothing under it.
    #[test]
    fn a_filtered_grid_packs_the_survivors() {
        let rows = rows_where(&grid(), 3, |index| index % 2 == 0 && index < 20);

        assert_eq!(
            shape(&rows),
            [
                "H",
                "[0]",
                "H",
                "[2 4 6]",
                "[8]",
                "H",
                "[10 12 14]",
                "[16]",
                "H",
                "[18]",
            ]
        );
    }

    /// The selection is a flat item index; the row it is in is the strip
    /// that holds it.
    #[test]
    fn the_selected_item_is_found_in_its_strip() {
        let rows = rows(&grid(), 8);

        assert_eq!(row_of(&rows, 0), Some(2));
        assert_eq!(row_of(&rows, 1), Some(4));
        assert_eq!(row_of(&rows, 8), Some(4));
        assert_eq!(row_of(&rows, 9), Some(6));
        assert_eq!(row_of(&rows, 16), Some(6));
        assert_eq!(row_of(&rows, 17), Some(7));
        assert_eq!(row_of(&rows, 34), Some(11));
        assert_eq!(row_of(&rows, 35), None);
        assert_eq!(row_of(&super::rows(&example(), 8), 1), Some(2));
    }

    /// A strip is a cell and a gap tall, every one of them, the short last
    /// one included (I-10); a header is as tall as in a list of rows.
    #[test]
    fn a_strip_is_a_cell_and_a_gap_tall() {
        let metrics = Metrics::BASE;
        let strip = metrics.grid_cell + metrics.grid_gap;
        let rows = rows(&grid(), 8);

        assert_eq!(row_height(&rows[7], &metrics), strip);
        assert_eq!(row_height(&rows[6], &metrics), strip);
        assert_eq!(row_height(&rows[0], &metrics), metrics.section_height);
        let total: f32 = rows.iter().map(|row| row_height(row, &metrics)).sum();
        assert_eq!(total, 5.0 * metrics.section_height + 7.0 * strip);
    }

    /// Revealing the last cell from the top scrolls the list so its strip's
    /// bottom is the viewport's: the sum of the heights above it.
    #[test]
    fn revealing_a_cell_below_the_fold_scrolls_to_its_strip() {
        let metrics = Metrics::BASE;
        let rows = rows(&grid(), 8);
        let height = |row: &Row| row_height(row, &metrics);
        let viewport = metrics.section_height * 2.0 + (metrics.grid_cell + metrics.grid_gap) * 3.0;

        let target = row_of(&rows, 34).unwrap();
        let (index, offset) = reveal(&rows, &metrics, (0, 0.0), target, viewport);
        let top: f32 = rows[..index].iter().map(height).sum::<f32>() + offset;
        let total: f32 = rows.iter().map(height).sum();
        assert_eq!(top, total - viewport);

        // Back up to the first cell of a section: its header shows too.
        let first = row_of(&rows, 9).unwrap();
        assert_eq!(reveal(&rows, &metrics, (9, 0.0), first, viewport), (5, 0.0));
    }

    /// The columns are the ones that fit in the window less the gutter
    /// either side, and in the list's share of it beside a detail pane.
    #[test]
    fn a_grid_takes_the_columns_that_fit_inside_the_gutters() {
        let metrics = Metrics::BASE;
        let mut list = grid();
        let width = |columns: f32| {
            columns * metrics.grid_cell + (columns - 1.0) * metrics.grid_gap + 2.0 * metrics.gutter
        };

        assert_eq!(grid_columns(&list, &metrics, width(8.0)), 8);
        assert_eq!(grid_columns(&list, &metrics, width(8.0) - 1.0), 7);
        list.showing_detail = true;
        let pane = (width(8.0) - 2.0 * metrics.gutter) / layout::view::DETAIL_LIST_SHARE
            + 2.0 * metrics.gutter;
        assert!(grid_columns(&list, &metrics, pane) < 8);
    }

    /// A preview card is in the palette's monospace family unless the user
    /// prefers the interface family, which inherits.
    #[test]
    fn a_card_is_monospace_unless_the_interface_font_is_preferred() {
        let mut theme = Theme::default();
        assert_eq!(
            card_font(&theme, PreviewFont::Monospace),
            Some("Cascadia Mono")
        );
        theme.font_mono = "Consolas".into();
        assert_eq!(card_font(&theme, PreviewFont::Monospace), Some("Consolas"));
        assert_eq!(card_font(&theme, PreviewFont::Interface), None);
    }

    /// A match in a preview card is the match colour on a tint of itself,
    /// in bold: a change of colour alone is easy to read past.
    #[test]
    fn a_card_match_is_tinted_coloured_and_bold() {
        let colors = Theme::default().colors;
        let style = card_match(&colors);
        assert_eq!(style.color, Some(color(colors.match_highlight)));
        assert_eq!(
            style.background_color,
            Some(color(colors.match_highlight.with_alpha(MATCH_ALPHA)))
        );
        assert_eq!(style.font_weight, Some(FontWeight::BOLD));
    }

    /// The card's scroll position belongs to one row's preview: it starts at
    /// the top again when the selection or the text it shows changes, and
    /// is kept across renders of the same one.
    #[test]
    fn a_card_scrolls_back_to_the_top_only_for_a_new_preview() {
        let key = card_scroll_key(3, "fn main() {}");
        assert_eq!(key, card_scroll_key(3, "fn main() {}"));
        assert_ne!(key, card_scroll_key(4, "fn main() {}"));
        assert_ne!(key, card_scroll_key(3, "fn other() {}"));
    }

    /// A section without a title contributes no header. The alternative is a
    /// blank band of nothing where a label should be.
    #[test]
    fn only_titled_sections_produce_a_header() {
        let rows = rows(&example(), 1);
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
        let rows = rows(&example(), 1);
        let items: Vec<usize> = rows.iter().flat_map(Row::items).copied().collect();
        assert_eq!(items, [0, 1, 2]);
    }

    /// Down past the bottom edge scrolls by exactly what is needed, whether
    /// or not the row was ever drawn -- the F10 bug was that it did not move.
    #[test]
    fn revealing_a_row_below_the_fold_scrolls_just_enough() {
        // Header, five items, header, six items: the Dictionary view for `display`.
        let mut rows = vec![Row::Section("Noun".into(), None)];
        rows.extend((0..5).map(Row::Item));
        rows.push(Row::Section("Verb".into(), None));
        rows.extend((5..11).map(Row::Item));
        let metrics = Metrics::BASE;
        let header = metrics.section_height;
        let item = metrics.row_height;
        let row_height = |row: &Row| row_height(row, &metrics);
        // What the launcher gives it: two headers and six items.
        let viewport = header * 2.0 + item * 6.0;

        // Visible already: nothing moves.
        assert_eq!(
            reveal(&rows, &Metrics::BASE, (0, 0.0), 7, viewport),
            (0, 0.0)
        );

        // The second verb, one row below the edge: scroll by one item.
        let (index, offset) = reveal(&rows, &Metrics::BASE, (0, 0.0), 8, viewport);
        let top: f32 = rows[..index].iter().map(row_height).sum::<f32>() + offset;
        assert_eq!(top, item);

        // The last row, from the top.
        let (index, offset) = reveal(&rows, &Metrics::BASE, (0, 0.0), 12, viewport);
        let top: f32 = rows[..index].iter().map(row_height).sum::<f32>() + offset;
        let total: f32 = rows.iter().map(row_height).sum();
        assert_eq!(top, total - viewport);
    }

    #[test]
    fn moving_up_to_a_sections_first_item_shows_its_header() {
        let mut rows = vec![Row::Section("Noun".into(), None)];
        rows.extend((0..5).map(Row::Item));
        rows.push(Row::Section("Verb".into(), None));
        rows.extend((5..11).map(Row::Item));

        // Scrolled so the first verb (row 7) is above the top edge.
        assert_eq!(reveal(&rows, &Metrics::BASE, (9, 0.0), 7, 200.0), (6, 0.0));
        // And back to the very first item: its header too.
        assert_eq!(reveal(&rows, &Metrics::BASE, (9, 0.0), 1, 200.0), (0, 0.0));
    }

    /// Carbon's numbers: a list 371 tall holds nine 38 px rows, 342.
    fn carbon() -> (Metrics, f32, f32) {
        let metrics = Metrics {
            row_height: 38.0,
            section_height: 32.0,
            list_height: 371.0,
            ..Metrics::BASE
        };
        (metrics, 371.0, 9.0 * 38.0)
    }

    #[test]
    fn a_pane_whose_rows_fit_is_as_tall_as_a_full_root_list() {
        let (metrics, cap, full) = carbon();
        let rows = vec![Row::Section("Today".into(), None), Row::Item(0)];

        assert_eq!(rows_height(&rows, Shape::Pane, &metrics, cap, full), full);
    }

    #[test]
    fn a_detail_view_is_as_tall_as_a_full_root_list() {
        let (metrics, cap, full) = carbon();

        assert_eq!(rows_height(&[], Shape::Detail, &metrics, cap, full), full);
    }

    /// Two headers and eight rows fit in 371 and are taller than nine rows:
    /// the pane grows to them rather than clipping the last.
    #[test]
    fn a_pane_never_clips_rows_that_fit() {
        let (metrics, cap, full) = carbon();
        let mut rows = vec![Row::Section("Today".into(), None)];
        rows.extend((0..4).map(Row::Item));
        rows.push(Row::Section("Yesterday".into(), None));
        rows.extend((4..8).map(Row::Item));

        let height = rows_height(&rows, Shape::Pane, &metrics, cap, full);

        assert_eq!(height, 2.0 * 32.0 + 8.0 * 38.0);
        assert!(height > full && height <= cap);
    }

    /// Rows that overflow end on a whole row within the style's height, in a
    /// list and in a pane alike: what the root list does.
    #[test]
    fn overflowing_rows_end_on_a_whole_row_within_the_cap() {
        let (metrics, cap, full) = carbon();
        let mut rows = vec![Row::Section("Today".into(), None)];
        rows.extend((0..20).map(Row::Item));

        for shape in [Shape::List, Shape::Pane] {
            assert_eq!(
                rows_height(&rows, shape, &metrics, cap, full),
                32.0 + 8.0 * 38.0
            );
        }
    }

    #[test]
    fn a_short_list_is_as_tall_as_its_rows() {
        let (metrics, cap, full) = carbon();
        let rows = vec![Row::Item(0), Row::Item(1)];

        assert_eq!(
            rows_height(&rows, Shape::List, &metrics, cap, full),
            2.0 * 38.0
        );
    }

    /// A view that is still loading has no rows yet, and neither has one whose
    /// search matched nothing: both are as tall as a full root list, so the
    /// window does not grow when the rows arrive or shrink when a search
    /// empties the list.
    #[test]
    fn a_loading_or_empty_view_is_as_tall_as_a_full_root_list() {
        let (metrics, cap, full) = carbon();

        for shape in [Shape::List, Shape::Pane] {
            assert_eq!(rows_height(&[], shape, &metrics, cap, full), full);
        }
    }

    #[test]
    fn a_header_is_shorter_than_a_row() {
        let metrics = Metrics::BASE;
        assert!(
            row_height(&Row::Section("x".into(), None), &metrics)
                < row_height(&Row::Item(0), &metrics)
        );
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
        assert_eq!(rows(&list, 1).len(), 12);

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
