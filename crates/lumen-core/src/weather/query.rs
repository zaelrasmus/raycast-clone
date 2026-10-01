//! What the root list recognises as a question about the weather (F13,
//! slice 5): `weather in vancouver`, `weather vancouver`, `vancouver
//! weather`.
//!
//! The word *weather* is required, so typing a city alone never sends
//! anything anywhere. Recognising the shape is cheap and is all this does;
//! whether the place is named whole is the catalogue's to say, and until it
//! is, no answer is given and nothing is fetched.

/// The place a query asks the weather of, as typed.
pub fn recognise(query: &str) -> Option<&str> {
    let query = query.trim();
    let lower = query.to_ascii_lowercase();
    let place = if let Some(rest) = lower.strip_prefix("weather in ") {
        &query[query.len() - rest.len()..]
    } else if let Some(rest) = lower.strip_prefix("weather ") {
        &query[query.len() - rest.len()..]
    } else if let Some(rest) = lower.strip_suffix(" weather") {
        &query[..rest.len()]
    } else {
        return None;
    };
    let place = place.trim();
    // `weather in` alone, still being typed.
    (!place.is_empty() && !place.eq_ignore_ascii_case("in")).then_some(place)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_shapes_and_the_word_is_required() {
        assert_eq!(recognise("weather in Vancouver"), Some("Vancouver"));
        assert_eq!(recognise("  Weather vancouver "), Some("vancouver"));
        assert_eq!(recognise("vancouver weather"), Some("vancouver"));
        assert_eq!(recognise("weather in london canada"), Some("london canada"));
        for query in [
            "weather",
            "weather in",
            "weather in ",
            "vancouver",
            "weathervane",
        ] {
            assert_eq!(recognise(query), None, "{query}");
        }
    }
}
