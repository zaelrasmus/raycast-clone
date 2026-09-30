//! Copied images, kept as files beside the history (F12).
//!
//! The database holds a hash; the pixels are PNG files named by it, in a
//! folder of their own. A screenshot is megabytes, and a history of them as
//! rows is a database that grows by megabytes per copy and a write-ahead log
//! that rewrites them. As files, SQLite stays small, the same picture copied
//! twice is one file, and deleting an entry is deleting two files.
//!
//! Each picture is kept twice: the original, which is what is pasted, and a
//! thumbnail at the icon resolution, which is what a row draws. Scaling a
//! screenshot down every time the view opens would be the same work, again,
//! for every visible row.
//!
//! The hash is of the pixels rather than of the PNG, so the same picture
//! copied from two programs that encode differently is still one entry.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result, bail};
use image::{ImageFormat, RgbaImage};
use lumen_platform::ClipboardImage;
use sha2::{Digest, Sha256};

/// The longest side of a row's thumbnail, in pixels.
pub const THUMBNAIL_PX: u32 = crate::layout::ICON_RESOLUTION;

/// The largest picture recorded. Past this a copy is a poster or a scan, and
/// holding its pixels to encode it costs more memory than the whole launcher.
pub const MAX_PIXELS: u64 = 40_000_000;

/// A file younger than this is never swept: it may be a picture written a
/// moment ago whose row is about to be recorded.
const SWEEP_GRACE: Duration = Duration::from_secs(60);

/// A picture that has been written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub hash: String,
    pub width: u32,
    pub height: u32,
    /// The PNG's size on disk.
    pub bytes: u64,
}

/// Write `image` into `dir`, unless it is already there.
pub fn store(dir: &Path, image: &ClipboardImage) -> Result<Stored> {
    let pixels = u64::from(image.width) * u64::from(image.height);
    if pixels == 0 || image.rgba.len() as u64 != pixels * 4 {
        bail!(
            "the picture's pixels do not match its size ({}x{})",
            image.width,
            image.height
        );
    }
    if pixels > MAX_PIXELS {
        bail!(
            "the picture is too large to keep ({}x{})",
            image.width,
            image.height
        );
    }

    let hash = hash(image);
    let original = original(dir, &hash);
    if !original.exists() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let buffer = RgbaImage::from_raw(image.width, image.height, image.rgba.clone())
            .expect("the length was checked above");
        write(&thumbnail(dir, &hash), &encode(&shrink(&buffer))?)?;
        // The original last: its presence is what says the pair is complete.
        write(&original, &encode(&buffer)?)?;
    }
    let bytes = std::fs::metadata(&original)
        .with_context(|| format!("reading {}", original.display()))?
        .len();

    Ok(Stored {
        hash,
        width: image.width,
        height: image.height,
        bytes,
    })
}

/// A picture back, as pixels and as the PNG it was kept as, for pasting.
pub fn load(dir: &Path, hash: &str) -> Result<ClipboardImage> {
    let path = original(dir, hash);
    let png = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let decoded = image::load_from_memory_with_format(&png, ImageFormat::Png)
        .with_context(|| format!("decoding {}", path.display()))?
        .into_rgba8();
    Ok(ClipboardImage {
        width: decoded.width(),
        height: decoded.height(),
        rgba: decoded.into_raw(),
        png: Some(png),
    })
}

/// The original, which is what is pasted and what the pane shows.
pub fn original(dir: &Path, hash: &str) -> PathBuf {
    dir.join(format!("{}.png", name(hash)))
}

/// The thumbnail a row draws.
pub fn thumbnail(dir: &Path, hash: &str) -> PathBuf {
    dir.join(format!("{}.thumb.png", name(hash)))
}

/// Remove every picture in `dir` whose hash `keep` does not name.
///
/// Called after rows are deleted, rather than deleting files row by row:
/// one pass is right however the rows went -- pruned, deleted, cleared --
/// and it also collects what a crash between the file and the row left.
pub fn sweep(dir: &Path, keep: &[String]) -> Result<usize> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error).with_context(|| format!("reading {}", dir.display())),
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(hash) = file_name
            .to_str()
            .and_then(|name| name.strip_suffix(".png"))
            .map(|name| name.strip_suffix(".thumb").unwrap_or(name))
        else {
            continue;
        };
        if !is_hash(hash) || keep.iter().any(|kept| kept == hash) {
            continue;
        }
        let recent = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_none_or(|age| age < SWEEP_GRACE);
        if recent {
            continue;
        }
        if std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Where a picture copied from a browser lives online, from the HTML the
