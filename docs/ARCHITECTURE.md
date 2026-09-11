# Architecture and execution plan

A native command palette in Rust + GPUI. Windows 11 now, Arch Linux + Hyprland later.

The authoritative source for this document is
[`launcher-rust-gpui-plan-EN.pdf`](launcher-rust-gpui-plan-EN.pdf) (v0.3, 26 August 2026).
What follows is the working version: the decisions, invariants, budgets and risks
in the form they are actually referenced from code, plus a record of where the
implementation deviates from the plan and why.

---

## 1. Vision and non-goals

**What it is.** A native desktop command palette written entirely in Rust, with
its interface built on GPUI. The declared priority is quality: imperceptible
latency, a near-zero idle footprint, and an aesthetic on par with Raycast.

**What it is not.**

- Not a commercial product. No market pressure, no launch date, nobody to beat.
- Not aiming for compatibility with the Raycast ecosystem, its store or its API (AD-10).
- Not attempting to support every Linux desktop. The target is Hyprland and
  compositors supporting `wlr-layer-shell`. GNOME is explicitly out of scope.
- Not a project for accumulating features. Every UI primitive added must be
  rendered well on two platforms, forever.

**Success criterion.** That it becomes the application the author opens fifty
times a day for a year without it ever once feeling slow, ugly or fragile.

### Why native, rather than Tauri

Open latency is not the differentiator — Tauri already reaches ~12 ms and there
is no perceptible gain left there. The differentiator is the **idle** column. A
launcher spends 99.9 % of its time hidden, and 435 MB with 3.2 % CPU permanently
is the price of the WebView. That, not open speed, is what justifies going native.

| Metric (July 2026 reference, M4 Max) | Asyar (Tauri) | Raycast | Target here |
| ------------------------------------ | ------------- | ------- | ----------- |
| Hotkey → window visible (p50)        | 12.0 ms       | 21.7 ms | < 40 ms     |
| Cold start to usable                 | 572 ms        | 888 ms  | < 400 ms    |
| Idle RAM                             | 435.6 MB      | 272.6 MB| < 150 MB    |
| Idle CPU (30 s)                      | 3.20 %        | 0.04 %  | < 0.2 %     |
| On-disk size                         | 64 MB         | 209 MB  | < 40 MB     |

---

## 2. Licensing

Lumen is **MIT OR Apache-2.0**.

This was decided before the first commit (AD-11), because deciding later means
relicensing, and because the choice governs which third-party code may be read
with intent to reuse.

Consequences that bind day to day:

- **Vicinae** (C++23/Qt, GPL-3.0) and **Asyar** (Tauri, GPL-3.0 core) may be
  *read* to understand architecture, protocol shape and manifest structure —
  the highest-value activity available — but their code must never be copied
  into this repository. That would force Lumen to GPL-3.0.
- Asyar's SDK and extension tooling are MIT and are reusable with attribution.
- GPUI itself is Apache-2.0 and published on crates.io, so the UI layer
  introduces no copyleft obligation.

---

## 3. Architecture decisions

| # | Decision | Reason |
| - | -------- | ------ |
| AD-01 | Rust throughout, GPUI as the interface layer | GPUI renders on the GPU and draws all its own controls, guaranteeing visual parity across platforms. Accepted cost: pre-1.0, breaking changes between versions, sparse documentation. The practical reference is Zed's source. |
| AD-02 | Resident daemon plus a client that toggles visibility over IPC | Solves startup (the window is created once and merely hidden) and Wayland (no global shortcut API exists, so the compositor runs the binary and a second execution toggles the instance). |
| AD-03 | Four layers: `core`, `platform`, `ui`, `app` | `core` holds index, matching, providers and actions with no knowledge of the OS or of GPUI. `ui` depends on GPUI. `platform` defines the system boundary. If GPUI proves unworkable, the presentation layer is replaced without losing the project. |
| AD-04 | A single `trait Platform` as the OS boundary | A deliberately small surface. Any need to cross it with something else signals coupling and must be fixed, not accommodated by widening the trait. |
| AD-05 | The theme is a data file, not code, and reloads live | Aesthetics are iterated hundreds of times. Recompiling per iteration kills the design process. The highest-return afternoon of work in the project. |
| AD-06 | The type contract is defined once and is transport-independent | UI tree and event types are Rust structs with `serde`; WIT and JSON Schema are *derived*. The contract is the durable artefact, the transport is reversible. |
| AD-07 | A closed, small UI vocabulary | Extensions never draw pixels; they emit a declarative tree over fixed primitives. Consistency by construction, not by third-party discipline. |
| AD-08 | Static manifest indexed at install time | With zero extension processes running, commands appear because they are in the local index. |
| AD-09 | Every extension capability goes through the host RPC | No real sandbox at first, but the boundary is drawn, so adding one later changes the runtime rather than redesigning the system. |
| AD-10 | No compatibility with Raycast's API or store | Avoids reimplementing their reconciler and inheriting trademark/ToS risk. Their API *shape* is adopted so porting is mechanical. |
| AD-11 | Licence chosen before the first commit | See section 2. |
| AD-12 | Themes are pure data, no code, no runtime | Zed's model. Semantic tokens only; a token that names a component turns every new component into a break for every existing theme. |
| AD-13 | Themes control colour and typography, never geometry | If a third-party theme can alter geometry, the calibrated layout dies the first day someone publishes one. This is the decision that protects what motivates the project. |
| AD-14 | Extensibility in four tiers, not one mechanism | T0 themes, T1 script commands, T2 WebAssembly view extensions, T3 optional JS runtime. Each has a radically different effort-to-value ratio. |
| AD-15 | Keyboard handling at two levels with a sharp boundary | Outside the window is bespoke platform code behind `trait Platform`; inside is GPUI's action and keymap system. GPUI resolves keystrokes against the focus tree, which requires the window to already hold focus — so it cannot cover global invocation. |
| AD-16 | The press-and-hold gesture is not a shortcut | Accelerator APIs require modifiers plus a non-modifier key and report only the press. A lone held modifier with measured duration and a release event is a gesture over keyboard state, not an accelerator. |

