//! Pictures the host wrote, loaded for drawing (F12): a copied image's
//! thumbnail in a row and the image itself in the pane.
//!
//! [`crate::icons`]'s rules, and one more. Never on the UI thread; the space
//! is reserved before the picture arrives (I-10); and **bounded**. An icon is
//! a few kilobytes and there are a few hundred; a screenshot is megabytes of
//! pixels and a GPU texture beside them, and a history can hold hundreds. So
//! this keeps a budget of decoded bytes, forgets the least recently drawn
//! past it, and forgets everything when the launcher hides: nothing here is
//! worth its memory while nobody is looking (the idle budget).
//!
//! Forgetting a picture frees its texture too, with `drop_image`, deferred to
//! when no window is being drawn -- a texture dropped from inside a window's
//! update would be missed in that very window.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{App, RenderImage};
use lumen_platform::IconImage;
use parking_lot::Mutex;

/// The longest side a pane's picture is decoded at. The pane is narrower than
/// this; a screenshot is not.
pub const PREVIEW_PX: u32 = 1024;

/// Decoded bytes held before the least recently drawn picture goes: a few
/// previews and every thumbnail a screen can show, with room to spare.
const BUDGET: usize = 24 * 1024 * 1024;

/// How a picture is fitted to its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fit {
    /// Whole, within the size on its longest side: the pane.
    Within,
    /// The centre square, filling the size: a row's icon square, where a
    /// tall or wide picture fitted whole would be a sliver.
    Square,
}

type Key = (PathBuf, u32, Fit);

enum Slot {
    Loading,
    Ready(Arc<RenderImage>, usize),
    Failed,
}

#[derive(Default)]
struct State {
    slots: HashMap<Key, Slot>,
    /// Ready pictures, least recently drawn first.
    order: VecDeque<Key>,
    bytes: usize,
}

/// Pictures by path and size, decoded off the UI thread.
#[derive(Clone, Default)]
pub struct ImageCache {
    state: Arc<Mutex<State>>,
}

impl ImageCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The picture at `path`, fitted within `max_px` on its longest side,
    /// queueing it if it has not been seen. `None` while it loads or when it
    /// could not be read; the caller draws the reserved space either way.
    pub fn get(
        &self,
        path: &Path,
        max_px: u32,
        fit: Fit,
        cx: &mut App,
    ) -> Option<Arc<RenderImage>> {
        let key = (path.to_path_buf(), max_px, fit);
        {
            let mut state = self.state.lock();
            match state.slots.get(&key) {
                Some(Slot::Ready(image, _)) => {
                    let image = image.clone();
                    if let Some(at) = state.order.iter().position(|seen| *seen == key) {
                        state.order.remove(at);
                    }
                    state.order.push_back(key);
                    return Some(image);
                }
                Some(Slot::Loading | Slot::Failed) => return None,
                None => {
                    state.slots.insert(key.clone(), Slot::Loading);
                }
            }
        }
        self.load(key, cx);
        None
    }

    /// Forget every picture, and free their textures.
    pub fn release(&self, cx: &mut App) {
        let released: Vec<Arc<RenderImage>> = {
            let mut state = self.state.lock();
            state.order.clear();
            state.bytes = 0;
            state
                .slots
                .drain()
                .filter_map(|(_, slot)| match slot {
                    Slot::Ready(image, _) => Some(image),
                    Slot::Loading | Slot::Failed => None,
                })
                .collect()
        };
        free(released, cx);
    }

    fn load(&self, key: Key, cx: &mut App) {
        let state = self.state.clone();
        cx.spawn(async move |cx| {
            let loaded = cx
                .background_executor()
                .spawn({
                    let (path, max_px, fit) = key.clone();
                    async move { decode(&path, max_px, fit) }
                })
                .await;

            let evicted = {
                let mut state = state.lock();
                // Released while it loaded: the launcher has hidden.
                if !matches!(state.slots.get(&key), Some(Slot::Loading)) {
                    return;
                }
                match loaded {
                    Ok(icon) => {
                        let bytes = icon.rgba.len();
                        let image = Arc::new(crate::icons::to_render_image(icon));
                        state.slots.insert(key.clone(), Slot::Ready(image, bytes));
                        state.order.push_back(key);
                        state.bytes += bytes;
                        evict(&mut state)
                    }
                    Err(error) => {
                        log::debug!("could not draw {}: {error:#}", key.0.display());
                        state.slots.insert(key, Slot::Failed);
                        Vec::new()
                    }
                }
            };
            cx.update(|cx| {
                free(evicted, cx);
                // A row is on screen with its space reserved; nothing repaints
                // until something asks for a frame.
                cx.refresh_windows();
            })
            .ok();
        })
        .detach();
    }
}

