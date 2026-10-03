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
//!
//! A file's icon in *Search Files* (F16) goes further: kept by type where
//! every file of the type has the same one, by path where it may be its own,
//! and the by-path ones forgotten when the launcher hides
//! ([`lumen_core::file_search::icon`] says which, and why).

use std::collections::HashMap;
use std::hash::Hash;
use std::path::{Path, PathBuf};
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
/// The search field's magnifying glass (F15).
pub const SEARCH: &str = "icons/search.svg";

/// Lumen's mark, for the search field and the footer (F15).
pub const LOGO: &str = "icons/lumen.svg";

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
        Kind::Folder => "icons/folder.svg",
        Kind::Quicklink | Kind::Website => "icons/link.svg",
        Kind::Timer => "icons/timer.svg",
        Kind::Currency => "icons/currency.svg",
        Kind::Clock => "icons/clock.svg",
        Kind::Weather => "icons/weather-partly-cloudy.svg",
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

/// What a file's icon is kept as (F16).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum FileKey {
    /// Its type's, by lower-case extension: one for every `.json`.
    Type(String),
    /// Its own.
    Own(PathBuf),
}

/// Icons, extracted off the UI thread and cached by source.
///
/// Cloned into a virtualised list's row closure; every clone is the same
/// cache.
#[derive(Clone)]
pub struct IconCache {
    platform: Arc<dyn Platform>,
    slots: Arc<Mutex<HashMap<IconSource, Slot>>>,
    files: Arc<Mutex<HashMap<FileKey, Slot>>>,
    size_px: u32,
}

impl IconCache {
    pub fn new(platform: Arc<dyn Platform>, size_px: u32) -> Self {
        Self {
            platform,
            slots: Arc::new(Mutex::new(HashMap::new())),
            files: Arc::new(Mutex::new(HashMap::new())),
            size_px,
        }
    }

    /// The icon for `source`, queueing extraction if it has not been seen.
    ///
    /// Returns `None` while loading or after a failure; the caller draws the
    /// reserved placeholder either way, so both cases look the same on screen.
    pub fn get(&self, source: &IconSource, cx: &mut App) -> Option<Arc<RenderImage>> {
        {
            let mut slots = self.slots.lock();
            match slots.get(source) {
                Some(Slot::Ready(image)) => return Some(image.clone()),
                Some(Slot::Loading | Slot::Failed) => return None,
                None => {
                    slots.insert(source.clone(), Slot::Loading);
                }
            }
        }

        self.spawn_load(&self.slots, source.clone(), source.clone(), cx);
        None
    }

    /// The shell's icon for the file or folder at `path` (F16), as
    /// [`get`](Self::get): `None` while it loads and when it cannot be had.
    pub fn file(&self, path: &Path, folder: bool, cx: &mut App) -> Option<Arc<RenderImage>> {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default();
        let key = match lumen_core::file_search::icon::shared(&name, folder) {
            Some(extension) => FileKey::Type(extension),
            None => FileKey::Own(path.to_path_buf()),
        };
        {
            let mut files = self.files.lock();
            match files.get(&key) {
                Some(Slot::Ready(image)) => return Some(image.clone()),
                Some(Slot::Loading | Slot::Failed) => return None,
                None => {
                    files.insert(key.clone(), Slot::Loading);
                }
            }
        }

        // A type's icon is extracted from the first file of it drawn.
        let source = IconSource::ShellItem(path.display().to_string());
        self.spawn_load(&self.files, key, source, cx);
        None
    }

    /// Forget every file's own icon, and free its texture; a type's stays.
    ///
    /// Called when the launcher hides and when a view closes: an icon kept
    /// by path is one of hundreds a search can show, and worth its memory
    /// only while it is on screen (the idle budget). A type that failed is
    /// forgotten too, so the next file of it is asked.
    pub fn release_files(&self, cx: &mut App) {
        let released: Vec<Arc<RenderImage>> = {
            let mut files = self.files.lock();
            let mut released = Vec::new();
            files.retain(|key, slot| match (key, slot) {
                (FileKey::Type(_), Slot::Ready(_)) => true,
                (_, Slot::Ready(image)) => {
                    released.push(image.clone());
                    false
                }
                // A load in flight is dropped when it lands.
                (_, Slot::Loading | Slot::Failed) => false,
            });
            released
        };
        if released.is_empty() {
            return;
        }
        // Deferred to when no window is being drawn, as the pictures are.
        cx.defer(move |cx| {
            for image in released {
                cx.drop_image(image, None);
            }
        });
    }

    /// Extract `source` off the UI thread into `slots` under `key`, which
    /// the caller has marked as loading. Kept only while it still is: a
    /// slot released while it loaded stays released.
    fn spawn_load<K>(
        &self,
        slots: &Arc<Mutex<HashMap<K, Slot>>>,
        key: K,
        source: IconSource,
        cx: &mut App,
    ) where
        K: Clone + Eq + Hash + Send + 'static,
    {
        let platform = self.platform.clone();
        let slots = slots.clone();
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
            {
                let mut slots = slots.lock();
                if !matches!(slots.get(&key), Some(Slot::Loading)) {
                    return;
                }
                slots.insert(key, slot);
            }

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
pub(crate) fn to_render_image(icon: IconImage) -> RenderImage {
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
