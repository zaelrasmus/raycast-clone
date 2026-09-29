# The icon set

Ten glyphs, drawn for this project, MIT OR Apache-2.0 with everything else.

## The grid

Every icon is a `20 × 20` viewBox with its drawing confined to the inner
`16 × 16` — a two-unit margin on every side. That is the "optically aligned on
a 16 and 20 px grid" of F2, and it is what lets a glyph sit in the same square
as an application icon extracted from the shell without one looking larger than
the other.

Two units of margin rather than none because a shell icon is drawn to its own
edges. A stroked glyph at the same nominal size reads as bigger: its corners
are further from the centre and there is no shading to soften them.

## The rules

- **Stroke 1.5, never filled.** A filled glyph beside a stroked one reads as
  emphasis nobody asked for. `fill="none"` on every path.
- **Round caps and joins.** The interface has an 8-unit corner radius
  everywhere else; square-cut strokes fight it.
- **`currentColor`.** GPUI paints an SVG as a mask in the element's text
  colour, so the value here is never seen — but it is what makes these files
  correct on their own, and it is what the theme's `icon` token drives.
- **Coordinates on the half-unit.** A 1.5 stroke centred on a whole coordinate
  straddles the pixel grid and blurs at 100% scaling. Horizontal and vertical
  edges sit on `.5` so the stroke lands on whole pixels.
- **Optical weight before geometric truth.** The bolt is not centred on its
  bounding box; it is centred on where the eye puts it.

## Which glyph is which

| File | Used for |
| ---- | -------- |
| `application.svg` | An application with no icon of its own |
| `command.svg` | Lumen's own commands |
| `calculator.svg` | An arithmetic answer |
| `script.svg` | A tier-T1 script command |
| `snippet.svg` | A saved block of text |
| `clipboard.svg` | Clipboard history |
| `window.svg` | A window layout |
| `file.svg` | A file |
| `link.svg` | A quicklink |
| `timer.svg` | A timer, and the *Timers* view (F8) |
