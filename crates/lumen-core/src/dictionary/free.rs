//! FreeDictionaryAPI: English Wiktionary, keyless.
//!
//! `freedictionaryapi.com/api/v1/entries/{language}/{word}` answers with
//! Wiktionary's data as JSON: senses by part of speech, examples, IPA and
//! inflected forms, for words of 279 languages explained in English. No key;
//! 1,000 requests an hour per address, a `429` past that. The data is CC BY-SA
//! 4.0, and the service asks for a link back to the Wiktionary page and a
//! visible credit to itself, which the view gives in every pane (F10).
//!
//! Two things measured while building this shape the code:
//!
//! - **Lookups are case-sensitive.** `Display` has no entry; `display` does,
//!   and a German noun is only found with its capital. So the word is asked
//!   for as typed, and in lower case only when that finds nothing.
//! - **A word the service does not have is not an error.** It is a `200`
//!   with no entries, and is reported as `None`.

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::{Dictionary, Entry, Form, Part, Sense};

const ENDPOINT: &str = "https://freedictionaryapi.com/api/v1/entries";

/// The credit the service's terms ask for, shown in every pane.
pub const ATTRIBUTION: &str = "Wiktionary, via FreeDictionaryAPI.com \u{00b7} CC BY-SA 4.0";

/// The most forms kept per part of speech. A Spanish verb has two hundred,
/// most of them a conjugation table; the first few are its principal parts.
const MAX_FORMS: usize = 8;

/// Form tags that mark the source's own bookkeeping, not a form of the word.
const NOT_FORMS: &[&str] = &["table-tags", "inflection-template", "class"];

pub struct FreeDictionary {
    client: reqwest::blocking::Client,
}

impl FreeDictionary {
    pub fn new(client: reqwest::blocking::Client) -> Self {
        Self { client }
    }

    fn fetch(&self, word: &str, language: &str) -> Result<Option<Entry>> {
        let url = url(word, language)?;
        // Without the URL: it carries the word, and errors reach the log.
        let response = self
            .client
            .get(url)
            .send()
            .map_err(reqwest::Error::without_url)
            .context("reaching FreeDictionaryAPI")?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            bail!(
                "FreeDictionaryAPI's limit of 1,000 lookups an hour is reached; it resets on the hour"
            );
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            bail!("FreeDictionaryAPI answered {status}");
        }
        let body = response
            .text()
            .map_err(reqwest::Error::without_url)
            .context("reading FreeDictionaryAPI's answer")?;
        parse(&body, language)
    }
}

impl Dictionary for FreeDictionary {
    fn define(&self, word: &str, language: &str, explained_in: &str) -> Result<Option<Entry>> {
        // English Wiktionary writes in English and nothing else. Answering in
        // English when another language was asked for would be a silent
        // fallback, which F7 rules out.
        if explained_in != "en" {
            bail!("FreeDictionaryAPI explains words in English only");
        }
        if let Some(entry) = self.fetch(word, language)? {
            return Ok(Some(entry));
        }
        let lower = word.to_lowercase();
        if lower != word {
            return self.fetch(&lower, language);
        }
        Ok(None)
    }
}

fn url(word: &str, language: &str) -> Result<url::Url> {
    let mut url = url::Url::parse(ENDPOINT)?;
    // Segments, so a space or a slash in a phrase is escaped, not a path.
    url.path_segments_mut()
        .map_err(|()| anyhow::anyhow!("the endpoint cannot take a path"))?
        .push(language)
        .push(word);
    Ok(url)
}

#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    word: String,
    #[serde(default)]
    entries: Vec<ApiEntry>,
    #[serde(default)]
    source: Option<Source>,
}

