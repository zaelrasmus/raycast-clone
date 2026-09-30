//! What a clipboard entry is, for the row's icon, the *Type* filter and the
//! pane (F12).
//!
//! Decided when an entry is read, not when it is recorded: only the payload
//! the system distinguishes is stored ([`Payload`]), and a link or a colour is
//! text that reads as one. A rule improved here applies to the whole history
//! the next time the view opens.
//!
//! Both rules are strict on purpose. The entry must be *only* a link or *only*
//! a colour, on one line; a paragraph that mentions a URL is text. A false
//! positive would put a swatch or an *Open in Browser* on something that is
//! neither, and a false negative only costs a plainer row.

use super::store::{Entry, Payload};
use crate::theme::Color;

/// What an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Link,
    Color(Color),
    Files,
}

impl Kind {
    /// Every kind, in the order the *Type* filter lists them.
    pub const FILTERS: [Filter; 4] = [Filter::Text, Filter::Link, Filter::Color, Filter::Files];

    pub fn filter(self) -> Filter {
        match self {
            Kind::Text => Filter::Text,
            Kind::Link => Filter::Link,
            Kind::Color(_) => Filter::Color,
            Kind::Files => Filter::Files,
        }
    }
}

/// A kind without its value, for the *Type* filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Text,
    Link,
    Color,
    Files,
}

impl Filter {
    /// The id the scope option carries.
    pub fn id(self) -> &'static str {
        match self {
            Filter::Text => "text",
            Filter::Link => "link",
            Filter::Color => "color",
            Filter::Files => "files",
        }
    }

    /// The option's label, plural: what the list is narrowed to.
    pub fn label(self) -> &'static str {
        match self {
            Filter::Text => "Text",
            Filter::Link => "Links",
            Filter::Color => "Colors",
            Filter::Files => "Files",
        }
    }

    /// The pane's *Type* row, singular.
    pub fn name(self) -> &'static str {
        match self {
            Filter::Text => "Text",
            Filter::Link => "Link",
            Filter::Color => "Color",
            Filter::Files => "File",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Kind::FILTERS.into_iter().find(|filter| filter.id() == id)
    }
}

/// What `entry` is.
pub fn kind(entry: &Entry) -> Kind {
    match entry.payload {
        Payload::Files => Kind::Files,
        Payload::Text => classify(&entry.text),
    }
}

/// What a piece of copied text reads as.
pub fn classify(text: &str) -> Kind {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.contains(['\n', '\r']) {
        return Kind::Text;
    }
    if let Some(color) = color(trimmed) {
        return Kind::Color(color);
    }
    if link(trimmed).is_some() {
        return Kind::Link;
    }
    Kind::Text
}

/// The address to open for a link, with `https://` added to a bare `www.`.
pub fn link(text: &str) -> Option<String> {
    let text = text.trim();
    if text.chars().any(char::is_whitespace) {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    let (rest, address) = match ["https://", "http://"]
        .into_iter()
        .find_map(|scheme| lower.strip_prefix(scheme))
    {
        Some(rest) => (rest, text.to_string()),
        None if lower.starts_with("www.") => (lower.as_str(), format!("https://{text}")),
        None => return None,
    };
    // A host: something before the first separator, with a dot in it or
    // `localhost`. `https://` alone, or `www.`, is not an address.
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    let name = host.split(':').next().unwrap_or(host);
    let valid = name == "localhost"
        || (name.contains('.')
            && !name.starts_with('.')
            && !name.ends_with('.')
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '.' || c == '-'));
    valid.then_some(address)
}

