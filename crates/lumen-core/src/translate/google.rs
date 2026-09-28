//! Google's keyless endpoint.
//!
//! The one Chrome's dictionary extension uses: no key, no account, which is
//! why it is the default -- the command works the first time it is opened. It
//! is also undocumented, so it can change shape or refuse requests without
//! notice. [`parse`] is strict about what it needs and ignores the rest, and a
//! failure is shown in the view as a failure rather than as an empty
//! translation.
//!
//! Not `translate.googleapis.com/translate_a/single`, the endpoint most
//! libraries use. Measured while building this: it answered the blocking
//! `reqwest` client with its "unusual traffic" page (429) on the first
//! request, whatever the headers, while curl on the same machine got 200 --
//! it judges the TLS client, not the request. This host answered the same
//! client normally.

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use super::{Language, Translation, Translator, languages};

const ENDPOINT: &str = "https://clients5.google.com/translate_a/t";

pub struct Google {
    client: reqwest::blocking::Client,
}

impl Google {
    pub fn new(client: reqwest::blocking::Client) -> Self {
        Self { client }
    }
}

impl Translator for Google {
    fn translate(
        &self,
        text: &str,
        from: Option<&'static Language>,
        to: &'static Language,
    ) -> Result<Translation> {
        let url = url(text, from, to)?;
        // Without the URL: it carries the text, and errors reach the log.
        let response = self
            .client
            .get(url)
            .send()
            .map_err(reqwest::Error::without_url)
            .context("reaching Google")?;
        let status = response.status();
        if !status.is_success() {
            bail!("Google answered {status}");
        }
        let body = response
            .text()
            .map_err(reqwest::Error::without_url)
            .context("reading Google's answer")?;
        parse(&body)
    }
}

fn url(text: &str, from: Option<&'static Language>, to: &'static Language) -> Result<url::Url> {
    Ok(url::Url::parse_with_params(
        ENDPOINT,
        [
            ("client", "dict-chrome-ex"),
            ("sl", from.map_or("auto", Language::google)),
            ("tl", to.google()),
            ("q", text),
        ],
    )?)
}

/// Read the endpoint's answer.
///
/// One entry per `q` sent, and one is sent. With the source detected it is
/// `[["translation", "es"]]`; with the source given, just `["translation"]`.
pub fn parse(body: &str) -> Result<Translation> {
    let value: Value = serde_json::from_str(body).context("Google's answer is not JSON")?;
    let first = value
        .get(0)
        .context("Google's answer has no translation in it")?;

    let (text, detected) = match first {
        Value::String(text) => (text.as_str(), None),
        Value::Array(pair) => (
            pair.first()
                .and_then(Value::as_str)
                .context("Google's answer has no translation in it")?,
            pair.get(1).and_then(Value::as_str),
        ),
        _ => bail!("Google's answer has no translation in it"),
    };
    if text.trim().is_empty() {
        bail!("Google returned an empty translation");
    }

    Ok(Translation {
        text: text.to_string(),
        detected: detected.and_then(languages::detected),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translate::languages::find;

    // Both bodies are copied from the live endpoint.

    #[test]
    fn a_detected_source_comes_with_the_translation() {
        let translation = parse(r#"[["Hello how are you? Good morning.","es"]]"#).unwrap();
        assert_eq!(translation.text, "Hello how are you? Good morning.");
        assert_eq!(translation.detected.unwrap().code, "es");
    }

    #[test]
    fn a_given_source_is_not_repeated_back() {
        let translation = parse(r#"["Hello how are you?\nGood morning."]"#).unwrap();
        assert_eq!(translation.text, "Hello how are you?\nGood morning.");
        assert_eq!(translation.detected, None);
    }

    #[test]
    fn a_source_outside_the_table_is_unknown_rather_than_a_guess() {
        assert_eq!(parse(r#"[["Hello","haw"]]"#).unwrap().detected, None);
    }

    #[test]
    fn an_answer_of_another_shape_is_an_error() {
        assert!(parse("<html>429</html>").is_err());
        assert!(parse(r#"{"error": "nope"}"#).is_err());
        assert!(parse("[]").is_err());
        assert!(parse("[[]]").is_err());
        assert!(parse(r#"[""]"#).is_err());
    }

    #[test]
    fn the_request_names_both_languages_in_googles_spelling() {
        let url = url("你好 & more", find("zh"), find("en").unwrap()).unwrap();
        let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        assert!(query.contains(&("sl".into(), "zh-CN".into())));
        assert!(query.contains(&("tl".into(), "en".into())));
        assert!(query.contains(&("q".into(), "你好 & more".into())));
    }

    /// Against the real endpoint. Ignored because CI must not depend on a
    /// third party answering; run with `cargo test -- --ignored` after
    /// touching this file, since the endpoint is undocumented.
    #[test]
    #[ignore = "needs the network"]
    fn the_real_endpoint_still_answers_in_this_shape() {
        let config = crate::translate::TranslateConfig::default();
        let google = crate::translate::translator(&config).unwrap();
        let translation = google
            .translate("Buenos días", None, find("en").unwrap())
            .unwrap();
        assert_eq!(translation.detected.unwrap().code, "es");
        assert!(translation.text.to_lowercase().contains("good morning"));
    }

    #[test]
    fn no_source_asks_google_to_detect_it() {
        let url = url("hola", None, find("en").unwrap()).unwrap();
        assert!(url.query().unwrap().contains("sl=auto"));
    }
}