---

## 4. Invariants

Reproduced verbatim in [`../CLAUDE.md`](../CLAUDE.md), which is the copy that
gets read during development.

---

## 5. Performance budgets

Reproduced in [`../CLAUDE.md`](../CLAUDE.md). Current measurement status is in
the README.

---

## 6. Phases

| Phase | Goal | Status |
| ----- | ---- | ------ |
| **F0** | Platform spike on Windows: transparent undecorated window, backdrop, monitor placement, global gesture, focus capture and restore | **Done** |
| **F1** | The minimum usable launcher: daemon + client, application index, icons, fuzzy matching with frecency, benchmark harness | **Mostly done** — see README for what is outstanding |
| **F2** | Design system and aesthetics: token file with live reload, theme tier T0, bundled typeface, icon set, action panel, appearance animation | **Mostly done** — themes, live reload, the fade, the bundled typeface and the action panel are in. The icon set is not, and the appearance animation omits the scale (7.5) |
| **F3** | Internal providers: calculator, clipboard history, snippets, window management, file search. Tier T1 script commands. Context-aware shortcut overlay | **Overlay, settings editor, calculator, snippets, clipboard history, window management and tier T1 done**, behind the `trait Provider` seam. File search not started |
| **F4** | View extensions, tiers T2 and T3 | Not started |
| **F5** | Port to Linux and Wayland | Stubbed and cross-checked in CI (I-2) |

### F4 build order is not optional

1. Hand-write the JSON of a `List` tree with ten items and an action panel.
   Render it in GPUI from a static file. No protocol, no processes.
2. Wrap that behind `render_node(node) -> AnyElement`, with list virtualisation.
3. Only then move the tree across a channel and introduce events.
4. Write the **second** extension before stabilising anything in the protocol.
   The first extension lies: it fits perfectly because the protocol was designed
   while looking at it.

---

## 7. Deviations from the plan

Every one of these is a place the implementation knowingly differs from the PDF.
They are listed so the difference is a decision on the record rather than a
discrepancy someone finds later.

### 7.1 Invocation uses `RegisterHotKey`, not Raw Input

*Plan (section 7):* Raw Input as the single event source, deriving both
invocation and the hold gesture from it.

*Here:* `RegisterHotKey` for invocation, Raw Input for the hold gesture, both on
one message-only window with one message loop.

*Why:* Raw Input cannot suppress a keystroke. A launcher bound to `Alt+Space`
that does not suppress leaves the keystroke to reach the focused application,
where `Alt+Space` opens the system menu — the launcher would appear with a
window menu open behind it. What is load-bearing in AD-16 is narrower than "one
mechanism": a lone held modifier does not fit the accelerator model, but
invocation fits it perfectly. Splitting by *concern* rather than by mechanism
keeps the property section 7 was protecting (not two coexisting event loops) and
still never uses `WH_KEYBOARD_LL`.

### 7.2 The application index reads `shell:AppsFolder`

*Plan (section 7):* Start Menu shortcuts on disk, plus packaged applications via
WinRT's `PackageManager`.

