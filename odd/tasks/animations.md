# Animations: launcher and action popover

Raycast-like opening: fade + scale + a small overshoot ("bounce"), for the
launcher and the Ctrl+K popover. Configurable and switchable.

Branch: `feat/animations` (from `main` at `2bf727b`).

## Decisions (user)

- On/off lives in settings: `on` / `off` / `system` (default `system`, which
  honours Windows' "Animation effects" accessibility switch).
- Tuning is per surface (launcher, popover) in the style, as motion tokens
  within ranges: duration, start scale, overshoot, slide, start opacity.
- Closing is instant (Escape never waits on an animation).

## Approach

GPUI 0.2.2 has no transform for `div` (only `svg`). Every primitive passes
through `Window::paint_*` (vendor/gpui/src/window.rs ~2806-3129), which
already applies DPI scale and an element-opacity stack. Add a uniform
element-scale stack there as a vendored patch (ARCHITECTURE 7.35 pattern):
no shader changes, no per-frame layout.

## Tasks

- [x] 1. measure: per-frame cost during the open animation (I-8, before the feature)
- [x] 2. gpui patch: element scale stack + `div` API, documented, tested
- [ ] 3. platform: read the Windows animation-effects switch (trait Platform, I-9)
- [ ] 4. config + settings window: `animations = on | off | system`
- [ ] 5. style: motion tokens per surface, with ranges and base defaults (I-11, I-12)
- [ ] 6. launcher opening: fade + scale + overshoot from tokens
- [ ] 7. popover opening: same, scaled from its anchor; window growth handled
- [ ] 8. build, reinstall, user tunes on screen

## Evidence

- `1c8dacd` test(ui): source-scanning tests normalise CRLF (autocrlf checkout
  broke `only_functions_that_resize_clear_the_action_panel`).
- Task 1: `c7b5489` scripts/measure-appear.ps1. Baseline fade-only, 20 summons:
  p50 1.18 ms, p95 1.43 ms (after first frame); patched, no scale: p95 1.44 ms.
- Task 2: `a97afc8` `Styled::scale` / `scale_about(factor, origin fraction)`,
  `ElementScale`, `Window::with_element_scale`; paint_* + content masks;
  ARCHITECTURE 7.49/7.50. 11 gpui tests pass via
  `CARGO_TARGET_DIR=target/gpui-test cargo test --offline --manifest-path vendor/gpui/Cargo.toml --lib element_scale`
  (delete the generated vendor/gpui/Cargo.lock after).
  Risk for task 7: deferred elements escape the scale, as they escape opacity.
