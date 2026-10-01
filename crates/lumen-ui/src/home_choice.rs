//! The home city's dropdown, on the World Clock and Weather pages alike
//! (F13): one value, `places.home`, edited from either.
//!
//! Home is *Automatic* -- the largest city in the system's zone -- or one
//! of *My Places*, which the views are where to add. The choices are read
//! from `places.json` and labelled from their keys, so opening settings
//! never reads the catalogue of 34,000 cities. A home no longer among
//! *My Places* stays a choice of its own, rather than lost by the next save.

use crate::select::Choice;

/// The value of the *Automatic* choice.
const AUTOMATIC: &str = "";

/// What both pages say under the field.
pub const HINT: &str =
    "Shared by World Clock and Weather. Add a city with Add to My Places in either.";

/// *Automatic*, then *My Places*, then the saved home if it is neither, and
/// which is chosen.
pub fn choices(home: Option<&str>, mine: &[String]) -> (Vec<Choice>, usize) {
    let zone = lumen_core::clock::here()
        .iana_name()
        .map(|zone| format!(" ({zone})"))
        .unwrap_or_default();
    let mut choices = vec![Choice {
        value: Some(AUTOMATIC.into()),
        label: format!("Automatic, from your time zone{zone}"),
    }];
    let mut keys: Vec<&str> = mine.iter().map(String::as_str).collect();
    if let Some(home) = home
        && !keys.contains(&home)
    {
        keys.push(home);
    }
    choices.extend(keys.into_iter().map(|key| Choice {
        value: Some(key.into()),
        label: label(key),
    }));
    let selected = home
        .and_then(|home| {
            choices
                .iter()
                .position(|choice| choice.value.as_deref() == Some(home))
        })
        .unwrap_or(0);
    (choices, selected)
}

/// The home a dropdown's value means: `None` is *Automatic*.
pub fn chosen(value: Option<&str>) -> Option<String> {
    value
        .filter(|value| *value != AUTOMATIC)
        .map(str::to_string)
}

/// `CA/British Columbia/Vancouver` as `Vancouver, British Columbia (CA)`.
fn label(key: &str) -> String {
    let mut parts = key.splitn(3, '/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(country), Some(""), Some(name)) => format!("{name} ({country})"),
        (Some(country), Some(region), Some(name)) => format!("{name}, {region} ({country})"),
        _ => key.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_reads_as_a_place() {
        assert_eq!(
            label("CA/British Columbia/Vancouver"),
            "Vancouver, British Columbia (CA)"
        );
        assert_eq!(label("SG//Singapore"), "Singapore (SG)");
        assert_eq!(label("broken"), "broken");
    }

    #[test]
    fn a_home_no_longer_kept_is_still_a_choice() {
        let mine = vec!["CA/British Columbia/Vancouver".to_string()];
        let (choices, selected) = choices(Some("JP/Tokyo/Tokyo"), &mine);
        assert_eq!(choices.len(), 3);
        assert_eq!(choices[selected].label, "Tokyo, Tokyo (JP)");
        assert_eq!(
            chosen(choices[selected].value.as_deref()).as_deref(),
            Some("JP/Tokyo/Tokyo")
        );

        let (choices, selected) = super::choices(None, &mine);
        assert_eq!(selected, 0);
        assert_eq!(chosen(choices[0].value.as_deref()), None, "automatic");
    }
}
