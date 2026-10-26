# Paste and Keep Window Open (F18 follow-up)

Branch: `main` (the user authorised working on main directly).
Reference: Raycast's *Paste and Keep Window Open*, `Ctrl+Shift+Enter`, in the
grid and in Ctrl+K, for pasting several emoji in a row.

## Known before starting

- Today `paste_into` (lumen-app main.rs ~:2599) hides the launcher, waits for
  the target to come to the front (`wait_for_foreground`, paste.rs), sends
  Ctrl+V, and puts the clipboard back on a thread of its own.
- The launcher window is `WS_EX_TOPMOST` (window.rs:46): it can stay visible
  while another window has the focus.
- Lumen already brings a window to the front with `AttachThreadInput` +
  `SetForegroundWindow` (activate.rs ~:370).

## Tasks

- [x] 1. (spike commit, examples/paste_round_trip.rs) Without hiding: give
      foreground 50/50, back 50/50, window never hidden. Plain
      SetForegroundWindow both ways: give 4.1/4.8 ms, landed 15.6/17.4,
      back 13.7/15.4 (p50/p95). AttachThreadInput slower, no gain. Lost
      pastes 0-5/50 from other clipboard openers (same with hiding): keep
      paste.rs read detection, never retry blindly; return focus after the
      target has read the clipboard. Foreground lock untested (off here).
      Spike: focus to the target with the launcher still visible, Ctrl+V
      delivered, focus back to the launcher. Reliability over many trials,
      flicker, timing. Decide the approach from the numbers.
- [x] 2+3. ("feat: Paste and Keep Window Open") `Platform::paste_keep_open`
      (trait method kept over a Paste field: different behaviour, Paste stays
      data; Linux todo!()), focus back after the read/timeout, a global paste
      mutex serialises pastes, `pastes_kept_open` guard against
      dismiss_on_focus_loss, `view::paste_action::KEEP_OPEN` on
      ctrl-shift-enter. 1,743 tests. NOT yet tried live.
- [ ] 2. Platform: a paste that returns focus, signalling when it is done
      (trait Platform, both implementations, I-9).
- [ ] 3. Core + UI: the action in the emoji view, `ctrl-shift-enter` from the
      keymap (I-14), the grid kept as it was, Frequently Used updated live.
- [x] 3b. (2388468) No modifier wait in keep-open: Shift/Alt/Win released in
      the Ctrl+V batch, Shift/Ctrl re-pressed only if physically held
      (WH_KEYBOARD_LL watch, atomics only), Alt/Win never re-pressed (dummy
      VK 0xE8 against menus); focus back at the first read, clipboard owner
      lingers for grace/restore; rapid presses batched into one paste in
      order (PasteBatch in Shell); plain Enter during a batch joins it and
      closes after. 1,755 tests. Expected ~40-60 ms per paste (not measured).
      Installed; PENDING user's hand test.
- [x] 4. (docs committed; user accepted "not perfect, good enough") (9635f36 fix: empty INPUT before each plain SetForegroundWindow, no
      AttachThreadInput; no copy-instead on refusal; Frequently Used frozen
      while open. Idle-machine runs p1/p2 passed nearly all; automated runs
      fail while the user is using the machine, so the user checks by hand.
      Installed. PENDING: user's manual check, then docs.)
      Docs (ARCHITECTURE: F18 decisions + a 7.x entry if the platform part
      deviates), live check, reinstall.

## Evidence

(commit ids per task)
