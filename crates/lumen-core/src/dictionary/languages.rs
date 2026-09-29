//! The languages a word can be looked up in (F10).
//!
//! Not Translate's table: that one is what DeepL translates, and a
//! dictionary's reach is whatever Wiktionary has written up. This is the
//! living languages with roughly ten thousand entries or more in English
//! Wiktionary, as FreeDictionaryAPI counted them on 2026-09-29 -- enough that
//! a common word is likely to be there. Historical languages (Old English,
//! Gothic) and Wiktionary's "Translingual" are left out: they are for
//! etymologists, and a picker of 279 is not choosing.
//!
//! Codes are Wiktionary's, which are ISO 639 where one exists: `sh` for
//! Serbo-Croatian, which Wiktionary treats as one language.

/// One language a word can belong to.
#[derive(Debug, PartialEq, Eq)]
pub struct Language {
    pub code: &'static str,
    /// In English, like every other label in the launcher.
    pub name: &'static str,
}

const fn language(code: &'static str, name: &'static str) -> Language {
    Language { code, name }
}

pub const LANGUAGES: &[Language] = &[
    language("sq", "Albanian"),
    language("ar", "Arabic"),
    language("hy", "Armenian"),
    language("az", "Azerbaijani"),
    language("bg", "Bulgarian"),
    language("ca", "Catalan"),
    language("zh", "Chinese"),
    language("cs", "Czech"),
    language("da", "Danish"),
    language("nl", "Dutch"),
    language("en", "English"),
    language("eo", "Esperanto"),
    language("et", "Estonian"),
    language("fi", "Finnish"),
    language("fr", "French"),
    language("gl", "Galician"),
    language("ka", "Georgian"),
    language("de", "German"),
    language("el", "Greek"),
    language("he", "Hebrew"),
    language("hi", "Hindi"),
    language("hu", "Hungarian"),
    language("is", "Icelandic"),
    language("id", "Indonesian"),
    language("ga", "Irish"),
    language("it", "Italian"),
    language("ja", "Japanese"),
    language("kk", "Kazakh"),
    language("ko", "Korean"),
    language("la", "Latin"),
    language("lv", "Latvian"),
    language("lt", "Lithuanian"),
    language("mk", "Macedonian"),
    language("ms", "Malay"),
    language("ml", "Malayalam"),
    language("nb", "Norwegian Bokmål"),
    language("nn", "Norwegian Nynorsk"),
    language("fa", "Persian"),
    language("pl", "Polish"),
    language("pt", "Portuguese"),
    language("ro", "Romanian"),
    language("ru", "Russian"),
    language("sa", "Sanskrit"),
    language("gd", "Scottish Gaelic"),
    language("sh", "Serbo-Croatian"),
    language("sk", "Slovak"),
    language("es", "Spanish"),
    language("sw", "Swahili"),
    language("sv", "Swedish"),
    language("tl", "Tagalog"),
    language("ta", "Tamil"),
    language("te", "Telugu"),
    language("th", "Thai"),
    language("tr", "Turkish"),
    language("uk", "Ukrainian"),
    language("vi", "Vietnamese"),
    language("cy", "Welsh"),
    language("yi", "Yiddish"),
];

/// English: where the view starts, and what an unknown code falls back to.
pub fn english() -> &'static Language {
    find("en").expect("English is in the table")
}

/// A language by code or name, ignoring case.
pub fn find(code_or_name: &str) -> Option<&'static Language> {
    let wanted = code_or_name.trim();
    LANGUAGES.iter().find(|language| {
        language.code.eq_ignore_ascii_case(wanted) || language.name.eq_ignore_ascii_case(wanted)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_the_table_is_sorted_by_name() {
        for pair in LANGUAGES.windows(2) {
            assert!(
                pair[0].name < pair[1].name,
                "{} before {}",
                pair[0].name,
                pair[1].name
            );
        }
        for (index, language) in LANGUAGES.iter().enumerate() {
            assert!(
                LANGUAGES[index + 1..]
                    .iter()
                    .all(|other| other.code != language.code)
            );
        }
    }

    #[test]
    fn a_language_is_found_by_code_or_name() {
        assert_eq!(find("ES").unwrap().name, "Spanish");
        assert_eq!(find("french").unwrap().code, "fr");
        assert!(find("klingon").is_none());
        assert_eq!(english().code, "en");
    }
}
