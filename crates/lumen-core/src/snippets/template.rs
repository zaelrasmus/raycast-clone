//! Dynamic placeholders in a snippet's text (F11, slice 3).
//!
//! Raycast's syntax, as its manual writes it, so a snippet moves between the
//! two unchanged: `{clipboard}`, `{date offset="+1d" format="yyyy-MM-dd"}`,
//! `{snippet name="Signature"}`, each with `| modifiers` after it.
//!
//! # What is not a placeholder is text
//!
//! Anything in braces that is not a valid placeholder -- an unknown name, an
//! unknown parameter, a bad offset -- is left exactly as written. Snippets are
//! often code, and `function() { return x; }` or a JSON object must paste as
//! written; a typo is visible in what is pasted, which is where it is found.
//! Raycast does the same.
//!
//! # Parsed once, filled in when pasted
//!
//! Parsing is pure and happens on the text alone. What a placeholder becomes
//! -- the clipboard, the time, another snippet -- is asked of [`Sources`] when
//! the snippet is pasted, so the rules here are tested without a clipboard or
//! a clock.

use jiff::{Span, Zoned};
use lumen_platform::{DateStyle, LocalTime};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use crate::providers::calculator;

/// Everything but RFC 3986's unreserved characters, as quicklinks encode.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// What placeholders are filled in from.
pub trait Sources {
    /// The clipboard's text now (`offset` 0), or the history's entry
    /// `offset` places back.
    fn clipboard(&self, offset: usize) -> Option<String>;
    /// The text of the snippet called `name`, compared without case.
    fn snippet(&self, name: &str) -> Option<String>;
    /// `when` as the system writes it, in `locale` or the user's own.
    fn system_date(
        &self,
        when: &LocalTime,
        style: DateStyle,
        locale: Option<&str>,
    ) -> Option<String>;
    fn uuid(&self) -> String;
}

