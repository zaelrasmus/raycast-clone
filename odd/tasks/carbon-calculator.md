# Carbon calculator and search rule

Branch: `feat/carbon-calculator` (off `feat/letter-spacing` at `a13a366`).
References: Raycast calculator `C:\Users\kiza2\Pictures\Screenshots\Screenshot 2026-10-01 111336.png`,
Raycast list `...111232.png`; Lumen Carbon calculator capture (user's, 2026-10-02 12:07).

## Decisions

- Everything is schema (I-11): new tokens and variants any style may set; base values keep
  today's look (I-12). Only Carbon opts in now.
- The search line is present but invisible in calculator view: 1 px `border.subtle` (#363636)
  with the card's #363636 fill 3 px below. Raycast: 2 physical px, header in between.
- The footer shows "Actions" only with more than one action: the answer gains "Copy
  Expression"; the primary becomes "Copy Answer".

## Tasks

- [x] 1. Schema + UI: token `rule.width` (search bottom, footer top); card tokens/variants:
      header above the card, card rows, value weight, dimmed operators, labels with a
      background, vertical divider with the arrow in its gap. Tests. Commit.
- [x] 2. Calculator actions: "Copy Answer" primary, "Copy Expression" second. Tests. Commit.
- [x] 3. Carbon values measured against Raycast's calculator capture; isolated captures. Commit.
- [x] 4. Docs: ARCHITECTURE schema table + decision, README if it lists the card. Commit.
- [x] 5. Release build and reinstall (ask the user first).

## Evidence
- Task 1: `77c347a`. RED compile-level (missing types/fns). fmt, clippy clean; tests 880 core + 107 ui, 0 failed. Not yet seen on screen. Chip fill uses `border.strong`; operators set `+ - − * × / ÷ ^ %`.
- Task 2: `2b91774`. New Action variants CopyAnswer/CopyExpression; money rows unchanged. Tests 882 core + 107 ui, 0 failed.
- Task 3: `f6d1c8b` divider hairline in border.strong (was border.subtle = card fill on Carbon);
  `8668932` token card.label-gap (RED compile, GREEN; 882 core + 107 ui); `0663133` Carbon values.
  Measured vs Raycast: digits 22.0 vs 22.3 px, value-chip gap 41.5 vs 42, rules 2 px #363636
  both. Card 143 vs 154 px (whole rows). Capture uses SendInput Unicode (SendKeys ate '+').
- Task 4: `see git log` ARCHITECTURE 7.43, schema rows, F15 row 34 tokens / 13 variants.
- Task 5: installed via scripts/install.ps1 (20.5 MB), daemon started 12:41; user's carbon/graphite/ink styles match the release.
