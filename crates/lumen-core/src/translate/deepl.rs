//! DeepL, with the user's own key.
//!
//! The free plan allows 500,000 characters a month. Free keys end in `:fx` and
//! are only accepted by the free host, so the host is chosen from the key
//! rather than asked for as a second setting.

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use super::{Language, Translation, Translator, languages};

const FREE_ENDPOINT: &str = "https://api-free.deepl.com/v2/translate";
const PRO_ENDPOINT: &str = "https://api.deepl.com/v2/translate";

pub struct DeepL {
    client: reqwest::blocking::Client,
    key: String,
}

impl DeepL {
    pub fn new(client: reqwest::blocking::Client, key: String) -> Self {
        Self { client, key }
    }
}

impl Translator for DeepL {
    fn translate(
        &self,
        text: &str,
        from: Option<&'static Language>,
        to: &'static Language,
    ) -> Result<Translation> {
        let body = serde_json::to_vec(&request(text, from, to))?;
        let response = self
            .client
            .post(endpoint(&self.key))
            .header("Authorization", format!("DeepL-Auth-Key {}", self.key))
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .context("reaching DeepL")?;

        let status = response.status();
        match status.as_u16() {
            200 => {}
            403 => bail!("DeepL refused the API key"),
            456 => bail!("DeepL's character quota for this month is used up"),
            429 => bail!("DeepL is receiving too many requests; try again shortly"),
            _ => bail!("DeepL answered {status}"),
        }
        let body = response.text().context("reading DeepL's answer")?;
        parse(&body)
    }
}

/// The host a key is valid on.
fn endpoint(key: &str) -> &'static str {
    if key.ends_with(":fx") {
        FREE_ENDPOINT
    } else {
        PRO_ENDPOINT
    }
}

#[derive(Serialize)]
struct Request<'a> {
    text: [&'a str; 1],
    target_lang: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_lang: Option<String>,
}

fn request<'a>(
    text: &'a str,
    from: Option<&'static Language>,
    to: &'static Language,
) -> Request<'a> {
    Request {
        text: [text],
        target_lang: to.deepl_target(),
        source_lang: from.map(Language::deepl_source),
    }
}

#[derive(Deserialize)]
struct Response {
    translations: Vec<Translated>,
}

#[derive(Deserialize)]
struct Translated {
    text: String,
    detected_source_language: Option<String>,
}

pub fn parse(body: &str) -> Result<Translation> {
    let response: Response =
        serde_json::from_str(body).context("DeepL's answer is not readable")?;
    let Some(first) = response.translations.into_iter().next() else {
        bail!("DeepL returned no translation");
    };
    Ok(Translation {
        detected: first
            .detected_source_language
            .as_deref()
            .and_then(languages::detected),
        text: first.text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translate::languages::find;

    #[test]
    fn a_free_key_goes_to_the_free_host() {
        assert_eq!(endpoint("abc-123:fx"), FREE_ENDPOINT);
        assert_eq!(endpoint("abc-123"), PRO_ENDPOINT);
    }

    #[test]
    fn the_request_uses_deepls_codes_and_omits_an_unknown_source() {
        let json = serde_json::to_value(request("hola", None, find("en").unwrap())).unwrap();
        assert_eq!(json["target_lang"], "EN-US");
        assert_eq!(json["text"][0], "hola");
        assert!(json.get("source_lang").is_none());

        let json = serde_json::to_value(request("hola", find("es"), find("pt").unwrap())).unwrap();
        assert_eq!(json["source_lang"], "ES");
        assert_eq!(json["target_lang"], "PT-BR");
    }

    #[test]
    fn the_answer_gives_the_text_and_the_source() {
        let translation =
            parse(r#"{"translations":[{"detected_source_language":"ES","text":"Hello"}]}"#)
                .unwrap();
        assert_eq!(translation.text, "Hello");
        assert_eq!(translation.detected.unwrap().code, "es");
    }

    #[test]
    fn an_empty_answer_is_an_error() {
        assert!(parse(r#"{"translations":[]}"#).is_err());
        assert!(parse("Forbidden").is_err());
    }
}
