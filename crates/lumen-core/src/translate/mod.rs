//! Translation (F7).
//!
//! # Where the network is
//!
//! Only in [`Translator::translate`], and that is only ever called from a
//! worker thread the translate view starts. Root search offers presets as
//! catalogue rows (see [`crate::providers::translate`]) and never translates
//! anything, so I-3 is not at risk: nothing here runs while the root list is
//! being ranked.
//!
//! # What is here and what is not
//!
//! The languages, the presets read from `config.json`, the two services, and
//! [`session::Session`] -- the view's state as a plain value that turns events
//! into views and requests. Starting threads, timers and the view itself are
//! the daemon's (`lumen-app`), exactly as running an extension's process is.

pub mod deepl;
pub mod google;
pub mod languages;
pub mod session;
pub mod store;

use std::time::Duration;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub use crate::net::{Failure, failure};
pub use languages::{LANGUAGES, Language};

/// The most targets one preset may have.
///
/// Every target is a request per pause in typing: against the keyless endpoint
/// that is more requests for it to throttle, and against DeepL more characters
/// counted against the month's quota. Four is already a lot of languages to
/// read at once.
pub const MAX_TARGETS: usize = 4;

/// The most text sent in one request, in characters.
///
/// A launcher is for a sentence or a paragraph, not a document. The cap keeps
/// a paste of a whole file from being sent, four times over, on every pause.
pub const MAX_CHARS: usize = 5_000;

/// How long a request may take before it is reported as failed.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The `translate` section of `config.json`.
#[derive(Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct TranslateConfig {
    pub service: Service,
    /// DeepL's API key. Only read when `service` is `deepl`.
    pub deepl_key: Option<String>,
    /// Saved directions. Read leniently -- see [`TranslateConfig::presets`].
    pub presets: Vec<PresetConfig>,
}

/// Written by hand so that the key never reaches a log line. `Config` derives
/// `Debug` and is printed when a reload fails; a secret in the log is a secret
/// in every bug report somebody attaches it to.
impl std::fmt::Debug for TranslateConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranslateConfig")
            .field("service", &self.service)
            .field("deepl_key", &self.deepl_key.as_ref().map(|_| "<set>"))
            .field("presets", &self.presets)
            .finish()
    }
}

/// Which service translates.
///
/// Chosen, never fallen back between: text the user chose to send to DeepL
/// must not go to Google because DeepL was unreachable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Service {
    /// The keyless endpoint: works with no setup.
    #[default]
    Google,
    Deepl,
}

impl Service {
    /// The name the view shows, so the user knows who receives what they type.
    pub fn label(self) -> &'static str {
        match self {
            Service::Google => "Google Translate",
            Service::Deepl => "DeepL",
        }
    }
}

/// A preset as written in `config.json`, before it is checked.
///
/// Strings rather than [`Language`]s so that one unknown code skips one preset
/// instead of failing the whole file -- and with it the hotkey, the theme and
/// every other setting in it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct PresetConfig {
    pub name: String,
    pub alias: Option<String>,
    /// A code, a name, or `auto` to detect it. Absent means `auto`.
    pub from: Option<String>,
    /// One code or name, or a list of them.
    pub to: Targets,
}

/// `"to": "en"` and `"to": ["en", "fr"]` both mean what they say.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Targets {
    One(String),
    Many(Vec<String>),
}

impl Default for Targets {
    fn default() -> Self {
        Targets::Many(Vec::new())
    }
}

impl Targets {
    fn codes(&self) -> Vec<&str> {
        match self {
            Targets::One(code) => vec![code.as_str()],
            Targets::Many(codes) => codes.iter().map(String::as_str).collect(),
        }
    }
}

/// A saved direction: a source, or detection, and up to [`MAX_TARGETS`]
/// targets.
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub name: String,
    pub alias: Option<String>,
    /// `None` detects the source.
    pub from: Option<&'static Language>,
    pub to: Vec<&'static Language>,
}

impl Preset {
    /// What the preset does, in words: "Spanish to English and French".
    pub fn describe(&self) -> String {
        describe(self.from, &self.to)
    }

    /// The targets, as a sentence fragment: "English and French".
    pub fn targets(&self) -> String {
        names(&self.to)
    }