*Here:* one enumeration of the `shell:AppsFolder` virtual folder.

*Why:* two sources means two notions of identity, two icon paths, and a
deduplication problem when a packaged application also ships a shortcut. The
shell already merges them, and the AppsFolder is exactly the list behind "All
apps" — which is the operative definition of "100 % of installed applications"
in the F1 acceptance criteria. Each item yields a display name plus a parsing
name that serves as both launch target and icon source.

### 7.3 `DwmExtendFrameIntoClientArea` is required before the backdrop

Not mentioned in the plan, and discovered during F0: setting
`DWMWA_SYSTEMBACKDROP_TYPE` alone is *accepted and silently ignored*. DWM only
composites a system backdrop behind frame it owns, so the frame must first be
extended across the whole client area. Without it the window is merely
see-through — desktop content shows through sharply rather than being composited.

This is precisely the shape of failure R-1 anticipated, which is why all three
rungs of the backdrop ladder (`system-acrylic`, `legacy-acrylic`, `solid`) are
implemented and selectable from configuration rather than chosen once at
compile time.

### 7.4 GPUI's deferred placement must not be triggered

`Window::activate_window()` causes GPUI to apply a placement it stored for a
window created hidden, and that placement lands *after* our `SetWindowPos`,
dragging the launcher to the rectangle it was constructed with. Taking the
foreground is therefore left entirely to the platform layer
(`AttachThreadInput` + `SetForegroundWindow`), and the UI only calls
`window.focus(&handle)` to point GPUI's focus tree at the view.

### 7.5 The appearance animation omits the scale

F2 asks for "fade plus a 0.98-to-1.0 scale over roughly 120 ms". The fade is
implemented; the scale is not, because GPUI 0.2.2 has no transform for `div`
elements, and faking it by animating width or height is exactly what the
overlay animation rules forbid (it forces per-frame layout recalculation).

The fade is driven by GPUI's animation element rather than by measuring elapsed
time inside `render`: a repaint requested from within a render pass is not
guaranteed to arrive, and when it does not, the window is stranded at whatever
opacity the first frame computed.

### 7.6 Geometry tokens live in code, not in the theme file

I-5 says no colour, spacing, radius or type size may appear literally in UI code.
I-11 says a theme may define only colour and type family. Both hold because there
are *two* token sources with different authors: colour and type family come from
the theme file, which anyone may write; geometry comes from `lumen_core::layout`,
which only the project may change. What I-5 forbids — a literal `px(38.)` buried
in a render function — stays forbidden either way.

### 7.7 The window methods take a role rather than gaining overlay twins

F3 says the overlay "is a new platform surface, not a UI feature… It widens
`trait Platform`." It does — but as a `WindowRole` parameter on the four window
methods rather than as `show_overlay` / `hide_overlay` / `attach_overlay` /
`set_overlay_bounds`.

The method count stays where AD-04 wants it, the differences between the two
surfaces stay expressed as data rather than duplicated code, and "two warm
windows, not one" becomes something the boundary states outright instead of a
convention two call sites happen to follow.

### 7.8 Lumen's own sheet is generated from the keymap table, not from the registry

F3 asks that Lumen's entries "are not written by hand: they are generated by
enumerating GPUI's global action registry". The intent — the sheet cannot drift
from the bindings that are installed — is honoured, but the source is the
binding table in `lumen-ui/src/keymap.rs`, which feeds both `cx.bind_keys` and
the generated sheet.

The registry alone is not sufficient: it can report that `lumen::SelectNext` is
bound to `down`, but it has no human-readable description of what the action
does, and deriving one from the type name puts "SelectNext" on screen where the
user needs "Move down". The registry is still consulted, by
`verify_against_registry`, to report any registered action the table has
forgotten.

### 7.9 Providers distinguish a catalogue from an answer

Not in the plan, and found by running it. F3 says only "each one behind a
`trait Provider` in `core`", which leaves open how a provider's results reach
the list.

The first attempt ranked every provider result through the fuzzy matcher, like
applications. That silently discards every calculation: matching compares the
query `12*7` against the result's title `84`, which do not resemble each other
at all, so the row scored nothing and never appeared.

So a provider contributes in one of two ways. A **catalogue** exists independently
of the query -- Lumen's commands, later the snippets of F3 -- joins the
application index and is matched exactly like an application. An **answer** is
computed from the query, already *is* what the user asked for, and is shown
without matching, above the fuzzy guesses.

### 7.10 The settings window is created lazily

I-4 says the window is created once at startup and thereafter only shown and
hidden. That invariant exists to keep *invocation* instant, and it is honoured
for the launcher and the overlay, which are both summoned by a gesture.

