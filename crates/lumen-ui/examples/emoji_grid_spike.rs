//! F18, task 3: can the vendored GPUI draw a grid of colour emoji, and what
//! does a frame of it cost? Measured before any grid code exists.
//!
//! A THROWAWAY SPIKE. The colours, sizes and spacing below are literals, which
//! I-5 forbids in UI code; they are allowed here only because this example is
//! not product code, is never imported by it, and goes when the grid lands.
//! The geometry approximates the Raycast screenshot the view follows.
//!
//! Emoji are drawn as text with the system colour font, Segoe UI Emoji, the
//! decision F18 took for v1. Symbols the font lacks fall to DirectWrite's
//! system fallback, as they will in the view.
//!
//! ```text
//! # Draw the first screen and stay open; --offset starts at another row.
//! cargo run --release -p lumen-ui --example emoji_grid_spike -- --still
//! cargo run --release -p lumen-ui --example emoji_grid_spike -- --still --offset 210
//!
//! # Advance one row on every frame for N frames, then quit. The scroll walks
//! # the emoji groups (1,637 entries, 205 rows) and wraps, printing
//! # `spike: pass K` to stderr as each pass starts, so with N = 402 the second
//! # pass draws glyphs already in the atlas. ZED_MEASUREMENTS=1 makes gpui
//! # print `frame duration: ...` for each frame.
//! ZED_MEASUREMENTS=1 cargo run --release -p lumen-ui --example emoji_grid_spike -- --scroll 402
//! ```
//!
//! The scroll waits `--wait-ms` (default 2000) after the first frame before it
//! starts and the same again after it ends before quitting, printing
//! `spike: first frame`, `spike: scroll done` to stderr, so the working set can
//! be sampled from outside at both points.
//!
//! ```text
//! # F18, task 4a: --evict adds a second half to the scroll. After it ends the
//! # window drops its emoji glyphs (`Window::drop_emoji_glyphs`, ARCHITECTURE
//! # 7.53), draws the first screen again, saves it to
//! # target\emoji-spike-evicted.png, then scrolls N frames a second time.
//! cargo run --release -p lumen-ui --example emoji_grid_spike -- --scroll 201 --evict
//! ```
//!
//! With --evict the spike prints its own memory to stderr at four points: (a)
//! after the first frame, (b) after the scroll, (c) `--wait-ms` after the
//! eviction and (d) after the second scroll. Each line gives the working set,
//! the private bytes and the process's GPU memory (the `GPU Process Memory`
//! counters), where the atlas textures are counted.

use std::process::Command;
use std::time::Duration;

use gpui::{
    App, Application, Bounds, Context, FontWeight, Hsla, Render, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};
use lumen_core::emoji;

const COLUMNS: usize = 8;
const ROWS: usize = 5;
const CELL: f32 = 102.0;
const GAP: f32 = 8.0;
const GLYPH: f32 = 56.0;
const PADDING: f32 = 16.0;
const SELECTED: usize = 9;

#[derive(Clone, Copy)]
enum Mode {
    Still,
    Scroll { frames: usize },
}

struct Spike {
    glyphs: Vec<&'static str>,
    /// Rows the scroll walks before it wraps: the emoji groups only.
    emoji_rows: usize,
    row: usize,
    mode: Mode,
    wait: Duration,
    /// Frames advanced so far; `None` until the scroll has started.
    advanced: Option<usize>,
    started_wait: bool,
    /// Drop the emoji glyphs after the scroll and scroll again (`--evict`).
    evict: bool,
    /// Whether the eviction has happened, so the second scroll ends the run.
    evicted: bool,
}

/// Runs a PowerShell script and returns its trimmed output.
fn powershell(script: &str) -> String {
    Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_default()
}

/// This process's working set, private bytes and GPU memory (dedicated,
/// shared), in MB.
fn memory_mb() -> [f64; 4] {
    let script = format!(
        r#"$id = {pid}
$p = Get-Process -Id $id
$memory = "\GPU Process Memory(pid_$($id)_*)"
$c = (Get-Counter "$memory\Dedicated Usage", "$memory\Shared Usage").CounterSamples
$d = ($c | Where-Object Path -like '*dedicated*' | Measure-Object CookedValue -Sum).Sum
$s = ($c | Where-Object Path -like '*shared*' | Measure-Object CookedValue -Sum).Sum
"$($p.WorkingSet64) $($p.PrivateMemorySize64) $d $s""#,
        pid = std::process::id(),
    );
    let output = powershell(&script);
    let mut values = output
        .split_whitespace()
        .map(|value| value.parse::<f64>().unwrap_or(0.0) / (1024.0 * 1024.0));
    std::array::from_fn(|_| values.next().unwrap_or(0.0))
}

/// Saves this process's main window, as it is on screen, to `path`.
fn screenshot(path: &str) {
    let script = format!(
        r#"Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class SpikeWin {{
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  public struct RECT {{ public int L, T, R, B; }}
}}
"@
[SpikeWin]::SetProcessDPIAware() | Out-Null
$r = New-Object SpikeWin+RECT
[SpikeWin]::GetWindowRect((Get-Process -Id {pid}).MainWindowHandle, [ref]$r) | Out-Null
$bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
$bmp.Save('{path}', [System.Drawing.Imaging.ImageFormat]::Png)"#,
        pid = std::process::id(),
    );
    powershell(&script);
}

