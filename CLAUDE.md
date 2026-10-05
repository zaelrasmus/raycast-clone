# Lumen

A native command palette in Rust + GPUI. Windows 11 now, Arch + Hyprland later.

Full context: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Invariants

Not negotiated case by case.

| #    | Invariant |
| ---- | --------- |
| I-1  | `lumen-core` contains no `#[cfg(windows)]` or `#[cfg(unix)]` and no direct OS call. Everything goes through `trait Platform`. |
| I-2  | `lumen-platform-linux` always compiles, even if its methods are `todo!()`. **Paused:** the Linux cross-check is not run, locally or in CI, until F5 resumes (ARCHITECTURE 7.38). |
| I-3  | Root search never waits on an extension process. No exceptions and no "short" timeouts. |
| I-4  | The window is created once when the daemon starts and from then on is only shown or hidden. Never destroyed. |
| I-5  | No colour, spacing, radius or type size appears literally in UI code. Everything comes from a token source: colour and type family from the palette, the launcher's geometry from the active style, the rest from `lumen_core::layout`. One exception: `Icon::Swatch` draws a copied colour, which is content, not styling (F12). |
| I-6  | The `gpui` version is pinned exactly in `Cargo.toml`. Never `*`, never a range. Updating is a deliberate commit with its own verification. |
| I-7  | Every capability an extension needs is requested over host RPC. The subprocess does not reach resources directly even where the system would allow it. |
| I-8  | The performance budgets below are measured in CI. A regression breaks the build. |
| I-9  | Changing the signature of `trait Platform` requires updating both implementations in the same commit. |
| I-10 | Fixed row height and reserved space for icons and accessories. Zero layout shift when results change. |
| I-11 | A palette may define only colour and type family. A style may set only the geometry tokens and variants in its schema, each within its range; no structure or layout is themeable (F15). |
| I-12 | Every missing theme token resolves from the base theme. A theme written today must still load a year from now. |
| I-13 | The UI tree and event types exist in exactly one place. No parallel definitions in WIT, in JSON Schema or in the SDK. |
| I-14 | In-app shortcuts are declared as GPUI actions and bound from a keymap file. No hand-written key comparisons in UI code. One exception: the hotkey recorder (ARCHITECTURE 7.56). |
| I-15 | No system input callback allocates, logs, takes a lock or touches UI state. It pushes the event to a channel and returns. |

## Performance budgets

Numbers, not adjectives. Measured, not asserted.

| Budget | Threshold | How it is measured |
| ------ | --------- | ------------------ |
| Global gesture → first frame | p50 < 20 ms, p95 < 40 ms | Synthetic summons until the first frame. 15 on an idle machine: `scripts/measure-gesture.ps1`. |
| Global command hotkey → action done | p50 < 20 ms, p95 < 40 ms | WM_HOTKEY until the command has run, 15 presses: `scripts/measure-hotkey.ps1`. |
| Per-frame cost while typing | p95 < 8 ms | Internal instrumentation of the render loop with the full list loaded. |
| Idle resident memory | < 100 MB | Working set, window hidden, no extensions active: `scripts/measure-idle.ps1`. |
| Idle CPU | < 0.02 % | Share of the machine over 60 s with the window hidden: `scripts/measure-idle.ps1`. |
| Cold start to usable | < 400 ms | Daemon launch until the first search responds. |
| Installed size | < 40 MB | Binary plus bundled assets. |

## Working rules

- One phase, one branch. One acceptance criterion, one pull request.
- For anything performance-related, write the measurement first and the
  functionality second. A benchmark written beforehand is a constraint; written
  afterwards it is a justification.
- `cargo fmt --check` and `cargo clippy -- -D warnings` pass before a commit.
  Do not run the Linux cross-check: it is paused (ARCHITECTURE 7.38).
- The characteristic risk here is **volume**, not incorrect code. It is very easy
  to accumulate functionality faster than it can be polished, and this project is
  defined by polish. When the invariants and budgets conflict with moving faster,
  they win.