Settings is opened deliberately and rarely. Paying idle memory for a third warm
window to save a few hundred milliseconds twice a month is the wrong side of the
budget in section 5, so it is created on first open -- and from then on kept and
only hidden, exactly like the others.

### 7.11 The action panel replaces the list rather than floating over it

F2 asks that every action be reachable from the keyboard; it does not say the
panel has to be an overlay. The first attempt made it one, and it does not
survive contact with the layout.

The launcher window is sized exactly to its content -- search field, rows,
footer -- so there is no spare room for a floating element. And an absolutely
positioned element with only `right` and `bottom` set gives taffy no way to
derive a height: it lays out at zero and is never painted. The element rendered
correctly and was simply invisible, which took a log inside the render function
to establish.

Replacing the list reuses the one region whose height is already known to be
right, and it reads better as an interaction anyway: the panel is a mode,
Escape leaves it, and nothing is hidden behind it.

### 7.12 A hotkey conflict does not stop the daemon

Global hotkeys are first come, first served on Windows. If the configured
combination is taken, Lumen logs which key failed and where to change it, then
keeps running and stays reachable over the socket. Refusing to start would turn
"pick a different key" into "the application is broken", with the explanation
visible only to someone who thought to run it from a terminal.

### 7.13 The clipboard privacy layer skips rather than redacts

The usual shape of this feature is to store the entry and mask it in the UI.
That is the wrong place to draw the line, because the secret is then on disk in
an unencrypted SQLite file and the protection is a rendering decision -- one
bug, one `SELECT`, or one backup away from being no protection at all.

So `lumen_core::clipboard::privacy` decides *before* the write, and a rejected
entry produces no row. There are two halves and only one of them can live in
`core`: the platform half honours the three Windows clipboard formats that mean
not-for-history (`Clipboard Viewer Ignore`,
`ExcludeClipboardContentFromMonitorProcessing`, and a zero in
`CanIncludeInClipboardHistory`), because only the platform can see them; the
`core` half declines anything copied out of a known password manager and
anything matching a credential format -- PEM blocks, AWS and GitHub and Slack
and OpenAI and Anthropic and Google and Stripe keys, JWTs, and URLs with
embedded credentials.

The rejection is logged at `debug` and without the text. A privacy layer that
writes the secret to the log has moved the file, not solved the problem.

The cost is that a false positive loses an entry silently. That is the right
side of the trade, and it is why the false-positive cases are tests rather than
an afterthought.

### 7.14 Clipboard history is an index of its own, not a `Provider`

Everything else in F3 fits the provider contract: a catalogue that joins the
application index and is matched with it. The clipboard does not, for two
reasons that are both about ranking rather than about results.

It changes constantly. Folding it into `candidates` would mean re-preparing
several hundred application haystacks every time the user copies anything,
which is exactly the work the per-frame budget exists to keep out of the way.

And it must not appear in the root list. With an empty query the index is shown
in frecency order, so a hundred catalogue entries with no usage history would
interleave alphabetically with the applications: open the launcher, see what
you copied last Tuesday. The history is something you search, not something you
are shown before you have asked for anything.

So the view holds it as a second index, ranked separately, only when there is a
query, and capped so that a common letter cannot fill the list with pastes. The
provider module survives as the thing that says what a clipboard row looks
like, which is all it was ever really doing.

Refreshing it is the daemon's job and happens when an entry is recorded -- the
only moment anything knows the history changed -- rather than on a timer or
when the window opens, so the snapshot is already correct by the time anyone
searches it and no query runs that nobody asked for.

### 7.15 A layout is applied to the captured handle, never to "the foreground"

Activating a row hides the launcher and hands the foreground back before the
action runs. Any window-management feature that then asks the system which
window is in front is reading the answer to a question it asked too late: the
hand-back is asynchronous from the point of view of everything watching it, and
losing that race means resizing a window the user did not choose.

So `Action::ArrangeWindow` travels back to the daemon like a builtin, and the
daemon applies it to the `FocusOwner` it captured *before* showing the
launcher -- the same handle it is about to restore focus to. It reads that
handle first and hides second, because hiding consumes it.

### 7.16 The tiling arithmetic lives on the boundary, not in `core`

Everything else about what a result *does* lives in `lumen-core`, so the layout
maths looks like it belongs there too. It cannot: `core` depends on
`lumen-platform` and not the other way round, and a layout is a value the
platform has to be handed.