/// Prints the working set under `label`, sampled off the UI thread.
async fn report(label: &str, cx: &mut gpui::AsyncApp) {
    let [working_set, private, dedicated, shared] =
        cx.background_executor().spawn(async { memory_mb() }).await;
    eprintln!(
        "spike: working set {label}: {working_set:.1} MB (private {private:.1}, GPU dedicated {dedicated:.1}, GPU shared {shared:.1})"
    );
}

impl Spike {
    fn begin_scroll(&mut self, cx: &mut Context<Self>) {
        let wait = self.wait;
        let evict = self.evict;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            if evict {
                report("(a) first frame", cx).await;
            }
            this.update(cx, |spike, cx| {
                eprintln!("spike: pass 1");
                spike.advanced = Some(0);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn finish(&self, window: &Window, cx: &mut Context<Self>) {
        eprintln!("spike: scroll done");
        let wait = self.wait;
        if self.evict && !self.evicted {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(wait).await;
                report("(b) after the scroll", cx).await;
                this.update_in(cx, |spike, window, cx| {
                    eprintln!("spike: evict");
                    window.drop_emoji_glyphs();
                    spike.evicted = true;
                    spike.row = 0;
                    spike.advanced = None;
                    cx.notify();
                })
                .ok();
                cx.background_executor().timer(wait).await;
                report("(c) after the eviction", cx).await;
                let path = std::env::current_dir()
                    .unwrap_or_default()
                    .join("target")
                    .join("emoji-spike-evicted.png");
                let shot = path.display().to_string();
                cx.background_executor()
                    .spawn(async move { screenshot(&shot) })
                    .await;
                eprintln!("spike: screenshot {}", path.display());
                this.update(cx, |spike, cx| {
                    eprintln!("spike: pass 1 again");
                    spike.advanced = Some(0);
                    cx.notify();
                })
                .ok();
            })
            .detach();
            return;
        }
        let evict = self.evict;
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(wait).await;
            if evict {
                report("(d) after the second scroll", cx).await;
            }
            cx.update(|cx| cx.quit()).ok();
        })
        .detach();
    }
}

impl Render for Spike {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Mode::Scroll { frames } = self.mode {
            match self.advanced {
                None if !self.started_wait => {
                    self.started_wait = true;
                    eprintln!("spike: first frame");
                    self.begin_scroll(cx);
                }
                None => {}
                Some(done) if done < frames => {
                    if done > 0 {
                        self.row += 1;
                        if self.row + ROWS > self.emoji_rows {
                            self.row = 0;
                            eprintln!("spike: pass {}", done / (self.emoji_rows - ROWS + 1) + 1);
                        }
                    }
                    self.advanced = Some(done + 1);
                    window.request_animation_frame();
                }
                Some(done) => {
                    self.advanced = Some(done + 1);
                    if done == frames {
                        self.finish(window, cx);
                    }
                }
            }
        }

        let first = self.row * COLUMNS;
        let rows = (0..ROWS).map(|row| {
            div()
                .flex()
                .flex_row()
                .gap(px(GAP))
                .children((0..COLUMNS).map(|column| {
                    let index = row * COLUMNS + column;
                    let glyph = self.glyphs.get(first + index).copied().unwrap_or("");
                    div()
                        .size(px(CELL))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(10.0))
                        .bg(if index == SELECTED {
                            rgb(0x3a3a40)
                        } else {
                            rgb(0x26262b)
                        })
                        .font_family("Segoe UI Emoji")
                        .text_size(px(GLYPH))
                        .line_height(px(GLYPH * 1.25))
                        .child(glyph)
                }))
        });

        div()
            .size_full()
            .p(px(PADDING))
            .flex()
            .flex_col()
            .gap(px(GAP))
            .bg(rgb(0x18181b))
            .font_family(lumen_ui::FONT_FAMILY)
            .font_weight(FontWeight::NORMAL)
            .text_color(Hsla::white())
            .children(rows)
    }
}

fn main() {
    let mut mode = Mode::Still;
    let mut offset = 0;
    let mut wait = Duration::from_millis(2000);
    let mut evict = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut number = || -> usize {
            args.next()
                .and_then(|value| value.parse().ok())
                .unwrap_or_else(|| panic!("{arg} takes a number"))
        };
        match arg.as_str() {
            "--still" => mode = Mode::Still,
            "--scroll" => mode = Mode::Scroll { frames: number() },
            "--offset" => offset = number(),
            "--wait-ms" => wait = Duration::from_millis(number() as u64),
            "--evict" => evict = true,
            other => panic!("unknown argument {other}"),
        }
    }

    let entries = emoji::catalogue().entries();
    let glyphs: Vec<&'static str> = entries.iter().map(|entry| &*entry.chars).collect();
    let emoji_count = entries
        .iter()
        .filter(|entry| entry.category.is_emoji())
        .count();
    let emoji_rows = emoji_count.div_ceil(COLUMNS);

    Application::new()
        .with_assets(lumen_ui::Assets)
        .run(move |cx: &mut App| {
            lumen_ui::fonts::register(cx);
            let width = PADDING * 2.0 + COLUMNS as f32 * CELL + (COLUMNS - 1) as f32 * GAP;
            let height = PADDING * 2.0 + ROWS as f32 * CELL + (ROWS - 1) as f32 * GAP;
            let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: None,
                    is_resizable: false,
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|_| Spike {
                        glyphs,
                        emoji_rows,
                        row: offset,
                        mode,
                        wait,
                        advanced: None,
                        started_wait: false,
                        evict,
                        evicted: false,
                    })
                },
            )
            .expect("open the spike window");
            cx.activate(true);
        });
}