/// browser puts beside it (`CF_HTML`): the first `<img>`'s `src`, resolved
/// against the page's `SourceURL` when it is relative.
///
/// Only `http` and `https`: a `data:` URI is the picture again, and a
/// `file:` or `blob:` one names nothing another program could open. `None`
/// when there is no picture in the HTML, which is most HTML.
pub fn origin(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let tag_start = lower.find("<img")?;
    let tag_end = lower[tag_start..]
        .find('>')
        .map_or(lower.len(), |end| tag_start + end);
    let tag = &html[tag_start..tag_end];
    let tag_lower = &lower[tag_start..tag_end];

    // ` src=` and not `data-src=` or `srcset=`.
    let at = tag_lower
        .match_indices("src")
        .map(|(at, _)| at)
        .find(|&at| {
            let before = tag_lower[..at].chars().next_back();
            let after = tag_lower[at + 3..].trim_start();
            before.is_some_and(char::is_whitespace) && after.starts_with('=')
        })?;
    let value = tag[at + 3..].trim_start().strip_prefix('=')?.trim_start();
    let value = match value.chars().next()? {
        quote @ ('"' | '\'') => value[1..].split(quote).next()?,
        _ => value
            .split(|c: char| c.is_whitespace() || c == '>')
            .next()?,
    };
    let value = value.replace("&amp;", "&");

    let base = html
        .lines()
        .find_map(|line| line.strip_prefix("SourceURL:"))
        .and_then(|source| url::Url::parse(source.trim()).ok());
    let address = match (url::Url::parse(&value), base) {
        (Ok(address), _) => address,
        (Err(url::ParseError::RelativeUrlWithoutBase), Some(base)) => base.join(&value).ok()?,
        _ => return None,
    };
    matches!(address.scheme(), "http" | "https").then(|| address.to_string())
}