#[derive(Deserialize)]
struct Source {
    #[serde(default)]
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiEntry {
    #[serde(default)]
    language: Option<ApiLanguage>,
    #[serde(default)]
    part_of_speech: String,
    #[serde(default)]
    pronunciations: Vec<Pronunciation>,
    #[serde(default)]
    forms: Vec<Form>,
    #[serde(default)]
    senses: Vec<ApiSense>,
}

#[derive(Deserialize)]
struct ApiLanguage {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct Pronunciation {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct ApiSense {
    #[serde(default)]
    definition: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    examples: Vec<String>,
    #[serde(default)]
    synonyms: Vec<String>,
    #[serde(default)]
    subsenses: Vec<ApiSense>,
}

impl From<ApiSense> for Sense {
    fn from(sense: ApiSense) -> Self {
        Sense {
            definition: sense.definition.trim().to_string(),
            tags: sense.tags,
            examples: sense.examples,
            synonyms: sense.synonyms,
            subsenses: sense.subsenses.into_iter().map(Sense::from).collect(),
        }
    }
}

/// Read an answer. Entries of the same part of speech -- Wiktionary splits
/// them by etymology, so `run` has two "verb" entries -- become one part,
/// in the order the first appeared.
pub fn parse(body: &str, language: &str) -> Result<Option<Entry>> {
    // An object, checked first: serde would read a struct from an array too,
    // and a broken answer read as "no entry" tells the user the word does
    // not exist.
    let value: serde_json::Value =
        serde_json::from_str(body).context("FreeDictionaryAPI's answer is not JSON")?;
    if !value.is_object() {
        bail!("FreeDictionaryAPI's answer is not what it should be");
    }
    let answer: Answer = serde_json::from_value(value)
        .context("FreeDictionaryAPI's answer is not what it should be")?;

    let mut parts: Vec<Part> = Vec::new();
    let mut language_name = None;
    for entry in answer.entries {
        if language_name.is_none() {
            language_name = entry.language.map(|language| language.name);
        }
        let name = entry.part_of_speech.trim().to_lowercase();
        let index = match parts.iter().position(|part| part.name == name) {
            Some(index) => index,
            None => {
                parts.push(Part {
                    name,
                    ..Default::default()
                });
                parts.len() - 1
            }
        };
        let part = &mut parts[index];

        for pronunciation in entry.pronunciations {
            let text = pronunciation.text.trim();
            if pronunciation.kind == "ipa"
                && !text.is_empty()
                && !part.pronunciations.iter().any(|known| known == text)
            {
                part.pronunciations.push(text.to_string());
            }
        }
        for form in entry.forms {
            let bookkeeping = form
                .tags
                .iter()
                .any(|tag| NOT_FORMS.contains(&tag.as_str()));
            if part.forms.len() < MAX_FORMS
                && !bookkeeping
                && form.word != answer.word
                && !part.forms.iter().any(|known| known.word == form.word)
            {
                part.forms.push(form);
            }
        }
        part.senses.extend(
            entry
                .senses
                .into_iter()
                .map(Sense::from)
                .filter(|sense| !sense.definition.is_empty()),
        );
    }
    parts.retain(|part| !part.senses.is_empty());
    if parts.is_empty() {
        return Ok(None);
    }

    // The page, at the word language's section: Wiktionary puts every
    // language on one page, English first.
    let mut url = answer.source.map(|source| source.url).unwrap_or_default();
    if let Some(name) = language_name.filter(|_| !url.is_empty()) {
        url = format!("{url}#{}", name.replace(' ', "_"));
    }

    Ok(Some(Entry {
        word: answer.word,
        language: language.to_string(),
        explained_in: "en".into(),
        url,
        parts,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Both trimmed from the live service: fewer senses, no quotations.
    const DISPLAY: &str = include_str!("fixtures/display.json");
    const MOSTRAR: &str = include_str!("fixtures/mostrar.json");

    #[test]
    fn an_english_word_has_its_parts_senses_and_pronunciation() {
        let entry = parse(DISPLAY, "en").unwrap().unwrap();
        assert_eq!(entry.word, "display");
        assert_eq!(entry.explained_in, "en");
        let parts: Vec<&str> = entry.parts.iter().map(|part| part.name.as_str()).collect();
        assert_eq!(parts, ["noun", "verb"]);

        let noun = &entry.parts[0];
        assert_eq!(noun.senses[0].definition, "A show or spectacle.");
        assert!(noun.senses[0].examples[0].contains("trapeze"));
        assert_eq!(noun.pronunciations[0], "/dɪsˈpleɪ/");
        // Each pronunciation once, though both entries list it.
        assert_eq!(noun.pronunciations.len(), 2);
        assert_eq!(noun.forms[0].word, "displays");
        assert_eq!(entry.url, "https://en.wiktionary.org/wiki/display#English");
    }

    /// A Spanish word, explained in English -- the case F10 is for.
    #[test]
    fn a_spanish_word_is_explained_in_english() {
        let entry = parse(MOSTRAR, "es").unwrap().unwrap();
        assert_eq!(entry.language, "es");
        let verb = &entry.parts[0];
        assert!(verb.senses[0].definition.contains("to show"));
        assert_eq!(verb.pronunciations[0], "/mosˈtɾaɾ/");
        assert_eq!(entry.url, "https://en.wiktionary.org/wiki/mostrar#Spanish");
    }

    /// The source's bookkeeping is not a form, and a conjugation table is
    /// cut to its first few.
    #[test]
    fn forms_are_filtered_and_bounded() {
        let entry = parse(MOSTRAR, "es").unwrap().unwrap();
        let forms: Vec<&str> = entry.parts[0]
            .forms
            .iter()
            .map(|form| form.word.as_str())
            .collect();
        assert!(forms.len() <= MAX_FORMS);
        assert_eq!(&forms[..3], ["muestro", "mostré", "mostrado"]);
        assert!(!forms.contains(&"no-table-tags"));
        assert!(!forms.contains(&"es-conj"));
        // The headword is not one of its own forms.
        assert!(!forms.contains(&"mostrar"));
    }

    /// Wiktionary splits a part of speech by etymology; the view does not.
    #[test]
    fn entries_of_one_part_of_speech_are_merged() {
        let body = r#"{ "word": "run", "entries": [
            { "partOfSpeech": "verb", "senses": [{ "definition": "To move fast." }] },
            { "partOfSpeech": "noun", "senses": [{ "definition": "An act of running." }] },
            { "partOfSpeech": "verb", "senses": [{ "definition": "To melt." }] }
        ] }"#;
        let entry = parse(body, "en").unwrap().unwrap();
        assert_eq!(entry.parts.len(), 2);
        assert_eq!(entry.parts[0].senses.len(), 2);
        assert_eq!(entry.parts[0].senses[1].definition, "To melt.");
    }

    #[test]
    fn a_word_the_source_does_not_have_is_none() {
        let body =
            r#"{"word":"qwzzxvbq","entries":[],"source":{"url":"https://en.wiktionary.org"}}"#;
        assert_eq!(parse(body, "en").unwrap(), None);
    }

    #[test]
    fn an_answer_of_another_shape_is_an_error() {
        assert!(parse("<html>429</html>", "en").is_err());
        assert!(parse("[]", "en").is_err());
    }

    #[test]
    fn the_word_is_a_path_segment_escaped() {
        let url = url("display case/x", "en").unwrap();
        assert_eq!(url.path(), "/api/v1/entries/en/display%20case%2Fx");
    }

    /// Against the real service. Ignored because CI must not depend on a third
    /// party; run with `cargo test -- --ignored` after touching this file.
    #[test]
    #[ignore = "needs the network"]
    fn the_real_service_still_answers_in_this_shape() {
        let dictionary = crate::dictionary::dictionary().unwrap();
        let entry = dictionary.define("Display", "en", "en").unwrap().unwrap();
        assert_eq!(entry.word, "display");
        let entry = dictionary
            .define("significado", "es", "en")
            .unwrap()
            .unwrap();
        assert!(
            entry
                .senses()
                .any(|(_, sense)| sense.definition.contains("significan"))
        );
        assert!(dictionary.define("qwzzxvbq", "en", "en").unwrap().is_none());
    }

    #[test]
    fn another_explanation_language_is_refused_not_answered_in_english() {
        let dictionary = crate::dictionary::dictionary().unwrap();
        let error = dictionary.define("display", "en", "es").unwrap_err();
        assert!(error.to_string().contains("English only"));
    }
}