That turns out to be the better place anyway. A half is a half on Windows and
on Hyprland, the arithmetic is the part that is easy to get subtly wrong, and
putting it in `lumen_platform::tiling` means it is tested with no compositor
anywhere near it -- including the cases that actually bite: a work area whose
width divides by neither two nor three, whether the thirds meet without a seam,
and whether a window carried between monitors of different sizes arrives inside
the destination. `core` keeps what is genuinely its own, which is what each
layout is called.

### 7.17 Tiling compensates for the invisible resize border

Since Windows 10 a window's resize border is drawn outside what the user
perceives as its edge. `GetWindowRect` reports the outer rectangle and DWM
composites the inner one, so placing a window at exactly half the work area
leaves a visible strip of desktop down the middle and lets the window overhang
the screen by the same amount -- around seven physical pixels at 100%, more
when scaled.

`DWMWA_EXTENDED_FRAME_BOUNDS` reports what is actually drawn. The difference
between the two rectangles is the padding a target is expanded by. It is four
lines, and it is the whole difference between a tiling feature that looks
deliberate and one that looks approximate.

A window DWM will not describe is placed as asked rather than not placed at
all.

### 7.18 No window is touched from inside a GPUI callback

A subscription callback runs while GPUI holds a mutable borrow of the `App`.
Showing, hiding, moving or resizing a window sends window messages
*synchronously* to our own window procedure, on the same thread; GPUI's
procedure then tries to take the borrow it cannot have, and the message is
dropped.

Nothing reports this. The only sign is a line in the log reading
`RefCell already borrowed`, with no target and no indication of what it was
about, produced by an internal `log_err` on a `try_borrow_mut` several frames
away from the cause. It had been happening on every show and every dismiss
since F1 and was found by reading the log during unrelated work, which is
exactly how long a silent failure survives.

What gets dropped is size and position notification for a window that is being
resized at that moment -- the launcher hugs its content, so it is resized on
almost every keystroke.

So every subscription handler that touches a window now goes through
`off_the_callback`, which spawns the work onto the foreground executor: same
thread, same order, no borrow held. The three handlers are the launcher's
(dismiss, resize, arrange), the overlay's fade-out, and the settings window's
close.

---

## 8. Risk register

| # | Risk | Phase | Status |
| - | ---- | ----- | ------ |
| R-1 | Real transparency unworkable; the system backdrop does not show | F0 | **Hit and mitigated.** See 7.3. Three backdrop modes are implemented and configurable. |
| R-2 | Breaking changes in GPUI between versions | all | `gpui` pinned exactly (I-6). F0 assertions re-apply the window styles rather than assuming GPUI's. |
| R-3 | Insufficient GPUI documentation | all | Mitigated by reading the crate source directly; it ships its `examples/`. |
| R-4 | Windows assumptions leaking into the core | F1–F4 | I-1 and I-2, cross-checked in CI from the first commit. |
| R-5 | GPUI does not implement `wlr-layer-shell` | F5 | Accepted as a planned task with its own estimate. |
| R-6 | The extension protocol becomes coupled to its first runtime | F4 | AD-06, AD-09. |
| R-7 | Uncontrolled growth of the UI primitive vocabulary | F4 | Every new primitive requires explicit justification. |
| R-8 | Licence contamination from inadvertent reuse of GPL code | F0 | Decided (section 2). Study architecture, do not copy implementations. |
| R-9 | Silent performance degradation over months | F1+ | Partially mitigated: ranking is benchmarked in CI, but thresholds do not yet fail the build. |
| R-10 | Loss of motivation from deferring the aesthetic work | F2 | Live token reload landed early, in F1 rather than F2. |
| R-11 | Community themes break as the interface evolves | F2+ | Semantic tokens, default resolution from the base theme, versioned schema. Validated with two in-house themes. |
| R-12 | WebAssembly compilation friction | F4 | Consciously accepted. |
| R-13 | The global keyboard hook is unhooked by timeout or flagged by antivirus | F3 | **Avoided entirely.** The overlay gesture uses Raw Input, not `WH_KEYBOARD_LL`, and the default hold key is right control rather than Super precisely so no suppression -- and therefore no hook -- is ever required. |
| R-14 | Foreground detection returns the container, not the real application | F3 | **Half solved, half declared out of scope.** Packaged applications are resolved by descending past `ApplicationFrameHost` to the first foreign visible child window. Browsers and Electron applications still report their container; the title rung of the sheet fallback chain exists for them. |

---

## 9. The warning that will weigh most

> The characteristic risk of assisted development on a project like this is not
> incorrect code: it is **volume**. It is very easy to accumulate functionality
> faster than it can be polished, and this project is defined precisely by
> polish. The invariants and the budgets exist to act as a brake. When they
> conflict with moving faster, they win.