/// A colour written in CSS's common forms: `#rgb`, `#rgba`, `#rrggbb`,
/// `#rrggbbaa`, `rgb()`, `rgba()`, `hsl()` and `hsla()`.
pub fn color(text: &str) -> Option<Color> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('#') {
        return hex_color(hex);
    }
    let lower = text.to_ascii_lowercase();
    let open = lower.find('(')?;
    let body = lower[open + 1..].strip_suffix(')')?;
    let function = lower[..open].trim();
    // Commas, or the space-separated form with `/` before the alpha.
    let parts: Vec<&str> = body
        .split([',', ' ', '/'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    let alpha = |part: Option<&&str>| -> Option<u8> {
        match part {
            None => Some(255),
            Some(part) => {
                let value = match part.strip_suffix('%') {
                    Some(percent) => percent.parse::<f32>().ok()? / 100.0,
                    None => part.parse::<f32>().ok()?,
                };
                (0.0..=1.0)
                    .contains(&value)
                    .then(|| (value * 255.0).round() as u8)
            }
        }
    };
    match function {
        "rgb" | "rgba" if parts.len() == 3 || parts.len() == 4 => {
            let channel = |part: &str| -> Option<u8> {
                let value = match part.strip_suffix('%') {
                    Some(percent) => percent.parse::<f32>().ok()? * 2.55,
                    None => part.parse::<f32>().ok()?,
                };
                (0.0..=255.0).contains(&value).then(|| value.round() as u8)
            };
            Some(Color::rgba(
                channel(parts[0])?,
                channel(parts[1])?,
                channel(parts[2])?,
                alpha(parts.get(3))?,
            ))
        }
        "hsl" | "hsla" if parts.len() == 3 || parts.len() == 4 => {
            let hue = parts[0].strip_suffix("deg").unwrap_or(parts[0]);
            let hue = hue.parse::<f32>().ok()?.rem_euclid(360.0);
            let percent = |part: &str| -> Option<f32> {
                let value = part.strip_suffix('%')?.parse::<f32>().ok()?;
                (0.0..=100.0).contains(&value).then_some(value / 100.0)
            };
            let (r, g, b) = hsl_to_rgb(hue, percent(parts[1])?, percent(parts[2])?);
            Some(Color::rgba(r, g, b, alpha(parts.get(3))?))
        }
        _ => None,
    }
}

fn hex_color(hex: &str) -> Option<Color> {
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        3 | 6 | 8 => Color::parse(hex).ok(),
        // `#rgba`, which the theme does not accept: widen it to `#rrggbbaa`.
        4 => Color::parse(&hex.chars().flat_map(|c| [c, c]).collect::<String>()).ok(),
        _ => None,
    }
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> (u8, u8, u8) {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue / 60.0;
    let x = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let (r, g, b) = match sector as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = lightness - chroma / 2.0;
    let byte = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (byte(r), byte(g), byte(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_in_every_common_form_are_recognised() {
        let red = Color::rgb(255, 0, 0);
        for text in [
            "#f00",
            "#FF0000",
            " #ff0000 ",
            "rgb(255, 0, 0)",
            "rgb(255 0 0)",
            "rgb(100%, 0%, 0%)",
            "hsl(0, 100%, 50%)",
            "hsl(360deg 100% 50%)",
        ] {
            assert_eq!(classify(text), Kind::Color(red), "{text:?}");
        }
        assert_eq!(color("#f008"), Some(Color::rgba(255, 0, 0, 0x88)));
        assert_eq!(color("#ff000080"), Some(Color::rgba(255, 0, 0, 0x80)));
        assert_eq!(
            color("rgba(0, 128, 255, 0.5)"),
            Some(Color::rgba(0, 128, 255, 128))
        );
        assert_eq!(
            color("hsla(120, 100%, 25%, 50%)"),
            Some(Color::rgba(0, 128, 0, 128))
        );
    }

    /// A false positive puts a swatch on something that is not a colour.
    #[test]
    fn things_that_only_look_like_colours_are_text() {
        for text in [
            "#12345",
            "#ggg",
            "#issue",
            "#1",
            "rgb(256, 0, 0)",
            "rgb(1, 2)",
            "hsl(0, 100, 50)",
            "the colour is #ff0000",
            "#ff0000\n#00ff00",
        ] {
            assert_eq!(classify(text), Kind::Text, "{text:?}");
        }
    }

    #[test]
    fn links_are_recognised_and_bare_www_is_given_a_scheme() {
        assert_eq!(classify("https://example.com"), Kind::Link);
        assert_eq!(classify("http://localhost:3000/x"), Kind::Link);
        assert_eq!(classify("HTTPS://Example.com/a?b#c"), Kind::Link);
        assert_eq!(
            link("www.example.com/page").as_deref(),
            Some("https://www.example.com/page")
        );
        assert_eq!(
            link("https://example.com").as_deref(),
            Some("https://example.com")
        );
    }

    #[test]
    fn text_that_mentions_a_link_is_text() {
        for text in [
            "see https://example.com",
            "https://",
            "https://nodot",
            "www.",
            "https://example.com\nsecond line",
            "example.com",
        ] {
            assert_eq!(classify(text), Kind::Text, "{text:?}");
        }
    }

    #[test]
    fn a_file_list_is_files_whatever_it_says() {
        let entry = Entry {
            id: 1,
            payload: Payload::Files,
            text: "https://example.com".into(),
            copied_at: 0,
            source: None,
        };
        assert_eq!(kind(&entry), Kind::Files);
    }

    #[test]
    fn filters_round_trip_through_their_ids() {
        for filter in Kind::FILTERS {
            assert_eq!(Filter::from_id(filter.id()), Some(filter));
        }
        assert_eq!(Filter::from_id(""), None);
    }
}
