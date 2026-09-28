//! Quicklink icons from the sites themselves (F6).
//!
//! A link shows its site's icon where one has been fetched, and the quicklink
//! glyph otherwise: before the first fetch, offline, or when the site has
//! none. Nothing waits on the network. The launcher draws whatever the cache
//! holds, a background pass fills the cache, and a reindex picks the new files
//! up -- the same way a new application's icon arrives.
//!
//! # From the site, not a service
//!
//! The icon is read from the page the link points at: its `<link rel="icon">`
//! tags, then `/favicon.ico`. A favicon service would be one request and
//! always a clean PNG, but it would also learn the domain of every link the
//! user saves. Asking the site only tells it what opening the link would.
//!
//! # The cache
//!
//! One PNG per site, `ICON_RESOLUTION` square, under the local cache folder.
//! Refetched after [`STALE_AFTER`]; a refetch that fails keeps the old file.
//! A fetch that fails leaves a marker, so an offline machine or a site with no
//! icon is asked again after [`RETRY_AFTER`] rather than on every reindex.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result, bail};
use image::{RgbaImage, imageops::FilterType};
use url::Url;

use crate::layout::ICON_RESOLUTION;

/// How long a fetched icon is trusted before it is fetched again.
pub const STALE_AFTER: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// How long after a failure a site is asked again.
pub const RETRY_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// Per request. A launcher's background pass has no reason to be patient.
const TIMEOUT: Duration = Duration::from_secs(5);

/// The most read of a page or an icon. Icon links are in the `<head>`, and an
/// icon larger than this is not one.
const MAX_BYTES: u64 = 1024 * 1024;

/// How many of a page's icons are tried before `/favicon.ico`.
const MAX_CANDIDATES: usize = 3;

/// The site a link belongs to: its host, and port if it has one.
///
/// `None` for a link whose host is not known until it is opened -- one with a
/// placeholder in it -- and for anything that is not http or https.
pub fn site(link: &str) -> Option<String> {
    let link = link.trim();
    let rest = link
        .strip_prefix("https://")
        .or_else(|| link.strip_prefix("http://"))
        .or_else(|| link.strip_prefix("HTTPS://"))
        .or_else(|| link.strip_prefix("HTTP://"))?;
    let host = rest.split(['/', '?', '#']).next()?.to_ascii_lowercase();
    let usable = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
    usable.then_some(host)
}

/// Where `site`'s icon is kept.
pub fn cache_file(dir: &Path, site: &str) -> PathBuf {
    let name: String = site
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("{name}.png"))
}

fn failure_marker(dir: &Path, site: &str) -> PathBuf {
    cache_file(dir, site).with_extension("failed")
}

/// The icon to draw for `site`, if one has been fetched.
pub fn cached(dir: &Path, site: &str) -> Option<PathBuf> {
    let file = cache_file(dir, site);
    file.is_file().then_some(file)
}

/// One icon a page offers, and how well it would suit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub url: Url,
    /// Higher is better.
    pub score: u32,
}

/// The icons `html` offers, best first.
///
/// Read from `<link>` tags whose `rel` names an icon. A mask icon is left
/// out: it is a one-colour silhouette meant to be tinted, and drawn as it is
/// it is a black shape.
pub fn candidates(html: &str, page: &Url) -> Vec<Candidate> {
    let lower = html.to_ascii_lowercase();
    let mut found = Vec::new();

    let mut from = 0;
    while let Some(start) = lower[from..].find("<link").map(|at| from + at) {
        let end = lower[start..]
            .find('>')
            .map_or(lower.len(), |at| start + at);
        let tag = &html[start..end];
        from = end;

        let rel = attribute(tag, "rel")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let words: Vec<&str> = rel.split_whitespace().collect();
        let is_icon = words.iter().any(|word| {
            matches!(
                *word,
                "icon" | "apple-touch-icon" | "apple-touch-icon-precomposed"
            )
        });
        if !is_icon || words.contains(&"mask-icon") {
            continue;
        }
        let Some(url) = attribute(tag, "href").and_then(|href| page.join(href.trim()).ok()) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https") {
            continue;
        }

        let svg = attribute(tag, "type").is_some_and(|kind| kind.contains("svg"))
            || url.path().to_ascii_lowercase().ends_with(".svg");
        let size = attribute(tag, "sizes")
            .map(|sizes| largest_size(&sizes))
            .unwrap_or(0);
        let touch = words
            .iter()
            .any(|word| word.starts_with("apple-touch-icon"));
        found.push(Candidate {
            score: score(size, svg, touch),
            url,
        });
    }

    // Best first; the page's own order among equals.
    found.sort_by(|a, b| b.score.cmp(&a.score));
    found.dedup_by(|a, b| a.url == b.url);
    found
}

