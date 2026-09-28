//! The languages a preset may name.
//!
//! A fixed table rather than whatever a service reports, because the keyless
//! Google endpoint publishes no list to ask, and because a preset that loads
//! with one service must load with the other: switching `service` must never
//! turn a working preset into a skipped one. So this is the intersection --
//! the languages DeepL translates, all of which Google does too.

/// One language, and what each service calls it.
#[derive(Debug, PartialEq, Eq)]
pub struct Language {
    /// What a preset writes: an ISO 639-1 code, lower case.
    pub code: &'static str,
    /// What the user reads, in English. English because every other label in
    /// the launcher is.
    pub name: &'static str,
    /// Google's code, where it differs from `code`.
    google: Option<&'static str>,
    /// DeepL's code for a *target*, where it differs from the upper-cased
    /// `code`. DeepL deprecated bare `EN` and `PT` as targets; the variants
    /// chosen match what Google produces for the same code, so a preset reads
    /// the same whichever service is set.
    deepl_target: Option<&'static str>,
}

impl Language {
    /// The code sent to Google.
    pub fn google(&self) -> &'static str {
        self.google.unwrap_or(self.code)
    }

    /// The code sent to DeepL as a source.
    pub fn deepl_source(&self) -> String {
        self.code.to_ascii_uppercase()
    }

    /// The code sent to DeepL as a target.
    pub fn deepl_target(&self) -> String {
        self.deepl_target
            .map(str::to_string)
            .unwrap_or_else(|| self.deepl_source())
    }
}

const fn language(code: &'static str, name: &'static str) -> Language {
    Language {
        code,
        name,
        google: None,
        deepl_target: None,
    }
}

pub const LANGUAGES: &[Language] = &[
    language("ar", "Arabic"),
    language("bg", "Bulgarian"),
    language("cs", "Czech"),
    language("da", "Danish"),
    language("de", "German"),
    language("el", "Greek"),
    Language {
        deepl_target: Some("EN-US"),
        ..language("en", "English")
    },
    language("es", "Spanish"),
    language("et", "Estonian"),
    language("fi", "Finnish"),
    language("fr", "French"),
    language("hu", "Hungarian"),
    language("id", "Indonesian"),
    language("it", "Italian"),
    language("ja", "Japanese"),
    language("ko", "Korean"),
    language("lt", "Lithuanian"),
    language("lv", "Latvian"),
    Language {
        google: Some("no"),
        ..language("nb", "Norwegian")
    },
    language("nl", "Dutch"),
    language("pl", "Polish"),
    Language {
        deepl_target: Some("PT-BR"),
        ..language("pt", "Portuguese")
    },
    language("ro", "Romanian"),
    language("ru", "Russian"),
    language("sk", "Slovak"),
    language("sl", "Slovenian"),
    language("sv", "Swedish"),
    language("tr", "Turkish"),
    language("uk", "Ukrainian"),
    Language {
        google: Some("zh-CN"),
        deepl_target: Some("ZH-HANS"),
        ..language("zh", "Chinese")
    },
];

/// The language a preset names, by code or by name, ignoring case.
///
/// Names are accepted as well as codes because `"from": "Spanish"` is what
/// somebody writes before they have looked anything up.
pub fn find(code_or_name: &str) -> Option<&'static Language> {
    let wanted = code_or_name.trim();
    LANGUAGES.iter().find(|language| {
        language.code.eq_ignore_ascii_case(wanted) || language.name.eq_ignore_ascii_case(wanted)
    })
}

/// The language a service reported detecting, by its own code.
///
/// Lenient about regions -- `EN-GB`, `zh-TW` -- because a detection that names
/// a variant is still a detection of the language. A code outside the table is
/// `None`, and the view says the source is unknown rather than guessing.
pub fn detected(code: &str) -> Option<&'static Language> {
    let code = code.trim();
    if let Some(language) = LANGUAGES
        .iter()
        .find(|language| language.google().eq_ignore_ascii_case(code))
    {
        return Some(language);
    }
    let base = code.split(['-', '_']).next().unwrap_or(code);
    LANGUAGES
        .iter()
        .find(|language| language.code.eq_ignore_ascii_case(base))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_language_is_found_by_code_or_name_in_any_case() {
        assert_eq!(find("es").unwrap().name, "Spanish");
        assert_eq!(find("ES").unwrap().name, "Spanish");
        assert_eq!(find("spanish").unwrap().code, "es");
        assert!(find("klingon").is_none());
    }

    #[test]
    fn codes_are_unique() {
        for (index, language) in LANGUAGES.iter().enumerate() {
            assert!(
                LANGUAGES[index + 1..]
                    .iter()
                    .all(|other| other.code != language.code),
                "{} appears twice",
                language.code
            );
        }
    }

    #[test]
    fn each_service_gets_its_own_spelling() {
        let english = find("en").unwrap();
        assert_eq!(english.google(), "en");
        assert_eq!(english.deepl_source(), "EN");
        assert_eq!(english.deepl_target(), "EN-US");

        let chinese = find("zh").unwrap();
        assert_eq!(chinese.google(), "zh-CN");
        assert_eq!(chinese.deepl_target(), "ZH-HANS");
    }

    #[test]
    fn a_detected_variant_is_its_language() {
        assert_eq!(detected("EN-GB").unwrap().code, "en");
        assert_eq!(detected("zh-CN").unwrap().code, "zh");
        assert_eq!(detected("zh-TW").unwrap().code, "zh");
        assert_eq!(detected("no").unwrap().code, "nb");
        assert_eq!(detected("NB").unwrap().code, "nb");
        assert!(detected("xx").is_none());
    }
}
