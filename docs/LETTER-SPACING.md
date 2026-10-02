# Letter spacing

**Status: done.** The gpui patch is commit `8412ea5`, the `text.tracking`
style token `5907c17`, Graphite's tracking `f62234f` and the presets' type
sizes `515919b` (see "What was done differently"). The rest of this file is the plan as it was written, kept as
history; the section below says where the build departed from it.

**Why.** Raycast and Vicinae set their interface in Inter, the face Lumen
bundles (F15 decisions). What still differs on screen is partly the
rasteriser, which nothing reaches, and partly tracking: web text is commonly
set with a little positive letter spacing, and gpui 0.2.2 has none. A style
cannot ask for what the renderer cannot do.

**Estimate.** One slice for the gpui patch with its tests, one for the style
token and the presets. The patch is larger than the two Lumen already carries
(7.35), and it has to be carried across every gpui upgrade (I-6).

---

## What was done differently

- **`LetterSpacing(f32)`, not `Option<Pixels>`.** A newtype in em with a
  manual `Eq` and `Hash` (`-0.0` and NaN fold to zero), because `TextRun`
  derives `Eq` and `FontRun` is a hash key. The fields on `TextStyle` and
  `TextRun` are plain, not optional: zero means none.
- **Converted to pixels in `layout_line`,** from the run's own font size,
  after shaping and after the `force_width` block, so the grid snap compares
  unspaced positions. A glyph's offset is the sum of the spacing of every
  glyph before it, each matched to its run by glyph index, which copes with
  RTL order. `FontRun` merging requires equal spacing, and when every run is
  zero the step returns at once.
- **`LineWrapper`** keeps its per-character caches unspaced and
  `width_for_char` adds the spacing. `line_wrapper_with_letter_spacing()`
  resets a pooled wrapper's spacing on checkout. Wrapped lines go through
  `layout_line`, so they take the step too.
- **The line's width includes the spacing after the last glyph,** as CSS does.
- **Ligatures:** one glyph, moved as one. Font features are unchanged -- there
  is no `liga` off with tracking. Still an open point, to decide by looking
  at tracked text.
- **Not updated:** the macOS-only `FontRun` literals in
  `vendor/gpui/src/platform/mac/text_system.rs` tests. Lumen does not build
  for macOS.
- **Lumen:** `text.tracking` applies on the launcher root (`styling::tracking`,
  which sets nothing at 0) and is inherited by the query, rows, headers and
  footer. Keycaps are reset to none (`styling::untracked`). It is not in
  `styling::typography`: settings and the overlay do not read a style (7.41).
  The style schema now has 30 tokens.
- **Presets: the gap was size, not tracking.** Measured against new captures
  of Raycast, Asyar and Vicinae (1.25 scale, whole-word fits), the presets'
  type was larger than the references. Carbon moves to 13 pt for title,
  subtitle and accessory, 11 for section headers and 12 for the footer;
  Graphite's titles to 12.8 pt with tracking -0.02. Measured words then land
  within about a pixel of Raycast and 1.5-2.3 px of Asyar. Carbon and Ink keep
  tracking 0; Ink's words remain 1.7-4.6 px wide of Vicinae, unchanged here.
- **Typing frame cost,** Graphite with -0.02 against the same style at 0,
  alternated twice in an isolated daemon: p95 2.31 and 2.27 ms against 2.29
  and 2.29 ms. No difference outside noise; the budget is 8 ms.

---

## The mechanism

DirectWrite shapes a line and returns a `LineLayout`
(`vendor/gpui/src/text_system/line_layout.rs:16`): runs of `ShapedGlyph`s,
each with an x position, plus the line's width. That structure is
platform-independent. Letter spacing is applied **after shaping, in shared
code**: glyph *n* of the line moves right by *n* × spacing, and the line's
width grows by (glyph count) × spacing.

Consequences, all wanted:

