//! Icon loading and caching.
//!
//! Three rules, all of them consequences of the F1 risk note that icons are
//! where these projects look cheap:
//!
//! 1. **Never on the UI thread.** Extraction is COM, disk and decode work, all
//!    of it far outside the 8 ms per-frame budget.
//! 2. **Reserve the space first.** A row's icon square exists before the icon
//!    does, so an icon that arrives three frames late does not move the title
//!    next to it (I-10).
//! 3. **Cache by source, not by row.** The same executable appears under several
//!    names; extracting its icon once per name is work done for nothing.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, RenderImage};
use image::{Frame, RgbaImage};
use lumen_core::Kind;
use lumen_platform::{IconImage, IconSource, Platform};
use parking_lot::Mutex;

/// The bundled glyph for a row that has no icon of its own.
///
/// Keyed on [`Kind`] rather than carried on the [`Item`](lumen_core::Item),
/// because which glyph a kind gets is a presentation decision and `core` is
/// not where presentation decisions live (AD-03). It also means a provider
/// cannot forget to set one.
///
/// [`Kind::Application`] has an entry too, even though an application almost
/// always has a real icon: almost always is not always, and an empty square
/// beside fifteen filled ones is worse than a plain glyph.
pub fn glyph(kind: Kind) -> &'static str {
    match kind {
        Kind::Application => "icons/application.svg",
        Kind::Command => "icons/command.svg",
        Kind::Calculation => "icons/calculator.svg",
        Kind::Script => "icons/script.svg",
        Kind::Snippet => "icons/snippet.svg",
        Kind::Clipboard => "icons/clipboard.svg",
        Kind::Window => "icons/window.svg",
        Kind::File => "icons/file.svg",
        Kind::Quicklink | Kind::Website => "icons/link.svg",
        Kind::Timer => "icons/timer.svg",
        Kind::Currency => "icons/currency.svg",
        // The bolt again: an extension command is a command, and a glyph per
        // extension means reading an image the host has not seen, which is a
        // host-RPC decision rather than an indexing one.
        Kind::Extension => "icons/command.svg",
    }
}

/// What the UI knows about one icon.
#[derive(Clone)]
enum Slot {
    /// Extraction is in flight. Draw the placeholder, do not queue it again.
    Loading,
    Ready(Arc<RenderImage>),
    /// Extraction failed. Remembered so a broken icon is not retried on every
    /// single frame for the life of the process.
    Failed,
}

/// Icons, extracted off the UI thread and cached by source.
pub struct IconCache {
    platform: Arc<dyn Platform>,
    slots: Arc<Mutex<HashMap<IconSource, Slot>>>,
    size_px: u32,
}

impl IconCache {
    pub fn new(platform: Arc<dyn Platform>, size_px: u32) -> Self {
        Self {
            platform,
            slots: Arc::new(Mutex::new(HashMap::new())),
            size_px,
        }
    }

    /// The icon for `source`, queueing extraction if it has not been seen.
    ///
    /// Returns `None` while loading or after a failure; the caller draws the
    /// reserved placeholder either way, so both cases look the same on screen.
    pub fn get(&self, source: &IconSource, cx: &mut App) -> Option<Arc<RenderImage>> {
        {
            let slots = self.slots.lock();
            match slots.get(source) {
                Some(Slot::Ready(image)) => return Some(image.clone()),
                Some(Slot::Loading | Slot::Failed) => return None,
                None => {}
            }
        }

        self.spawn_load(source.clone(), cx);
        None
    }

    fn spawn_load(&self, source: IconSource, cx: &mut App) {
        self.slots.lock().insert(source.clone(), Slot::Loading);

        let platform = self.platform.clone();
        let slots = self.slots.clone();
        let size_px = self.size_px;

        cx.spawn(async move |cx| {
            let loaded = cx
                .background_executor()
                .spawn({
                    let source = source.clone();
                    async move { platform.load_icon(&source, size_px) }
                })
                .await;

            let slot = match loaded {
                Ok(image) => Slot::Ready(Arc::new(to_render_image(image))),
                Err(error) => {
                    log::debug!("no icon for {source:?}: {error:#}");
                    Slot::Failed
                }
            };
            slots.lock().insert(source, slot);

            // The list is already on screen with a placeholder in this row;
            // nothing repaints until something asks for a frame.
            cx.refresh().ok();
        })
        .detach();
    }
}

/// Convert platform pixels into what GPUI's atlas expects.
///
/// The platform boundary deals in straight RGBA because that is the format that
/// means the same thing on Windows and on Wayland. GPUI's atlas wants
/// premultiplied BGRA. The conversion belongs here, in the presentation layer,
/// rather than in `trait Platform`: encoding the renderer's memory layout into
/// the OS boundary would couple the two layers AD-03 exists to separate, and it
/// runs once per icon rather than once per frame.
fn to_render_image(icon: IconImage) -> RenderImage {
    let mut pixels = icon.rgba;

    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = pixel[3] as u32;
        let premultiply = |channel: u8| ((channel as u32 * alpha + 127) / 255) as u8;

        let (r, g, b) = (pixel[0], pixel[1], pixel[2]);
        pixel[0] = premultiply(b);
        pixel[1] = premultiply(g);
        pixel[2] = premultiply(r);
    }

    let buffer = RgbaImage::from_raw(icon.width, icon.height, pixels)
        .expect("the platform guarantees width * height * 4 bytes");

    RenderImage::new(vec![Frame::new(buffer)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swaps_to_bgra_and_premultiplies() {
        let icon = IconImage {
            width: 1,
            height: 1,
            // Straight RGBA: pure red at half alpha.
            rgba: vec![255, 0, 0, 128],
        };

        let image = to_render_image(icon);
        let bytes = image.as_bytes(0).unwrap();

        // BGRA order, with the red channel premultiplied down to ~128.
        assert_eq!(bytes[0], 0, "blue");
        assert_eq!(bytes[1], 0, "green");
        assert!((127..=129).contains(&bytes[2]), "red was {}", bytes[2]);
        assert_eq!(bytes[3], 128, "alpha");
    }

    #[test]
    fn fully_opaque_pixels_survive_unchanged_apart_from_the_swap() {
        let icon = IconImage {
            width: 1,
            height: 1,
            rgba: vec![10, 20, 30, 255],
        };

        let bytes = to_render_image(icon).as_bytes(0).unwrap().to_vec();

        assert_eq!(bytes, vec![30, 20, 10, 255]);
    }
}
