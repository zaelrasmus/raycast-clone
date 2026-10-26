# Action panel as a Raycast-style popover

**Closed:** fast-forwarded into `main` at `2bf727b` (12 commits); feature
branches deleted locally; not pushed. Animation work continues in
`odd/tasks/animations.md` on `feat/animations`.

Rework Ctrl+K: fix the Escape height bug, give actions groups/icons/shortcuts,
and draw the panel as a popover over the list with its own search field.

Reference: Raycast screenshots (application vs command action panels) --
title of the target, separators between groups, icon per row, keycap
shortcuts, "Search for actions..." field, anchored bottom-right.

## Tasks

- [x] 1. fix: Escape (and every panel-closing path) restores the window height
  - `clear_query` panel branch (launcher.rs ~2832) never calls `resize_if_needed`
  - audit the other `action_panel = None` sites; add a regression test
  - branch `fix/action-panel-escape-height`
- [x] 2. feat(core): action metadata -- group, icon, shortcut per action
  - `lumen_core::item::Action` gains presentation metadata; distinct sets for
    applications and commands; shortcuts stay bound in the keymap (I-14)
  - branch `feat/action-metadata`
- [x] 3. feat(ui): popover over the list
  - explicit computed height (taffy), window grows to max(list, popover) only
    when needed; header, separators, icons, keycaps; own search field with
    focus handling; layout tokens (I-5, I-10)
  - branch `feat/action-panel-popover`

## Round 2 (user feedback after trying the build)

- [x] 4. fix: footer jumps (shrinks) on every Ctrl+K in the root list; views are fine
- [x] 5. fix: Ctrl+Enter on typed URLs and quicklinks opens a private window
- [x] 6. feat: popover configurable from the style: anchor (corner/edge/center)
  + X/Y offsets, width, max visible rows, within schema ranges (I-11)
- [x] 7. build and reinstall

## Decisions (made while the user was away; review on return)

- Shortcuts are not stored on `Action` (I-14): the panel displays the keys of
  the GPUI binding that performs the row, read from `keymap.rs`.
- `Action::group()` drives separators; `Action::glyph()` names a bundled SVG.
- Verbs name the target as Raycast does: Open Application, New Application
  Window, Open Command.
- New binding: `Ctrl+Enter` runs the second action.
- `view::Action` gains an optional `icon` (serde default, I-13 single place).
- Task 3 is split into two commits: popover layout, then its search field.
- No RDD review for tasks 2-3 (user waived it); build + reinstall at the end.

## Evidence

- Task 1: `e18740c` on `fix/action-panel-escape-height`. fmt, clippy, 108 lumen-ui tests pass.
  Review `review-9d443e1eddaaf008` approved (4 non-blocking notes: no behavioural
  resize test, CRLF boundary in the source-scan test, fn-attribution heuristic,
  unconditional browser_menu clear). Acknowledgement interrupted by a Pi crash;
  review status now fails with "unrelated target status is inconsistent".
  Not seen on screen yet.
- Task 2: `43278eb` (core groups/glyphs/verbs, view::Action.icon) and
  `a51ecf3` (keymap-derived shortcuts, Ctrl+Enter, ARCHITECTURE 7.44) on
  `feat/action-metadata`. fmt, clippy, workspace tests pass (core 887, ui 116).
  Deferred: assets/icons/README.md glyph count; providers not reordered by group.
- Task 3a: `91e812d` on `feat/action-panel-popover` (popover over the list,
  ARCHITECTURE 7.45). First attempt died at the session limit; resumed from its
  uncommitted layout/config work. fmt, clippy, workspace tests pass. Watch by eye:
  taffy layout of the absolute popover, filled keycaps on the selected row,
  shadow clipped at the window edge.
- Task 3b: `548d2ca` (own search field, ARCHITECTURE 7.46). fmt, clippy
  --all-targets, workspace tests pass (core 896, ui 120).
- Release build (5m09s) installed via scripts/install.ps1, daemon restarted.
  Pending: visual check by the user; branches unpushed, no PRs.
- Task 4: `db02304` (footer/search flex_none; popover wrapper shrinks instead).
  Cause from code: window grows a frame after the panel opens on a short root
  list, overflow landed on the footer. Not reproduced on screen.
- Task 5: `6ab4f74`. Argument quicklinks: Ctrl+Enter opens privately with an
  empty query (ArgumentRequest has no private flag).
- Task 6: `1e474f0` (style keys actions.anchor/width/offset-x/offset-y/max-rows,
  ARCHITECTURE 7.47). Worker left it uncommitted per role rules; parent committed.
- Task 7: release build 4m06s, installed, daemon running. Visual check pending.
- Round 3: `37a8edb` (body region is the only flexible part; views use the
  style's full list height; detail-pane lists in whole rows; regression from
  db02304 + pre-existing view-height bug from main) and `9787d1a` (Carbon popover
  keys). User's edited %APPDATA% carbon.json got the keys by hand (backup .bak),
  keeping their list.height 391. Installed, `lumen reload` run.
- Round 4: `78f6ed5` (loading/empty views at full height) and `2bf727b`
  (ARCHITECTURE 7.48, F15 token count 38/14). Installed.
  Follow-ups: delete unused `Metrics::empty_height` + `layout::view::EMPTY_HEIGHT`;
  short plain-list views still shrink after loading (by design, like root).
  Proposed next: popover open animation (fade + scale + overshoot) via a
  vendored-gpui element-scale patch; user asked for cost, not implementation.