    /// The preset used when none are configured.
    fn implied() -> Self {
        Self {
            name: "Any Language \u{2192} English".into(),
            alias: None,
            from: None,
            to: vec![languages::find("en").expect("English is in the table")],
        }
    }
}

impl TranslateConfig {
    /// The presets that make sense, in the order they were written.
    ///
    /// Lenient like the rest of the file (I-12's reasoning, applied to user
    /// data): a preset with no name, an unknown language or no targets is
    /// skipped with a log line, and the others still load. Targets past
    /// [`MAX_TARGETS`] are dropped. With nothing usable, one preset is implied
    /// so the command works on first use.
    pub fn presets(&self) -> Vec<Preset> {
        let mut presets: Vec<Preset> = Vec::new();

        for written in &self.presets {
            let mut preset = match resolve(written) {
                Ok(preset) => preset,
                Err(problem) => {
                    log::warn!("skipping a translate preset: {problem}");
                    continue;
                }
            };
            if presets.iter().any(|other| other.name == preset.name) {
                log::warn!("skipping a second translate preset named {:?}", preset.name);
                continue;
            }
            if preset.to.len() > MAX_TARGETS {
                log::warn!(
                    "the translate preset {:?} has {} targets; using the first {MAX_TARGETS}",
                    preset.name,
                    preset.to.len()
                );
                preset.to.truncate(MAX_TARGETS);
            }
            presets.push(preset);
        }

        if presets.is_empty() {
            presets.push(Preset::implied());
        }
        presets
    }
}

/// A direction in words: "Spanish to English and French".
pub fn describe(from: Option<&Language>, to: &[&Language]) -> String {
    let from = from.map_or("any language", |language| language.name);
    format!("{from} to {}", names(to))
}

/// Languages as a sentence fragment: "English, French and German".
pub fn names(languages: &[&Language]) -> String {
    let names: Vec<&str> = languages.iter().map(|language| language.name).collect();
    match names.as_slice() {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// One preset as written, checked on its own: a name, known languages, at
/// least one target. What the loader skips and the settings page refuses are
/// the same thing, so they are decided here once.
///
/// Not checked here, because they need the other presets or a policy: a name
/// repeated elsewhere, and more than [`MAX_TARGETS`] targets.
pub fn resolve(written: &PresetConfig) -> std::result::Result<Preset, String> {
    let name = written.name.trim();
    if name.is_empty() {
        return Err("a preset needs a name".into());
    }

    let from = match written.from.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(code) if code.eq_ignore_ascii_case("auto") => None,
        Some(code) => Some(
            languages::find(code)
                .ok_or_else(|| format!("{name:?} names an unknown language, {code:?}"))?,
        ),
    };

    let mut to = Vec::new();
    for code in written.to.codes() {
        let language = languages::find(code)
            .ok_or_else(|| format!("{name:?} names an unknown language, {code:?}"))?;
        if !to.contains(&language) {
            to.push(language);
        }
    }
    if to.is_empty() {
        return Err(format!("{name:?} has no language to translate into"));
    }

    Ok(Preset {
        name: name.to_string(),
        alias: written
            .alias
            .as_deref()
            .map(str::trim)
            .filter(|alias| !alias.is_empty())
            .map(str::to_string),
        from,
        to,
    })
}

/// What a service answered.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Translation {
    pub text: String,
    /// What the service thinks the source was, when it says and it is a
    /// language in the table.
    pub detected: Option<&'static Language>,
    /// A single word's other translations, by part of speech (F10). Only
    /// Google gives them, and only for one word; empty otherwise.
    pub alternatives: Vec<Alternative>,
}

/// Other translations of a word as one part of speech: *verb: mostrar,
/// exhibir, desplegar*.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Alternative {
    /// As the service names it, lower case: "verb".
    pub part: String,
    /// Most common first, at most [`MAX_ALTERNATIVES`].
    pub terms: Vec<String>,
}

/// The most terms kept per part of speech. Google lists thirty for a common
/// verb; past the first few they are rare senses nobody is looking for.
pub const MAX_ALTERNATIVES: usize = 8;

/// Whether `text` is one word, and so worth asking for alternatives.
pub fn is_one_word(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty() && !text.contains(char::is_whitespace)
}

