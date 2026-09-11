//! The privacy layer, adopted from Asyar (F3).
//!
//! > Cheap to implement in Rust and with a high return in perceived quality: a
//! > denylist of password managers, respect for the "do not include in history"
//! > clipboard flags on Windows, and regex redaction of known secret formats
//! > before writing to disk.
//!
//! Three of those four live here. The Windows clipboard flags are checked in
//! the platform layer, because only it can read them.
//!
//! # Skipping, not redacting
//!
//! The plan says "redaction". This skips the entry entirely instead, and the
//! difference is deliberate: a redacted entry still records that a secret was
//! copied, when, and what surrounded it. Storing the surroundings of a private
//! key is not meaningfully safer than storing the key. A clipboard history that
//! quietly omits a few entries is a far better failure than one that keeps a
//! partial record of every credential the user has ever handled.
//!
//! # The bar for a pattern
//!
//! Every pattern here matches a credential format with a distinctive,
//! documented prefix. Deliberately absent is anything that guesses -- no
//! entropy heuristics, no "looks like base64", no length thresholds. Those
//! produce false positives, and a false positive here is an entry the user
//! expected to find and cannot, with no explanation. Under-matching loses a
//! feature; over-matching loses trust.

use regex::RegexSet;
use std::sync::LazyLock;

/// Executables whose clipboard writes are never recorded.
///
/// Matched on the foreground application at the moment of the copy. A password
/// manager that sets the Windows exclusion flags is caught by the platform
/// anyway; this covers the ones that do not.
const DENYLISTED_APPS: &[&str] = &[
    "1password.exe",
    "keepass.exe",
    "keepassxc.exe",
    "bitwarden.exe",
    "lastpass.exe",
    "dashlane.exe",
    "enpass.exe",
    "protonpass.exe",
    "nordpass.exe",
];

/// Credential formats, each with a documented prefix.
static SECRETS: LazyLock<RegexSet> = LazyLock::new(|| {
    RegexSet::new([
        // Any PEM private key block.
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----",
        // AWS access key id.
        r"\b(AKIA|ASIA)[0-9A-Z]{16}\b",
        // GitHub personal access, OAuth, user-to-server, server-to-server and
        // refresh tokens all share this shape.
        r"\bgh[pousr]_[A-Za-z0-9]{36,}\b",
        // Slack.
        r"\bxox[baprs]-[A-Za-z0-9-]{10,}",
        // OpenAI and Anthropic.
        r"\bsk-(ant-)?[A-Za-z0-9_-]{20,}",
        // Google API keys.
        r"\bAIza[0-9A-Za-z_-]{35}\b",
        // Stripe live keys. Test keys are deliberately not matched: they are
        // not secrets and people paste them constantly.
        r"\b[rs]k_live_[0-9A-Za-z]{24,}",
        // A JSON Web Token: three base64url segments, the first two of which
        // decode to objects beginning `{"`.
        r"\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
        // Private keys and tokens in a URL's userinfo.
        r"://[^/\s:@]+:[^/\s:@]+@",
    ])
    .expect("the secret patterns are literals and compile")
});

/// Why an entry was not recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    /// The foreground application is on the denylist.
    DeniedApplication,
    /// The text matches a known credential format.
    LooksLikeASecret,
    /// Nothing worth keeping.
    Empty,
}

/// Decide whether a clipboard entry may be recorded.
///
/// `source` is the executable that was in the foreground when the copy
/// happened, lowercased, or `None` when it could not be identified.
pub fn may_record(text: &str, source: Option<&str>) -> Result<(), Rejected> {
    if text.trim().is_empty() {
        return Err(Rejected::Empty);
    }

    if let Some(source) = source
        && DENYLISTED_APPS
            .iter()
            .any(|denied| denied.eq_ignore_ascii_case(source))
    {
        return Err(Rejected::DeniedApplication);
    }

    if SECRETS.is_match(text) {
        return Err(Rejected::LooksLikeASecret);
    }

    Ok(())
}

