//! Translation presets as rows in the root list (F7).
//!
//! A catalogue, not an answer: the rows are the presets, which exist whether
//! or not anyone is searching, and choosing one opens the translate view.
//! Nothing here translates. Root search matching `es en` finds a preset; it
//! never sends `es en` anywhere (I-3).

use crate::item::{Action, Item, Kind, ViewSource};
use crate::providers::Provider;
use crate::translate::Preset;

pub struct TranslateProvider {
    presets: Vec<Preset>,
}

impl TranslateProvider {
    pub fn new(presets: Vec<Preset>) -> Self {
        Self { presets }
    }
}

impl Provider for TranslateProvider {
    fn id(&self) -> &'static str {
        "translate"
    }

    fn catalogue(&self) -> Vec<Item> {
        // The plain command opens whichever preset was used last, so the
        // person who only ever translates one way never has to choose.
        let open = Item {
            id: "translate:open".into(),
            title: "Translate".into(),
            subtitle: Some("Translate text with your last-used preset".into()),
            icon: None,
            keywords: vec!["translate".into(), "translation".into(), "language".into()],
            actions: vec![Action::OpenView(ViewSource::Translate { preset: None })],
            kind: Kind::Command,
        };

        std::iter::once(open)
            .chain(self.presets.iter().map(preset))
            .collect()
    }
}

fn preset(preset: &Preset) -> Item {
    // Codes and names of every language involved, so `es en`, `spanish` and
    // the alias all find it. "translate" too: typing the verb should list
    // every preset, not only the plain command.
    let mut keywords = vec!["translate".to_string()];
    keywords.extend(preset.alias.clone());
    for language in preset.from.iter().chain(&preset.to) {
        keywords.push(language.code.to_string());
        keywords.push(language.name.to_lowercase());
    }

    Item {
        // By name, so usage survives reordering the presets in the file.
        id: format!("translate:preset:{}", preset.name),
        title: preset.name.clone(),
        subtitle: Some(format!("Translate {}", preset.describe())),
        icon: None,
        keywords,
        actions: vec![Action::OpenView(ViewSource::Translate {
            preset: Some(preset.name.clone()),
        })],
        kind: Kind::Command,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translate::TranslateConfig;

    fn provider(source: &str) -> TranslateProvider {
        let config: TranslateConfig = serde_json::from_str(source).unwrap();
        TranslateProvider::new(config.presets())
    }

    #[test]
    fn every_preset_is_a_row_beside_the_plain_command() {
        let items = provider(
            r#"{ "presets": [
                { "name": "Spanish → English", "alias": "esen", "from": "es", "to": "en" },
                { "name": "To French", "to": "fr" }
            ] }"#,
        )
        .catalogue();

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].title, "Translate");
        assert_eq!(items[1].title, "Spanish → English");
        assert_eq!(
            items[1].subtitle.as_deref(),
            Some("Translate Spanish to English")
        );
        assert_eq!(
            items[1].primary(),
            Action::OpenView(ViewSource::Translate {
                preset: Some("Spanish → English".into())
            })
        );
    }

    #[test]
    fn a_preset_is_found_by_its_alias_and_its_languages() {
        let items = provider(
            r#"{ "presets": [ { "name": "Work", "alias": "esen", "from": "es", "to": "en" } ] }"#,
        )
        .catalogue();

        let keywords = &items[1].keywords;
        for word in ["esen", "es", "en", "spanish", "english", "translate"] {
            assert!(keywords.iter().any(|keyword| keyword == word), "{word}");
        }
    }

    #[test]
    fn with_no_presets_the_implied_one_is_offered() {
        let items = provider("{}").catalogue();
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[1].subtitle.as_deref(),
            Some("Translate any language to English")
        );
    }
}