/// How well an icon of this kind would draw at `ICON_RESOLUTION`.
///
/// Scalable is best. Then the smallest size at least as large as needed,
/// since it was drawn nearest the size it will be shown at; then anything
/// smaller, largest first. An icon that does not say its size is usually a
/// small one -- except a touch icon, which is 180 unless it says otherwise.
fn score(size: u32, svg: bool, touch: bool) -> u32 {
    let size = match (size, touch) {
        (0, true) => 180,
        (0, false) => 16,
        (size, _) => size,
    };
    if svg {
        return 3000;
    }
    if size >= ICON_RESOLUTION {
        2000 - size.min(1999)
    } else {
        size
    }
}

/// The largest square in a `sizes` attribute: `"16x16 32x32"` is 32. `any`
/// is scalable, and scored as such by the caller through its type.
fn largest_size(sizes: &str) -> u32 {
    sizes
        .split_whitespace()
        .filter_map(|size| {
            let (width, height) = size
                .to_ascii_lowercase()
                .split_once('x')
                .map(|(w, h)| (w.parse::<u32>().ok(), h.parse::<u32>().ok()))?;
            Some(width?.min(height?))
        })
        .max()
        .unwrap_or(0)
}

/// An attribute's value from one tag's source, quoted either way or not at
/// all. Case-insensitive in the name, as HTML is.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(name).map(|at| from + at) {
        from = at + name.len();
        // A whole attribute name, not the tail of another (`data-href`).
        let before = lower[..at].chars().next_back();
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let rest = lower[from..].trim_start();
        if !rest.starts_with('=') {
            continue;
        }
        // Back to the original text, past the `=` and any spaces.
        let value_at = tag.len() - rest.len() + 1;
        let value = tag[value_at..].trim_start();
        let (quote, body) = match value.chars().next() {
            Some(q @ ('"' | '\'')) => (Some(q), &value[1..]),
            _ => (None, value),
        };
        let end = match quote {
            Some(q) => body.find(q).unwrap_or(body.len()),
            None => body
                .find(|c: char| c.is_whitespace() || c == '/')
                .unwrap_or(body.len()),
        };
        return Some(body[..end].to_string());
    }
    None
}

/// Fetch icons for every site in `sites` that needs one, into `dir`.
///
/// Returns how many were written. Blocking, and slow when the network is:
/// run it on a background thread, never where the list is drawn (I-3).
pub fn refresh(dir: &Path, sites: &[String]) -> usize {
    let now = SystemTime::now();
    // Several links often share a site: one fetch serves them all.
    let mut due: Vec<&String> = sites.iter().filter(|site| is_due(dir, site, now)).collect();
    due.sort();
    due.dedup();
    if due.is_empty() {
        return 0;
    }
    if let Err(error) = std::fs::create_dir_all(dir) {
        log::warn!("no favicon cache at {}: {error}", dir.display());
        return 0;
    }

    // Made for the pass and dropped after it: a client holds a runtime
    // thread, and an idle launcher should hold nothing (I-8).
    let client = match reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("Lumen/", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            log::warn!("cannot fetch favicons: {error:#}");
            return 0;
        }
    };

    let mut written = 0;
    for site in due {
        match fetch(&client, site).and_then(|icon| store(dir, site, &icon)) {
            Ok(()) => {
                log::info!("fetched the icon for {site}");
                let _ = std::fs::remove_file(failure_marker(dir, site));
                written += 1;
            }
            Err(error) => {
                // Debug, not warn: offline, every link fails, and each is the
                // expected glyph rather than a fault.
                log::debug!("no icon for {site}: {error:#}");
                let _ = std::fs::write(failure_marker(dir, site), b"");
            }
        }
    }
    written
}

/// Whether `site` has no icon, or a stale one, and has not just failed.
fn is_due(dir: &Path, site: &str, now: SystemTime) -> bool {
    // A time in the future -- the clock was put back -- is a file just
    // written, not one with no age, which would be fetched on every pass.
    let age = |path: PathBuf| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
            .map(|modified| now.duration_since(modified).unwrap_or(Duration::ZERO))
    };
    if age(failure_marker(dir, site)).is_some_and(|age| age < RETRY_AFTER) {
        return false;
    }
    age(cache_file(dir, site)).is_none_or(|age| age >= STALE_AFTER)
}