/// Whether `executable` is a password manager.
pub fn is_denylisted(executable: &str) -> bool {
    DENYLISTED_APPS
        .iter()
        .any(|denied| denied.eq_ignore_ascii_case(executable))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_text_is_recorded() {
        assert_eq!(may_record("the quick brown fox", None), Ok(()));
        assert_eq!(may_record("https://example.com/page", None), Ok(()));
        assert_eq!(may_record("SELECT * FROM users", Some("code.exe")), Ok(()));
    }

    #[test]
    fn empty_and_whitespace_are_not_recorded() {
        assert_eq!(may_record("", None), Err(Rejected::Empty));
        assert_eq!(may_record("   \n\t ", None), Err(Rejected::Empty));
    }

    #[test]
    fn password_managers_are_denied_whatever_they_copied() {
        assert_eq!(
            may_record("hunter2", Some("1password.exe")),
            Err(Rejected::DeniedApplication)
        );
        // Case-insensitively: the foreground name is lowercased by the platform,
        // but this must not depend on that having happened.
        assert_eq!(
            may_record("hunter2", Some("KeePassXC.exe")),
            Err(Rejected::DeniedApplication)
        );
    }

    #[test]
    fn private_keys_are_never_recorded() {
        let key = "-----BEGIN RSA PRIVATE KEY-----\nMIIEow...\n-----END RSA PRIVATE KEY-----";
        assert_eq!(may_record(key, None), Err(Rejected::LooksLikeASecret));

        assert_eq!(
            may_record("-----BEGIN OPENSSH PRIVATE KEY-----\nabc", None),
            Err(Rejected::LooksLikeASecret)
        );
    }

    #[test]
    fn cloud_credentials_are_never_recorded() {
        for secret in [
            "AKIAIOSFODNN7EXAMPLE",
            "ghp_1234567890abcdefghijklmnopqrstuvwxyz",
            "xoxb-123456789012-abcdefghijkl",
            "sk-ant-api03-abcdefghijklmnopqrstuvwxyz123456",
            "AIzaSyC1234567890abcdefghijklmnopqrstuv",
            "sk_live_abcdefghijklmnopqrstuvwx",
        ] {
            assert_eq!(
                may_record(secret, None),
                Err(Rejected::LooksLikeASecret),
                "{secret} should have been rejected"
            );
        }
    }

    #[test]
    fn a_secret_anywhere_in_the_text_rejects_the_whole_entry() {
        // The surroundings of a key are not safe to keep just because the key
        // is not alone -- which is the whole reason this skips rather than
        // redacts.
        let text = "here you go:\nAKIAIOSFODNN7EXAMPLE\nlet me know if it works";
        assert_eq!(may_record(text, None), Err(Rejected::LooksLikeASecret));
    }

    #[test]
    fn json_web_tokens_are_never_recorded() {
        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.\
                   eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4ifQ.\
                   SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        assert_eq!(may_record(jwt, None), Err(Rejected::LooksLikeASecret));
    }

    #[test]
    fn credentials_embedded_in_a_url_are_never_recorded() {
        assert_eq!(
            may_record("https://admin:s3cret@internal.example.com/", None),
            Err(Rejected::LooksLikeASecret)
        );
    }

    // The false-positive cases matter as much as the true ones: an entry the
    // user expected to find and cannot is a bug they cannot diagnose.

    #[test]
    fn ordinary_text_that_merely_resembles_a_secret_is_kept() {
        for innocent in [
            // Long base64-ish strings are everywhere.
            "dGhlIHF1aWNrIGJyb3duIGZveCBqdW1wcyBvdmVyIHRoZSBsYXp5IGRvZw==",
            // A git hash.
            "a57646ac9f2e4b1d8c3a5e7f9b2d4c6a8e0f1a3b",
            // Stripe *test* keys are not secrets and are pasted constantly.
            "sk_test_abcdefghijklmnopqrstuvwx",
            // A plain URL with a port, which the userinfo pattern must not eat.
            "https://example.com:8443/path",
            // Prose containing the word "private key".
            "remember to rotate the private key next week",
        ] {
            assert_eq!(
                may_record(innocent, None),
                Ok(()),
                "{innocent} should have been kept"
            );
        }
    }

    #[test]
    fn an_unidentified_source_does_not_block_recording() {
        // An elevated process cannot be opened, so the executable is unknown.
        // Refusing to record everything copied from any elevated window would
        // quietly break the feature.
        assert_eq!(may_record("ordinary text", None), Ok(()));
    }

    #[test]
    fn the_denylist_is_exposed_for_the_platform_to_consult() {
        assert!(is_denylisted("1Password.exe"));
        assert!(!is_denylisted("code.exe"));
    }
}