- **Linux gets it for free** (F5): the step is not in `direct_write.rs`.
- **Caret, selection and hit-testing stay right.** `TextField` and
  `TextArea` find positions from the glyph positions of the shaped line
  (`x_for_index`, `index_for_x`), which the step has already moved.
- **Ligatures stay intact.** A ligature is one glyph, so it moves as one.
  Where tracking is set, the style should probably also turn `liga` off --
  tracked text with ligatures looks wrong. Decide during the slice.

## What has to change in gpui

Every changed line marked `LUMEN PATCH (docs/LETTER-SPACING.md)`, as 7.35's
are, in a commit of its own on top of the vendored crate.

1. **Carry it.** A `letter_spacing: Option<Pixels>` (or em, see below) on
   `TextStyle` (`style.rs`, beside `font_features`), copied by
   `TextStyle::to_run` (`style.rs:475`) into a new field on `TextRun`
   (`text_system.rs:733`). Absent means zero, so nothing changes until it is
   set. A `Styled::letter_spacing` setter alongside `font_family`.
2. **Key the cache with it.** `FontRun` (`line_layout.rs:597`) is part of the
   line-layout cache key (`CacheKey`, `line_layout.rs:607`) and today holds
   only `len` and `font_id`. Without spacing in the key, a line laid out at
   one spacing would be served for another.
3. **Apply it.** In `LineLayoutCache::layout_line`, after the platform's
   `layout_line` returns (`line_layout.rs:566`) and before the result is
   cached: walk the runs, offset each glyph by its count so far × the run's
   spacing, and add the total to `width`.
4. **Measure with it.** `LineWrapper` (`line_wrapper.rs`) measures characters
   on its own, through `width_for_char` (`:193`) and its caches, for wrapping
   and for `truncate_line` (`:132`), which is what draws an ellipsis. It has
   to add the spacing to each character's width, and its caches must be keyed
   by it too. Skipped, a long subtitle would be cut where the unspaced text
   would end -- a few pixels off, and visible.

`TextRun` literals that get the new field: gpui's own, at `style.rs:476`,
`line_wrapper.rs:334, 690, 698` and `text_system.rs:749`; Lumen's, at
`text_field.rs:659, 672, 676, 685` and `text_area.rs:1259, 1269, 1469`.

## What has to change in Lumen

- **A style token, not a palette one.** Spacing is a measurement, so it is
  geometry (AD-13, I-11). `text.tracking`, in **em**, so one number scales
  with every type size; converted to pixels per element as
  `size × tracking`. Base 0. Range about -0.05 to 0.1: beyond that text
  stops reading as words.
- **Where it applies.** On the launcher's root element, inherited like the
  family, so the query, rows, headers and footer all take it. The keycaps
  probably want none -- a two-letter key does not track well. Decide by
  looking.
- **`styling::typography`** is the one place the palette's typography is set
  on an element (F15); the style's tracking goes beside it, so the launcher,
  the overlay and settings cannot disagree.
- **Presets.** Measure before setting: compare a word's ink width in the
  reference captures (`target/sample.ps1`, `INK=`) with Lumen's at the same
  size. If the difference is inside a pixel, the preset keeps 0.

## Tests to write first

- In gpui: a line laid out with spacing *s* and *n* glyphs is
  `width + n × s` wide, and glyph *k* sits `k × s` right of where it was.
  The same text at two spacings gives two cache entries.
- `truncate_line` with spacing cuts earlier than without.
- In Lumen: `text.tracking` is bounded and resolved like every other token;
  the base style is pixel-identical to before (the capture and diff used in
  F15, slice 3).
- Typing frame cost (`scripts/measure-typing.ps1`) against the F15 close,
  since the step runs on every newly shaped line.

## Not this

- **DirectWrite's own tracking** (`IDWriteTextLayout1::SetCharacterSpacing`):
  gpui does not use `IDWriteTextLayout` for shaping, and it would be
  Windows-only.
- **Spacing on `Font`.** It would look simpler, but `Font` is what font ids
  are resolved from, so every spacing would load the face again.