/// The least recently drawn pictures past the budget, taken out. The newest
/// always stays, however large, or it would be loaded again every frame.
fn evict(state: &mut State) -> Vec<Arc<RenderImage>> {
    let mut evicted = Vec::new();
    while state.bytes > BUDGET && state.order.len() > 1 {
        let Some(key) = state.order.pop_front() else {
            break;
        };
        if let Some(Slot::Ready(image, bytes)) = state.slots.remove(&key) {
            state.bytes -= bytes;
            evicted.push(image);
        }
    }
    evicted
}

fn free(images: Vec<Arc<RenderImage>>, cx: &mut App) {
    if images.is_empty() {
        return;
    }
    cx.defer(move |cx| {
        for image in images {
            cx.drop_image(image, None);
        }
    });
}

/// Read and fit a picture, off the UI thread.
fn decode(path: &Path, max_px: u32, fit: Fit) -> anyhow::Result<IconImage> {
    let mut image = image::open(path)?.into_rgba8();
    if fit == Fit::Square {
        let (width, height) = image.dimensions();
        let side = width.min(height);
        image =
            image::imageops::crop_imm(&image, (width - side) / 2, (height - side) / 2, side, side)
                .to_image();
    }
    let (width, height) = image.dimensions();
    let longest = width.max(height);
    if longest > max_px {
        let scale = f64::from(max_px) / f64::from(longest);
        let fit = |side: u32| ((f64::from(side) * scale).round() as u32).max(1);
        image = image::imageops::resize(
            &image,
            fit(width),
            fit(height),
            image::imageops::FilterType::Triangle,
        );
    }
    Ok(IconImage {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(state: &mut State, name: &str, bytes: usize) {
        let key = (PathBuf::from(name), 1, Fit::Within);
        let pixels = vec![0u8; 4];
        let image = Arc::new(crate::icons::to_render_image(IconImage {
            width: 1,
            height: 1,
            rgba: pixels,
        }));
        state.slots.insert(key.clone(), Slot::Ready(image, bytes));
        state.order.push_back(key);
        state.bytes += bytes;
    }

    #[test]
    fn the_least_recently_drawn_go_first_and_the_newest_always_stays() {
        let mut state = State::default();
        ready(&mut state, "old", BUDGET / 2);
        ready(&mut state, "mid", BUDGET / 2);
        ready(&mut state, "new", BUDGET / 2);

        assert_eq!(evict(&mut state).len(), 1);
        assert!(
            !state
                .slots
                .contains_key(&(PathBuf::from("old"), 1, Fit::Within))
        );
        assert_eq!(state.bytes, BUDGET);

        let mut huge = State::default();
        ready(&mut huge, "huge", BUDGET * 2);
        assert!(evict(&mut huge).is_empty());
    }

    #[test]
    fn a_picture_is_fitted_within_its_size() {
        let dir = std::env::temp_dir().join(format!("lumen-ui-images-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wide.png");
        image::RgbaImage::new(400, 100).save(&path).unwrap();

        let fitted = decode(&path, 100, Fit::Within).unwrap();
        assert_eq!((fitted.width, fitted.height), (100, 25));
        let small = decode(&path, 1000, Fit::Within).unwrap();
        assert_eq!((small.width, small.height), (400, 100));
        // A wide picture's centre, filling a square.
        let square = decode(&path, 50, Fit::Square).unwrap();
        assert_eq!((square.width, square.height), (50, 50));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