/// A translation service.
///
/// Blocking, on purpose: it is only ever called from a worker thread, and the
/// blocking client is the one already in the binary (F6, step 3).
pub trait Translator: Send + Sync {
    fn translate(
        &self,
        text: &str,
        from: Option<&'static Language>,
        to: &'static Language,
    ) -> Result<Translation>;
}

/// The service `config` names, with a client ready to use.
///
/// Building the client reads the system's certificate store, which is why the
/// caller keeps it for a while rather than building one per request.
pub fn translator(config: &TranslateConfig) -> Result<Box<dyn Translator>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("Lumen/", env!("CARGO_PKG_VERSION")))
        .build()?;

    Ok(match config.service {
        Service::Google => Box::new(google::Google::new(client)),
        Service::Deepl => {
            let Some(key) = config
                .deepl_key
                .as_deref()
                .map(str::trim)
                .filter(|key| !key.is_empty())
            else {
                bail!("DeepL needs an API key: set translate.deepl-key in config.json");
            };
            Box::new(deepl::DeepL::new(client, key.to_string()))
        }
    })
}

/// `text`, cut to [`MAX_CHARS`] characters at a character boundary.
pub fn clip(text: &str) -> &str {
    match text.char_indices().nth(MAX_CHARS) {
        Some((end, _)) => &text[..end],
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(source: &str) -> TranslateConfig {
        serde_json::from_str(source).unwrap()
    }

    #[test]
    fn nothing_configured_implies_one_preset_into_english() {
        let presets = TranslateConfig::default().presets();
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].from, None);
        assert_eq!(presets[0].to[0].code, "en");
    }

    #[test]
    fn a_preset_takes_one_target_or_several() {
        let presets = config(
            r#"{ "presets": [
                { "name": "Spanish → English", "from": "es", "to": "en" },
                { "name": "Spanish → Many", "from": "Spanish", "to": ["en", "FR", "pt"] }
            ] }"#,
        )
        .presets();

        assert_eq!(presets.len(), 2);
        assert_eq!(presets[0].to.len(), 1);
        assert_eq!(presets[1].from.unwrap().code, "es");
        assert_eq!(presets[1].targets(), "English, French and Portuguese");
    }

    /// One mistake skips one preset, never the file.
    #[test]
    fn a_preset_naming_an_unknown_language_is_skipped_alone() {
        let presets = config(
            r#"{ "presets": [
                { "name": "Broken", "from": "xx", "to": "en" },
                { "name": "Also broken", "to": ["en", "klingon"] },
                { "name": "", "to": "en" },
                { "name": "Empty", "to": [] },
                { "name": "Fine", "from": "auto", "to": "de" }
            ] }"#,
        )
        .presets();

        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].name, "Fine");
        assert_eq!(presets[0].from, None);
    }

    #[test]
    fn targets_past_the_cap_are_dropped_and_repeats_ignored() {
        let presets = config(
            r#"{ "presets": [
                { "name": "Lots", "to": ["en", "en", "fr", "de", "it", "pt", "nl"] }
            ] }"#,
        )
        .presets();

        let codes: Vec<&str> = presets[0].to.iter().map(|language| language.code).collect();
        assert_eq!(codes, ["en", "fr", "de", "it"]);
    }

    #[test]
    fn a_second_preset_with_the_same_name_is_skipped() {
        let presets = config(
            r#"{ "presets": [
                { "name": "Same", "to": "en" },
                { "name": "Same", "to": "fr" }
            ] }"#,
        )
        .presets();

        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].to[0].code, "en");
    }

    #[test]
    fn the_key_never_appears_in_debug_output() {
        let config = config(r#"{ "service": "deepl", "deepl-key": "secret-123:fx" }"#);
        let printed = format!("{config:?}");
        assert!(!printed.contains("secret-123"));
        assert!(printed.contains("<set>"));
    }

    #[test]
    fn deepl_without_a_key_is_refused_rather_than_sent_to_google() {
        let error = translator(&config(r#"{ "service": "deepl" }"#))
            .err()
            .expect("no key must be an error");
        assert!(error.to_string().contains("deepl-key"));
    }

    #[test]
    fn clipping_respects_character_boundaries() {
        let long = "é".repeat(MAX_CHARS + 10);
        assert_eq!(clip(&long).chars().count(), MAX_CHARS);
        assert_eq!(clip("short"), "short");
    }
}