/// *54 kB*, *1.2 MB*: decimal, as Explorer's details pane and Raycast write
/// a file's size.
pub fn size_label(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["bytes", "kB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

fn hash(image: &ClipboardImage) -> String {
    let mut hasher = Sha256::new();
    hasher.update(image.width.to_le_bytes());
    hasher.update(image.height.to_le_bytes());
    hasher.update(&image.rgba);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A hash from the database, used as a file name only if it is one: a row
/// that says `..\..\something` names nothing.
fn name(hash: &str) -> &str {
    if is_hash(hash) { hash } else { "invalid" }
}

fn is_hash(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn shrink(image: &RgbaImage) -> RgbaImage {
    let (width, height) = image.dimensions();
    let longest = width.max(height);
    if longest <= THUMBNAIL_PX {
        return image.clone();
    }
    let scale = f64::from(THUMBNAIL_PX) / f64::from(longest);
    let fit = |side: u32| ((f64::from(side) * scale).round() as u32).max(1);
    image::imageops::thumbnail(image, fit(width), fit(height))
}

fn encode(image: &RgbaImage) -> Result<Vec<u8>> {
    let mut png = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .context("encoding a picture as PNG")?;
    Ok(png)
}

/// Written beside and renamed over, so a picture is never half there.
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let partial = path.with_extension("partial");
    std::fs::write(&partial, bytes).with_context(|| format!("writing {}", partial.display()))?;
    std::fs::rename(&partial, path).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lumen-images-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn picture(width: u32, height: u32, seed: u8) -> ClipboardImage {
        ClipboardImage {
            width,
            height,
            rgba: (0..width * height * 4)
                .map(|i| (i as u8).wrapping_add(seed))
                .collect(),
            png: None,
        }
    }

    #[test]
    fn a_picture_is_stored_once_with_a_thumbnail_and_comes_back_the_same() {
        let dir = dir("store");
        let image = picture(300, 150, 0);

        let stored = store(&dir, &image).unwrap();
        assert_eq!((stored.width, stored.height), (300, 150));
        assert!(stored.bytes > 0);
        assert_eq!(
            store(&dir, &image).unwrap(),
            stored,
            "the same picture again"
        );

        let thumb = image::open(thumbnail(&dir, &stored.hash)).unwrap();
        assert_eq!(thumb.width(), THUMBNAIL_PX);
        assert_eq!(thumb.height(), THUMBNAIL_PX / 2);

        let back = load(&dir, &stored.hash).unwrap();
        assert_eq!((back.width, back.height), (300, 150));
        assert_eq!(back.rgba, image.rgba);
        assert!(back.png.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn different_pictures_have_different_names() {
        let dir = dir("names");
        let a = store(&dir, &picture(4, 4, 0)).unwrap();
        let b = store(&dir, &picture(4, 4, 1)).unwrap();
        let c = store(&dir, &picture(2, 8, 0)).unwrap();
        assert_ne!(a.hash, b.hash);
        assert_ne!(a.hash, c.hash, "same bytes, another shape");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_picture_that_does_not_add_up_is_refused() {
        let dir = dir("refused");
        let mut image = picture(4, 4, 0);
        image.rgba.pop();
        assert!(store(&dir, &image).is_err());
        assert!(store(&dir, &picture(0, 4, 0)).is_err());
    }

    #[test]
    fn a_sweep_keeps_what_is_named_and_what_is_new() {
        let dir = dir("sweep");
        let kept = store(&dir, &picture(4, 4, 0)).unwrap();
        let gone = store(&dir, &picture(4, 4, 1)).unwrap();

        // Just written: kept whatever the rows say.
        assert_eq!(sweep(&dir, &[]).unwrap(), 0);

        let old = SystemTime::now() - SWEEP_GRACE * 2;
        for path in std::fs::read_dir(&dir).unwrap().flatten() {
            std::fs::File::options()
                .write(true)
                .open(path.path())
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        assert_eq!(sweep(&dir, std::slice::from_ref(&kept.hash)).unwrap(), 2);
        assert!(original(&dir, &kept.hash).exists());
        assert!(thumbnail(&dir, &kept.hash).exists());
        assert!(!original(&dir, &gone.hash).exists());

        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(sweep(&dir, &[]).unwrap(), 0, "no folder is nothing to do");
    }

    #[test]
    fn a_hash_from_the_database_cannot_name_another_file() {
        let dir = Path::new("C:\\images");
        assert_eq!(original(dir, "..\\..\\secret"), dir.join("invalid.png"));
    }

    #[test]
    fn a_picture_copied_from_a_page_knows_where_it_lives() {
        let html = "Version:0.9\r\nStartHTML:0000000105\r\n\
                    SourceURL:https://example.com/gallery/index.html\r\n\
                    <html><body><!--StartFragment-->\
                    <img alt=\"cat\" data-src=\"x\" src=\"https://cdn.example.com/cat.png?a=1&amp;b=2\">\
                    <!--EndFragment--></body></html>";
        assert_eq!(
            origin(html).as_deref(),
            Some("https://cdn.example.com/cat.png?a=1&b=2")
        );

        let relative =
            "SourceURL:https://example.com/gallery/index.html\r\n<IMG SRC='../img/dog.jpg'>";
        assert_eq!(
            origin(relative).as_deref(),
            Some("https://example.com/img/dog.jpg")
        );
    }

    #[test]
    fn html_without_an_address_to_open_has_no_origin() {
        assert_eq!(origin("<p>just text</p>"), None);
        assert_eq!(origin("<img src=\"data:image/png;base64,AAAA\">"), None);
        assert_eq!(origin("<img src=\"cat.png\">"), None, "relative, no page");
        assert_eq!(origin("<img srcset=\"a.png 2x\">"), None);
    }

    #[test]
    fn sizes_are_written_as_explorer_writes_them() {
        assert_eq!(size_label(512), "512 bytes");
        assert_eq!(size_label(54_321), "54 kB");
        assert_eq!(size_label(1_234_567), "1.2 MB");
    }
}