/// A snippet's text, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    Placeholder(Placeholder),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Placeholder {
    kind: Kind,
    modifiers: Vec<Modifier>,
    /// As written, braces included: what is pasted when it cannot be filled.
    source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Clipboard { offset: usize },
    Date(DateSpec),
    Uuid,
    Snippet { name: String },
    Calculator { expression: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DateSpec {
    style: DateStyle,
    offsets: Vec<Offset>,
    render: Render,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Render {
    /// The system's way, in its own locale or a named one.
    System { locale: Option<String> },
    /// A Unicode pattern: `yyyy-MM-dd`.
    Pattern(Vec<Field>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Offset {
    amount: i64,
    unit: Unit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Minutes,
    Hours,
    Days,
    Months,
    Years,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Modifier {
    Uppercase,
    Lowercase,
    Trim,
    PercentEncode,
    JsonStringify,
    /// Opts out of formatting a placeholder is given by default. A snippet
    /// gives none, so here it changes nothing; accepted so a placeholder
    /// written for a quicklink is still valid in a snippet.
    Raw,
    /// Evaluate the value as arithmetic, with the launcher's calculator.
    Calculate,
}

/// One piece of a date pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Field {
    Literal(String),
    /// A pattern letter and how many times it is repeated: `MMM` is `('M', 3)`.
    Letter(char, usize),
}

/// The letters a pattern may use. Any other letter makes it invalid, as it
/// would be to Raycast, rather than being written out as itself.
const PATTERN_LETTERS: &str = "yMLdEahHkKmsSZ";

impl Template {
    pub fn parse(text: &str) -> Self {
        let mut parts = Vec::new();
        let mut plain = String::new();
        let mut rest = text;

        while let Some(open) = rest.find('{') {
            plain.push_str(&rest[..open]);
            let after = &rest[open..];
            match placeholder_end(after).and_then(|end| {
                let source = &after[..end];
                placeholder(source).map(|placeholder| (placeholder, end))
            }) {
                Some((placeholder, end)) => {
                    if !plain.is_empty() {
                        parts.push(Part::Text(std::mem::take(&mut plain)));
                    }
                    parts.push(Part::Placeholder(placeholder));
                    rest = &after[end..];
                }
                None => {
                    plain.push('{');
                    rest = &after[1..];
                }
            }
        }
        plain.push_str(rest);
        if !plain.is_empty() {
            parts.push(Part::Text(plain));
        }
        Self { parts }
    }

    /// Whether anything in it is a placeholder: a snippet without one is
    /// pasted as it is, with nothing asked of anyone.
    pub fn has_placeholders(&self) -> bool {
        self.parts
            .iter()
            .any(|part| matches!(part, Part::Placeholder(_)))
    }

    fn refers_to_snippets(&self) -> bool {
        self.parts.iter().any(|part| {
            matches!(
                part,
                Part::Placeholder(Placeholder {
                    kind: Kind::Snippet { .. },
                    ..
                })
            )
        })
    }

    /// The text with every placeholder filled in, `now` being the moment of
    /// the paste in the user's time zone.
    pub fn expand(&self, now: &Zoned, sources: &dyn Sources) -> String {
        self.expand_at(now, sources, true)
    }

    fn expand_at(&self, now: &Zoned, sources: &dyn Sources, snippets: bool) -> String {
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Text(text) => out.push_str(text),
                Part::Placeholder(placeholder) => match placeholder.value(now, sources, snippets) {
                    Some(value) => out.push_str(&placeholder.modify(value)),
                    None => out.push_str(&placeholder.source),
                },
            }
        }
        out
    }
}

impl Placeholder {
    /// What the placeholder stands for, before modifiers; `None` leaves it
    /// as written.
    fn value(&self, now: &Zoned, sources: &dyn Sources, snippets: bool) -> Option<String> {
        match &self.kind {
            // Nothing copied is nothing inserted, as Raycast removes it.
            Kind::Clipboard { offset } => Some(sources.clipboard(*offset).unwrap_or_default()),
            Kind::Uuid => Some(sources.uuid()),
            Kind::Calculator { expression } => {
                calculator::amount(expression).map(calculator::format_number)
            }
            Kind::Date(spec) => spec.write(now, sources),
            // One level only: a snippet that refers to others cannot be
            // inserted, which is Raycast's rule and what keeps two snippets
            // that name each other from never ending.
            Kind::Snippet { name } => {
                if !snippets {
                    return None;
                }
                let inner = Template::parse(&sources.snippet(name)?);
                if inner.refers_to_snippets() {
                    return None;
                }
                Some(inner.expand_at(now, sources, false))
            }
        }
    }

    fn modify(&self, mut value: String) -> String {
        for modifier in &self.modifiers {
            value = match modifier {
                Modifier::Uppercase => value.to_uppercase(),
                Modifier::Lowercase => value.to_lowercase(),
                Modifier::Trim => value.trim().to_string(),
                Modifier::PercentEncode => utf8_percent_encode(&value, COMPONENT).to_string(),
                Modifier::JsonStringify => serde_json::to_string(&value).unwrap_or(value),
                Modifier::Raw => value,
                // What is not arithmetic is left as it was: the clipboard
                // holding a word is not a reason to paste nothing.
                Modifier::Calculate => calculator::amount(&value)
                    .map(calculator::format_number)
                    .unwrap_or(value),
            };
        }
        value
    }
}

impl DateSpec {
    fn write(&self, now: &Zoned, sources: &dyn Sources) -> Option<String> {
        let mut when = now.clone();
        for offset in &self.offsets {
            let span = match offset.unit {
                Unit::Minutes => Span::new().try_minutes(offset.amount).ok()?,
                Unit::Hours => Span::new().try_hours(offset.amount).ok()?,
                Unit::Days => Span::new().try_days(offset.amount).ok()?,
                Unit::Months => Span::new().try_months(offset.amount).ok()?,
                Unit::Years => Span::new().try_years(offset.amount).ok()?,
            };
            when = when.checked_add(span).ok()?;
        }
        match &self.render {
            Render::System { locale } => {
                sources.system_date(&local_time(&when), self.style, locale.as_deref())
            }
            Render::Pattern(fields) => Some(write_pattern(fields, &when)),
        }
    }
}

fn local_time(when: &Zoned) -> LocalTime {
    LocalTime {
        year: when.year(),
        month: when.month() as u8,
        day: when.day() as u8,
        weekday: when.weekday().to_sunday_zero_offset() as u8,
        hour: when.hour() as u8,
        minute: when.minute() as u8,
        second: when.second() as u8,
    }
}

/// Where the placeholder starting at `text[0] == '{'` ends, just past its
/// `}`. A quoted value may hold a brace.
fn placeholder_end(text: &str) -> Option<usize> {
    let mut quoted = false;
    for (index, character) in text.char_indices().skip(1) {
        match character {
            '"' => quoted = !quoted,
            '{' if !quoted => return None,
            '}' if !quoted => return Some(index + 1),
            '\n' => return None,
            _ => {}
        }
    }
    None
}

/// Read `{name key=value | modifier}`, braces included.
fn placeholder(source: &str) -> Option<Placeholder> {
    let inner = &source[1..source.len() - 1];
    let mut pipes = split_unquoted(inner, '|').into_iter();
    let head = pipes.next()?.trim();
    let modifiers = pipes
        .map(|modifier| match modifier.trim() {
            "uppercase" => Some(Modifier::Uppercase),
            "lowercase" => Some(Modifier::Lowercase),
            "trim" => Some(Modifier::Trim),
            "percent-encode" => Some(Modifier::PercentEncode),
            "json-stringify" => Some(Modifier::JsonStringify),
            "raw" => Some(Modifier::Raw),
            "calculate" => Some(Modifier::Calculate),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;

    let (name, rest) = match head.find(char::is_whitespace) {
        Some(space) => (&head[..space], &head[space..]),
        None => (head, ""),
    };
    let parameters = parameters(rest)?;
    let get = |key: &str| {
        parameters
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    };
    let only = |allowed: &[&str]| {
        parameters
            .iter()
            .all(|(key, _)| allowed.contains(&key.as_str()))
    };

    let kind = match name {
        "clipboard" if only(&["offset"]) => Kind::Clipboard {
            offset: match get("offset") {
                Some(offset) => offset.parse().ok()?,
                None => 0,
            },
        },
        "uuid" if parameters.is_empty() => Kind::Uuid,
        "snippet" if only(&["name"]) => Kind::Snippet {
            name: get("name")
                .filter(|name| !name.trim().is_empty())?
                .to_string(),
        },
        "calculator" if only(&["expression"]) => Kind::Calculator {
            expression: get("expression")?.to_string(),
        },
        "date" | "time" | "datetime" | "day" if only(&["offset", "format", "locale"]) => {
            let style = match name {
                "date" => DateStyle::Date,
                "time" => DateStyle::Time,
                "datetime" => DateStyle::DateTime,
                _ => DateStyle::Weekday,
            };
            let render = match (get("format"), get("locale")) {
                // A pattern says everything a locale would: Raycast refuses
                // the two together, and so does this.
                (Some(_), Some(_)) => return None,
                (Some(format), None) => Render::Pattern(pattern(format)?),
                (None, locale) => Render::System {
                    locale: locale.map(str::to_string),
                },
            };
            Kind::Date(DateSpec {
                style,
                offsets: match get("offset") {
                    Some(offset) => offsets(offset)?,
                    None => Vec::new(),
                },
                render,
            })
        }
        _ => return None,
    };

    Some(Placeholder {
        kind,
        modifiers,
        source: source.to_string(),
    })
}

/// Split on `separator` outside double quotes.
fn split_unquoted(text: &str, separator: char) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (index, character) in text.char_indices() {
        if character == '"' {
            quoted = !quoted;
        } else if character == separator && !quoted {
            pieces.push(&text[start..index]);
            start = index + character.len_utf8();
        }
    }
    pieces.push(&text[start..]);
    pieces
}

/// `key=value key="a value"`, each key once.
fn parameters(text: &str) -> Option<Vec<(String, String)>> {
    let mut parameters: Vec<(String, String)> = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        let equals = rest.find('=')?;
        let key = &rest[..equals];
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return None;
        }
        let after = &rest[equals + 1..];
        let (value, remaining) = if let Some(quoted) = after.strip_prefix('"') {
            let close = quoted.find('"')?;
            (&quoted[..close], &quoted[close + 1..])
        } else {
            let end = after.find(char::is_whitespace).unwrap_or(after.len());
            (&after[..end], &after[end..])
        };
        // After a value comes a space or the end, never more of a word.
        if !remaining.is_empty() && !remaining.starts_with(char::is_whitespace) {
            return None;
        }
        if parameters.iter().any(|(existing, _)| existing == key) {
            return None;
        }
        parameters.push((key.to_string(), value.to_string()));
        rest = remaining.trim_start();
    }
    Some(parameters)
}

/// `"+2y +5M -3d"`: a sign, a number and a unit, with no space between.
fn offsets(text: &str) -> Option<Vec<Offset>> {
    let offsets = text
        .split_whitespace()
        .map(|word| {
            let negative = match word.chars().next()? {
                '+' => false,
                '-' => true,
                _ => return None,
            };
            let unit = match word.chars().last()? {
                'm' => Unit::Minutes,
                'h' => Unit::Hours,
                'd' => Unit::Days,
                'M' => Unit::Months,
                'y' => Unit::Years,
                _ => return None,
            };
            let digits = &word[1..word.len() - 1];
            if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let amount: i64 = digits.parse().ok()?;
            Some(Offset {
                amount: if negative { -amount } else { amount },
                unit,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    (!offsets.is_empty()).then_some(offsets)
}

/// A Unicode date pattern: runs of one letter are fields, `'text'` is text,
/// `''` is a quote, and anything else that is not a letter is itself.
fn pattern(text: &str) -> Option<Vec<Field>> {
    let mut fields = Vec::new();
    let mut chars = text.chars().peekable();
    let mut literal = String::new();

    while let Some(character) = chars.next() {
        if character == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
                literal.push('\'');
                continue;
            }
            loop {
                match chars.next()? {
                    '\'' if chars.peek() == Some(&'\'') => {
                        chars.next();
                        literal.push('\'');
                    }
                    '\'' => break,
                    other => literal.push(other),
                }
            }
        } else if character.is_ascii_alphabetic() {
            if !PATTERN_LETTERS.contains(character) {
                return None;
            }
            let mut count = 1;
            while chars.peek() == Some(&character) {
                chars.next();
                count += 1;
            }
            if !literal.is_empty() {
                fields.push(Field::Literal(std::mem::take(&mut literal)));
            }
            fields.push(Field::Letter(character, count));
        } else {
            literal.push(character);
        }
    }
    if !literal.is_empty() {
        fields.push(Field::Literal(literal));
    }
    Some(fields)
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Sunday first, as [`jiff::civil::Weekday::to_sunday_zero_offset`] counts.
const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// Write `when` by a pattern. Names are English: a pattern and a locale
/// cannot be given together, and a pattern is chosen to look one exact way.
fn write_pattern(fields: &[Field], when: &Zoned) -> String {
    let mut out = String::new();
    for field in fields {
        match field {
            Field::Literal(text) => out.push_str(text),
            Field::Letter(letter, count) => out.push_str(&write_field(*letter, *count, when)),
        }
    }
    out
}

fn write_field(letter: char, count: usize, when: &Zoned) -> String {
    let padded = |value: i64, width: usize| format!("{value:0width$}");
    match letter {
        'y' if count == 2 => padded(i64::from(when.year()).rem_euclid(100), 2),
        'y' => padded(i64::from(when.year()), count),
        'M' | 'L' => {
            let month = when.month() as usize;
            match count {
                1 | 2 => padded(month as i64, count),
                3 => MONTHS[month - 1][..3].to_string(),
                4 => MONTHS[month - 1].to_string(),
                _ => MONTHS[month - 1][..1].to_string(),
            }
        }
        'd' => padded(i64::from(when.day()), count.min(2)),
        'E' => {
            let day = DAYS[when.weekday().to_sunday_zero_offset() as usize];
            match count {
                1..=3 => day[..3].to_string(),
                4 => day.to_string(),
                _ => day[..1].to_string(),
            }
        }
        'a' => (if when.hour() < 12 { "AM" } else { "PM" }).to_string(),
        'h' => {
            let hour = i64::from(when.hour()) % 12;
            padded(if hour == 0 { 12 } else { hour }, count.min(2))
        }
        'H' => padded(i64::from(when.hour()), count.min(2)),
        'K' => padded(i64::from(when.hour()) % 12, count.min(2)),
        'k' => {
            let hour = i64::from(when.hour());
            padded(if hour == 0 { 24 } else { hour }, count.min(2))
        }
        'm' => padded(i64::from(when.minute()), count.min(2)),
        's' => padded(i64::from(when.second()), count.min(2)),
        'S' => {
            let nanos = format!("{:09}", when.subsec_nanosecond());
            nanos[..count.min(9)].to_string()
        }
        'Z' => {
            let seconds = when.offset().seconds();
            let sign = if seconds < 0 { '-' } else { '+' };
            let minutes = seconds.abs() / 60;
            if count >= 5 {
                format!("{sign}{:02}:{:02}", minutes / 60, minutes % 60)
            } else {
                format!("{sign}{:02}{:02}", minutes / 60, minutes % 60)
            }
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Fake {
        clipboard: Vec<&'static str>,
        snippets: HashMap<&'static str, &'static str>,
    }

    impl Default for Fake {
        fn default() -> Self {
            Self {
                clipboard: vec!["copied", "before"],
                snippets: HashMap::from([
                    ("signature", "Best,\nAna"),
                    ("dated", "on {date format=\"yyyy-MM-dd\"}"),
                    ("nested", "see {snippet name=\"signature\"}"),
                    ("loop", "{snippet name=\"loop\"}"),
                ]),
            }
        }
    }

    impl Sources for Fake {
        fn clipboard(&self, offset: usize) -> Option<String> {
            self.clipboard.get(offset).map(|text| text.to_string())
        }
        fn snippet(&self, name: &str) -> Option<String> {
            self.snippets
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, text)| text.to_string())
        }
        fn system_date(
            &self,
            when: &LocalTime,
            style: DateStyle,
            locale: Option<&str>,
        ) -> Option<String> {
            if locale == Some("xx") {
                return None;
            }
            Some(format!(
                "{style:?}/{}/{}-{}-{} {}:{:02}",
                locale.unwrap_or("user"),
                when.year,
                when.month,
                when.day,
                when.hour,
                when.minute
            ))
        }
        fn uuid(&self) -> String {
            "E621E1F8-C36C-495A-93FC-0C247A3E6E5F".into()
        }
    }

    /// Wednesday 30 September 2026, 21:05:09.945 in New York.
    fn now() -> Zoned {
        "2026-09-30T21:05:09.945-04:00[America/New_York]"
            .parse()
            .unwrap()
    }

    fn expand(text: &str) -> String {
        Template::parse(text).expand(&now(), &Fake::default())
    }

    #[test]
    fn text_without_placeholders_is_itself() {
        let template = Template::parse("Best,\nAna");
        assert!(!template.has_placeholders());
        assert_eq!(expand("Best,\nAna"), "Best,\nAna");
        assert_eq!(expand(""), "");
    }

    #[test]
    fn code_and_unknown_placeholders_are_pasted_as_written() {
        for text in [
            "function() { return x; }",
            r#"{ "a": 1 }"#,
            "{}",
            "{ clipboard",
            "{nope}",
            "{clipboard offset=x}",
            "{clipboard | shout}",
            "{uuid version=4}",
            "{date offset=\"+ 2d\"}",
            "{date format=\"yyyy\" locale=\"fr-FR\"}",
            "{date format=\"qq\"}",
            "{snippet}",
            "{calculator}",
        ] {
            assert_eq!(expand(text), text, "{text}");
        }
    }

    #[test]
    fn the_clipboard_and_its_history() {
        assert_eq!(expand("[{clipboard}]"), "[copied]");
        assert_eq!(expand("{clipboard offset=1}"), "before");
        // Nothing that far back is nothing inserted, as Raycast removes it.
        assert_eq!(expand("[{clipboard offset=9}]"), "[]");
    }

    #[test]
    fn modifiers_apply_in_order() {
        let fake = Fake {
            clipboard: vec!["  Foo \"Bar\" "],
            ..Default::default()
        };
        let run = |text: &str| Template::parse(text).expand(&now(), &fake);

        assert_eq!(run("{clipboard | trim | uppercase}"), "FOO \"BAR\"");
        assert_eq!(run("{clipboard|trim|lowercase}"), "foo \"bar\"");
        assert_eq!(
            run("{clipboard | trim | percent-encode}"),
            "Foo%20%22Bar%22"
        );
        assert_eq!(
            run("{clipboard | trim | json-stringify}"),
            r#""Foo \"Bar\"""#
        );
        assert_eq!(run("{clipboard | raw}"), "  Foo \"Bar\" ");
    }

    #[test]
    fn the_calculator_as_a_placeholder_and_as_a_modifier() {
        assert_eq!(expand("{calculator expression=\"12*7\"}"), "84");
        assert_eq!(expand("{calculator expression=\"(1+2)/4\"}"), "0.75");
        // An expression that is not arithmetic cannot be filled in.
        assert_eq!(
            expand("{calculator expression=\"twelve\"}"),
            "{calculator expression=\"twelve\"}"
        );

        let fake = Fake {
            clipboard: vec!["19.99*3", "a word"],
            ..Default::default()
        };
        let run = |text: &str| Template::parse(text).expand(&now(), &fake);
        assert_eq!(run("{clipboard | calculate}"), "59.97");
        // A value that is not arithmetic is left as it was.
        assert_eq!(run("{clipboard offset=1 | calculate}"), "a word");
    }

    #[test]
    fn the_uuid() {
        assert_eq!(expand("{uuid}"), "E621E1F8-C36C-495A-93FC-0C247A3E6E5F");
        assert_eq!(
            expand("{uuid | lowercase}"),
            "e621e1f8-c36c-495a-93fc-0c247a3e6e5f"
        );
    }

    #[test]
    fn dates_the_system_way_in_the_users_locale_or_a_named_one() {
        assert_eq!(expand("{date}"), "Date/user/2026-9-30 21:05");
        assert_eq!(expand("{time}"), "Time/user/2026-9-30 21:05");
        assert_eq!(expand("{datetime}"), "DateTime/user/2026-9-30 21:05");
        assert_eq!(expand("{day}"), "Weekday/user/2026-9-30 21:05");
        assert_eq!(
            expand("{date locale=\"fr-FR\"}"),
            "Date/fr-FR/2026-9-30 21:05"
        );
        // A locale the system does not know leaves the placeholder.
        assert_eq!(expand("{date locale=xx}"), "{date locale=xx}");
    }

    #[test]
    fn offsets_move_the_moment_in_calendar_units() {
        assert_eq!(expand("{date offset=+1d}"), "Date/user/2026-10-1 21:05");
        assert_eq!(
            expand("{date offset=\"+2y +5M\"}"),
            "Date/user/2029-2-28 21:05"
        );
        assert_eq!(
            expand("{time offset=\"+3h +30m\"}"),
            "Time/user/2026-10-1 0:35"
        );
        assert_eq!(expand("{day offset=-3d}"), "Weekday/user/2026-9-27 21:05");
    }

    /// Raycast's own examples, at Raycast's moment moved to this one.
    #[test]
    fn patterns() {
        let cases = [
            ("EEEE, MMM d, yyyy", "Wednesday, Sep 30, 2026"),
            ("MM/dd/yyyy", "09/30/2026"),
            ("MM-dd-yyyy HH:mm", "09-30-2026 21:05"),
            ("MMM d, h:mm a", "Sep 30, 9:05 PM"),
            ("MMMM yyyy", "September 2026"),
            (
                "E, d MMM yyyy HH:mm:ss Z",
                "Wed, 30 Sep 2026 21:05:09 -0400",
            ),
            ("yyyy-MM-dd'T'HH:mm:ssZ", "2026-09-30T21:05:09-0400"),
            ("dd.MM.yy", "30.09.26"),
            ("HH:mm:ss.SSS", "21:05:09.945"),
            (
                "h:mm 'on the eve of' MMMM d",
                "9:05 on the eve of September 30",
            ),
            ("'o''clock' h", "o'clock 9"),
        ];
        for (format, expected) in cases {
            assert_eq!(
                expand(&format!("{{date format=\"{format}\"}}")),
                expected,
                "{format}"
            );
        }
        assert_eq!(
            expand("{date format=\"yyyy-MM-dd\" offset=\"+3M -5d\"}"),
            "2026-12-25"
        );
    }

    #[test]
    fn a_snippet_inside_a_snippet_one_level_deep() {
        assert_eq!(
            expand("Thanks!\n{snippet name=\"Signature\"}"),
            "Thanks!\nBest,\nAna"
        );
        // Its own placeholders are filled in too.
        assert_eq!(expand("{snippet name=dated}"), "on 2026-09-30");
        // One that refers to others cannot be inserted, which also stops a
        // snippet that names itself.
        assert_eq!(expand("{snippet name=nested}"), "{snippet name=nested}");
        assert_eq!(expand("{snippet name=loop}"), "{snippet name=loop}");
        assert_eq!(expand("{snippet name=missing}"), "{snippet name=missing}");
        assert_eq!(expand("{snippet name=signature | uppercase}"), "BEST,\nANA");
    }

    #[test]
    fn a_quoted_value_may_hold_braces_and_pipes() {
        assert_eq!(expand("{date format=\"'{'yyyy'}' | 'x'\"}"), "{2026} | x");
    }

    #[test]
    fn text_around_placeholders_is_kept_exactly() {
        assert_eq!(
            expand("a {uuid} b {nope} c {clipboard}"),
            "a E621E1F8-C36C-495A-93FC-0C247A3E6E5F b {nope} c copied"
        );
        assert_eq!(expand("{{clipboard}}"), "{copied}");
    }
}