fn fetch(client: &reqwest::blocking::Client, site: &str) -> Result<RgbaImage> {
    let page = Url::parse(&format!("https://{site}/"))?;

    // The page may be down while its icon is not, and the other way round,
    // so a failed page read still leaves `/favicon.ico` to try.
    let mut urls: Vec<Url> = match get(client, &page) {
        Ok((body, final_url)) => {
            let html = String::from_utf8_lossy(&body);
            candidates(&html, &final_url)
                .into_iter()
                .take(MAX_CANDIDATES)
                .map(|candidate| candidate.url)
                .collect()
        }
        Err(error) => {
            log::debug!("could not read {page}: {error:#}");
            Vec::new()
        }
    };
    urls.push(page.join("/favicon.ico")?);

    let mut last = None;
    for url in urls {
        match get(client, &url).and_then(|(bytes, _)| decode(&bytes)) {
            Ok(icon) => return Ok(icon),
            Err(error) => last = Some(error.context(url.to_string())),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("the site offers no icon")))
}

/// The body of `url`, at most `MAX_BYTES` of it, and where it ended up.
fn get(client: &reqwest::blocking::Client, url: &Url) -> Result<(Vec<u8>, Url)> {
    let response = client.get(url.clone()).send()?.error_for_status()?;
    let final_url = response.url().clone();
    let mut body = Vec::new();
    response.take(MAX_BYTES).read_to_end(&mut body)?;
    Ok((body, final_url))
}

/// Pixels from an icon file of any kind a site is likely to serve, sized to
/// `ICON_RESOLUTION`.
pub fn decode(bytes: &[u8]) -> Result<RgbaImage> {
    let image = match image::load_from_memory(bytes) {
        Ok(image) => image.to_rgba8(),
        Err(_) if looks_like_svg(bytes) => render_svg(bytes)?,
        Err(error) => return Err(error).context("not an image"),
    };
    if image.width() < 16 || image.height() < 16 {
        bail!("too small to draw: {}x{}", image.width(), image.height());
    }
    Ok(fit(&image))
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
    head.contains("<svg")
}

fn render_svg(bytes: &[u8]) -> Result<RgbaImage> {
    let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default())?;
    let size = tree.size();
    let scale = ICON_RESOLUTION as f32 / size.width().max(size.height());
    let mut pixmap = resvg::tiny_skia::Pixmap::new(ICON_RESOLUTION, ICON_RESOLUTION)
        .context("could not allocate the icon")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia is premultiplied; the cache and the platform are straight.
    let pixels = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let straight = pixel.demultiply();
            [
                straight.red(),
                straight.green(),
                straight.blue(),
                straight.alpha(),
            ]
        })
        .collect();
    RgbaImage::from_raw(ICON_RESOLUTION, ICON_RESOLUTION, pixels).context("bad pixel buffer")
}

/// `ICON_RESOLUTION` square, the image centred in it at its own aspect.
fn fit(image: &RgbaImage) -> RgbaImage {
    let side = ICON_RESOLUTION;
    let scale = side as f32 / image.width().max(image.height()) as f32;
    let width = ((image.width() as f32 * scale).round() as u32).clamp(1, side);
    let height = ((image.height() as f32 * scale).round() as u32).clamp(1, side);
    let resized = image::imageops::resize(image, width, height, FilterType::Lanczos3);

    let mut square = RgbaImage::new(side, side);
    image::imageops::overlay(
        &mut square,
        &resized,
        ((side - width) / 2).into(),
        ((side - height) / 2).into(),
    );
    square
}

