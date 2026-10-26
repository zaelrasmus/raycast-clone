# Letter spacing

Branch: `feat/letter-spacing` (off `main` at `af0afd3`).
Source plan: `docs/LETTER-SPACING.md`.

## Decisions

- gpui carries spacing in **em** on `TextStyle`/`TextRun`; `LineLayoutCache::layout_line`
  converts it to pixels with the `font_size` it already receives, so one value set on the
  launcher root scales with every type size beneath it.
- The Lumen token is `text.tracking` in the style (geometry, I-11), not the palette.
  Per 7.41 only the launcher's surface reads a style.

## Tasks

- [x] 1. gpui patch, test-first: `letter_spacing` on `TextStyle` -> `TextRun` -> `FontRun`
      (cache key); applied after shaping in `layout_line`; `LineWrapper` measures with it;
      `Styled::letter_spacing` setter; every line marked `LUMEN PATCH (docs/LETTER-SPACING.md)`;
      Lumen's `TextRun` literals updated. Commit.
- [x] 2. Lumen style token `text.tracking` (em, base 0, bounded about -0.05..0.1), applied on
      the launcher root, keycaps decided by looking; base style pixel-identical. Commit.
- [x] 3. (superseded by 3a/3b) Measure presets (Carbon, Graphite, Ink) against reference captures; set values only
      where the difference exceeds a pixel. Typing frame cost against the F15 close. Commit.
- [x] 4. Docs: LETTER-SPACING.md status, ARCHITECTURE decision, README line 495. Commit.
- [x] 3a. (resumed) With the user's references in `C:\Users\kiza2\Pictures\Screenshots`
      (Raycast `Screenshot 2026-10-01 111232.png`, Vicinae `...110901.png`, Asyar
      `...105301.png`): capture Lumen per preset under `LUMEN_HOME`, measure ink widths, set
      tracking where the difference exceeds a pixel. Commit.
- [x] 3b. Typing frame cost with a tracked style vs the F15 close (p95 2.00 ms). Commit docs.
- [x] 5. Release build and reinstall on the user's machine (`scripts/install.ps1`).

## Evidence

- Task 1: `8412ea5`. RED: 5 of 6 new letter-spacing tests failed on values before the
  change. GREEN (independent verify): `cargo fmt --check`, clippy `-D warnings` clean;
  `cargo test --all` 1063 passed / 0 failed; vendor gpui `text_system` 11 passed / 0 failed.
  Open for later: macOS-only `FontRun` literals in `platform/mac/text_system.rs` lack the
  field (out of scope); ligatures move as one glyph; line width includes trailing spacing
  (as CSS). Run vendor tests with `CARGO_TARGET_DIR=D:/Dev/raycast-clone/target`.
- Task 2: `5907c17`. Token `text.tracking` in `lumen-core/src/style.rs` (base 0, clamp
  -0.05..0.1, TOKEN_COUNT 30); applied at `launcher.rs` root via `styling::tracking`
  (sets nothing at 0); keycaps reset via `styling::untracked`. RED was compile-level for
  core tests; UI helper tests written alongside. Verify: fmt/clippy clean, 1069 passed / 0
  failed. Not yet looked at on screen.
- Task 3: deferred, no commit. The reference captures and `target/sample.ps1` are no longer
  in `target/`, so preset values cannot be measured; Carbon/Graphite/Ink keep 0.
  `scripts/measure-typing.ps1` force-kills the user's running `lumen.exe` (line 71) and
  types via SendKeys, so it was not run unattended. At tracking 0 the patch returns early
  (`line_layout.rs:607`). To finish: recreate reference captures, measure ink widths, set
  values over a pixel; run measure-typing with a tracked style against F15 close p95 2.00 ms.
- Task 4: `95391aa`. LETTER-SPACING.md status + deviations, ARCHITECTURE 7.42, schema row,
  F15 row (30 tokens), README line 495.
- Task 3a: `f62234f` Graphite tracking -0.02; `515919b` sizes (Carbon 13/13/13/11/12, Graphite
  title+subtitle 12.8), words within ~1 px of Raycast, 1.5-2.3 px of Asyar; Ink unchanged
  (1.7-4.6 px wide, open). `cargo test --all` all ok.
- Task 3b: isolated A/B, p95 2.31/2.27 ms tracked vs 2.29/2.29 untracked. Docs `a13a366`.
- Task 5: `scripts/install.ps1`, 20.5 MB, daemon running; user's carbon/graphite styles match
  the release. User config hash unchanged across all test runs.
