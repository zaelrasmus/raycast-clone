//! What the features that ask a service over the network share: sorting why
//! a request failed into what the user needs to be told.
//!
//! Written for Translate (F7) and moved here when Currency (F9) needed the
//! same sorting.

/// Why an answer did not arrive, as far as the user needs to know.
#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    /// The service could not be reached at all: no network, no DNS, a
    /// connection refused. Said as such, because the raw error -- "error
    /// sending request" -- tells nobody their Wi-Fi is off.
    Offline,
    /// It was reached and something else went wrong; the message says what.
    Other(String),
}

/// Sort an error from a request into a [`Failure`].
///
/// Offline is a connection that never opened. A timeout is not: the service
/// was slow, and "you are offline" would send the user to check a network
/// that works.
pub fn failure(error: &anyhow::Error) -> Failure {
    let request = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<reqwest::Error>());
    match request {
        Some(request) if request.is_connect() => Failure::Offline,
        Some(request) if request.is_timeout() => {
            Failure::Other("The service did not answer in time".into())
        }
        _ => Failure::Other(format!("{error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context as _;
    use std::time::Duration;

    /// A connection that cannot open is "offline", checked against a closed
    /// port on this machine so the test needs no network.
    #[test]
    fn a_connection_that_never_opens_is_offline() {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let error = client
            .get("http://127.0.0.1:9/")
            .send()
            .map_err(reqwest::Error::without_url)
            .context("reaching Google")
            .unwrap_err();
        assert_eq!(failure(&error), Failure::Offline);
    }

    #[test]
    fn any_other_error_keeps_its_message() {
        let error = anyhow::anyhow!("Google answered 429 Too Many Requests");
        assert_eq!(
            failure(&error),
            Failure::Other("Google answered 429 Too Many Requests".into())
        );
    }
}