/// Write the icon beside its final name, then rename it into place, so the
/// launcher never decodes half a file.
fn store(dir: &Path, site: &str, icon: &RgbaImage) -> Result<()> {
    let file = cache_file(dir, site);
    let staging = file.with_extension("png.saving");
    icon.save_with_format(&staging, image::ImageFormat::Png)
        .with_context(|| format!("writing {}", staging.display()))?;
    std::fs::rename(&staging, &file).with_context(|| format!("replacing {}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> Url {
        Url::parse("https://www.example.com/some/page").unwrap()
    }

    #[test]
    fn a_link_belongs_to_its_host() {
        assert_eq!(
            site("https://www.google.com/search?q={query}").as_deref(),
            Some("www.google.com")
        );
        assert_eq!(site("HTTP://Example.COM").as_deref(), Some("example.com"));
        assert_eq!(
            site("http://localhost:8080/x").as_deref(),
            Some("localhost:8080")
        );
    }

    /// Known only when opened, or not a web page at all: no site to ask.
    #[test]
    fn some_links_have_no_site_to_ask() {
        assert_eq!(site("https://{query}.example.com"), None);
        assert_eq!(site("https://{clipboard}"), None);
        assert_eq!(site("ftp://example.com"), None);
        assert_eq!(site("https://"), None);
        assert_eq!(site("https://user@example.com"), None);
    }

    #[test]
    fn a_site_becomes_a_safe_file_name() {
        let dir = Path::new("cache");
        assert_eq!(
            cache_file(dir, "www.google.com"),
            dir.join("www.google.com.png")
        );
        assert_eq!(
            cache_file(dir, "localhost:8080"),
            dir.join("localhost_8080.png")
        );
    }

    #[test]
    fn icon_links_are_found_and_resolved_against_the_page() {
        let html = r#"<html><head>
            <LINK REL="icon" href="/favicon-32.png" sizes="32x32">
            <link rel='apple-touch-icon' href='touch.png'>
            <link rel=stylesheet href=/style.css>
            <link rel="shortcut icon" href="//cdn.example.com/f.ico">
        </head></html>"#;

        let urls: Vec<String> = candidates(html, &page())
            .into_iter()
            .map(|candidate| candidate.url.to_string())
            .collect();

        assert_eq!(
            urls,
            vec![
                // A touch icon is 180 when it does not say: the best fit.
                "https://www.example.com/some/touch.png",
                "https://www.example.com/favicon-32.png",
                "https://cdn.example.com/f.ico",
            ]
        );
    }

    #[test]
    fn scalable_wins_and_a_mask_icon_is_left_out() {
        let html = r#"
            <link rel="icon" href="/big.png" sizes="512x512">
            <link rel="mask-icon" href="/mask.svg" color="black">
            <link rel="icon" type="image/svg+xml" href="/icon.svg">
        "#;

        let urls: Vec<String> = candidates(html, &page())
            .into_iter()
            .map(|candidate| candidate.url.path().to_string())
            .collect();

        assert_eq!(urls, vec!["/icon.svg", "/big.png"]);
    }

    /// Nearest at or above the size it will be drawn at, then the largest
    /// below it.
    #[test]
    fn the_size_nearest_above_what_is_drawn_is_preferred() {
        assert!(score(128, false, false) > score(512, false, false));
        assert!(score(512, false, false) > score(64, false, false));
        assert!(score(64, false, false) > score(32, false, false));
        assert_eq!(largest_size("16x16 32x32 any"), 32);
    }

    #[test]
    fn an_attribute_is_not_found_inside_another_ones_name() {
        let tag = r#"<link data-href="/wrong.png" href="/right.png" rel="icon""#;
        assert_eq!(attribute(tag, "href").as_deref(), Some("/right.png"));
    }

    #[test]
    fn a_page_with_no_icon_links_offers_none() {
        assert!(candidates("<html><head><title>x</title></head></html>", &page()).is_empty());
        assert!(candidates("<link rel=\"icon\" href=\"javascript:alert(1)\">", &page()).is_empty());
    }

    #[test]
    fn a_decoded_icon_is_drawn_square_at_the_icon_resolution() {
        let mut png = Vec::new();
        RgbaImage::from_pixel(64, 32, image::Rgba([255, 0, 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();

        let icon = decode(&png).unwrap();

        assert_eq!(
            (icon.width(), icon.height()),
            (ICON_RESOLUTION, ICON_RESOLUTION)
        );
        // Letterboxed, not stretched: the top row is empty.
        assert_eq!(icon.get_pixel(0, 0)[3], 0);
        assert_eq!(
            icon.get_pixel(ICON_RESOLUTION / 2, ICON_RESOLUTION / 2)[0],
            255
        );
    }

    #[test]
    fn an_svg_icon_is_rendered() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">
            <rect width="10" height="10" fill="#00ff00"/></svg>"##;
        let icon = decode(svg).unwrap();
        assert_eq!(icon.get_pixel(48, 48)[1], 255);
    }

    #[test]
    fn a_page_is_not_an_icon() {
        assert!(decode(b"<html><body>Not found</body></html>").is_err());
    }

    #[test]
    fn a_site_that_just_failed_is_not_asked_again_yet() {
        let dir = std::env::temp_dir().join(format!("lumen-favicons-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let now = SystemTime::now();

        assert!(is_due(&dir, "a.example", now));
        std::fs::write(failure_marker(&dir, "a.example"), b"").unwrap();
        assert!(!is_due(&dir, "a.example", now));
        assert!(is_due(
            &dir,
            "a.example",
            now + RETRY_AFTER + Duration::from_secs(1)
        ));

        std::fs::write(cache_file(&dir, "b.example"), b"png").unwrap();
        assert!(!is_due(&dir, "b.example", now));
        assert!(is_due(
            &dir,
            "b.example",
            now + STALE_AFTER + Duration::from_secs(1)
        ));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
