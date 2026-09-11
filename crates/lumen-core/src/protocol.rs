//! What the host and an extension say to each other (F4, step 3).
//!
//! # This is the transport's envelope, not the contract
//!
//! AD-06 draws a line that this file sits on the reversible side of: "The
//! contract is the durable artefact, the transport is reversible." The durable
//! part is [`crate::view`] -- the tree and the events, defined once (I-13).
//! What is here is only the envelope they travel in, plus the requests an
//! extension makes of the host.
//!
//! So this file may be replaced wholesale when the transport becomes the
//! WebAssembly component model, and `view.rs` will not move. That is the point
//! of having drawn the line: a change of transport is a change of one file.
//!
//! # Every capability goes through the host
//!
//! AD-09: "Network, filesystem, execution, clipboard and notifications pass
//! through the host even where the subprocess could technically perform them
//! directly... what matters is that the boundary is drawn, so that introducing
//! a sandbox later means changing the runtime rather than redesigning the
//! system." I-7 says the same as an invariant.
//!
//! Today an extension is an ordinary subprocess and *could* reach the clipboard
//! itself. [`Capability`] exists anyway, and is a closed set for the same
//! reason the view vocabulary is: the day the runtime is a sandbox, the set of
//! things an extension can ask for is already written down, already the only
//! route, and already what every extension uses.
//!
//! **What this does not do, stated plainly.** There is no sandbox. An extension
//! is a process the user installed, running with the user's own privileges,
//! exactly like the tier-T1 script commands beside it. Installing one is
//! trusting it. The boundary drawn here is what makes that *changeable*; it is
//! not a claim that it has already changed.

use serde::{Deserialize, Serialize};

use crate::view::{Event, Tone, View};

/// What the host sends an extension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ToExtension {
    /// Run this command. Always the first message, and sent exactly once.
    Run { command: String },
    /// Something happened in the view.
    ///
    /// Nested under a named field for the same reason [`FromExtension::Render`]
    /// is: this enum and [`Event`] are both tagged with `type`, and flattening
    /// one into the other produces an object with two `type` keys. Writing the
    /// first extension is what found it -- the reader had to fish the
    /// discriminator out by hand -- which is exactly the correction the plan's
    /// build order expects from putting an extension in front of a protocol
    /// before stabilising it.
    Event { event: Event },
    /// The answer to a [`Capability`] request, by its `id`.
    Response { id: u64, result: Answer },
}

/// The result of a capability request.
///
/// Two variants rather than `Result`, because `Result` serialises as `Ok`/`Err`
/// keys that read oddly from a language with no such type -- and the first
/// extensions will be written in whatever the author already has installed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum Answer {
    Ok {
        /// Present only for requests that return something.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<String>,
    },
    Failed {
        message: String,
    },
}

/// What an extension sends the host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum FromExtension {
    /// Draw this. Sent as often as the extension likes; the last one wins.
    ///
    /// The view is a *named field* rather than the variant's content, and that
    /// is not a style choice. Both this enum and [`View`] are tagged with
    /// `type`, so flattening one into the other produces an object with two
    /// `type` keys: unwritable by hand and rejected on the way back in. The
    /// test below pins it, because it is the kind of thing a later refactor
    /// undoes without noticing.
    Render { view: View },
    /// Ask the host to do something (I-7).
    ///
    /// Flattened, so the wire shape is
    /// `{"type":"request","id":1,"capability":"copy-to-clipboard","text":"…"}`
    /// rather than a `capability` object nested inside a `capability` field.
    /// The two enums are tagged with different keys, so there is nothing to
    /// collide -- and the first extension written against this got it wrong in
    /// exactly the direction the flattened shape makes right, which is the
    /// correction the plan's step 4 exists to collect.
    Request {
        id: u64,
        #[serde(flatten)]
        capability: Capability,
    },
    /// Say something briefly, without taking over the view.
    Toast { message: String, tone: Tone },
    /// Close the view and go back to the root list.
    Close,
}

/// Everything an extension may ask the host to do.
///
/// A closed set, and short. Each variant is a thing the host already knows how
/// to do and already does through `trait Platform`, so adding one is a decision
/// about what extensions may do -- which is the decision worth making slowly --
/// rather than a decision about how to do it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "capability", rename_all = "kebab-case")]
pub enum Capability {
    /// Put text on the system clipboard.
    CopyToClipboard { text: String },
    /// Read the clipboard as text. Answers with the text, or nothing.
    ReadClipboard,
    /// Hand a URL or a path to the shell.
    ///
    /// The same power a tier-T1 script command already has, and the same trust
    /// model: this is code the user installed. It is a capability rather than
    /// something the extension does itself so that the day there is a sandbox,
    /// this is the thing that gets a policy attached to it.
    Open { target: String },
}

/// Parse one line from an extension.
///
/// Lines rather than a framed protocol because an extension is whatever the
/// author already had installed -- a shell script, a small binary -- and every
/// language on every platform can write a line of JSON. A framing header would
/// buy nothing and cost the author a library.
pub fn parse(line: &str) -> Result<FromExtension, serde_json::Error> {
    serde_json::from_str(line)
}

/// Render one line to send to an extension.
///
/// Newline-terminated here rather than by the caller, so that a message cannot
/// be written without its terminator and leave the other side waiting forever
/// for a line that has already been sent.
pub fn line(message: &ToExtension) -> Result<String, serde_json::Error> {
    Ok(format!("{}\n", serde_json::to_string(message)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{Item, List, Section};

    #[test]
    fn a_render_carries_the_whole_tree() {
        let message = FromExtension::Render {
            view: View::List(List {
                sections: vec![Section {
                    title: Some("Open".into()),
                    items: vec![Item {
                        id: "1".into(),
                        title: "Something".into(),
                        ..Default::default()
                    }],
                }],
                ..Default::default()
            }),
        };

        let json = serde_json::to_string(&message).unwrap();
        assert_eq!(parse(&json).unwrap(), message);
    }

    /// The shape an extension author has to produce by hand, so it is worth
    /// pinning: anything harder than this and nobody writes the second one.
    #[test]
    fn a_render_is_writable_by_hand() {
        let message = parse(
            r#"{"type":"render","type":"list","sections":[{"items":[{"id":"a","title":"A"}]}]}"#,
        );
        // Two `type` keys is *not* writable by hand, which is exactly why the
        // view is nested rather than flattened into the envelope.
        assert!(message.is_err());

        let message = parse(
            r#"{"type":"render","view":{"type":"list","sections":[{"items":[{"id":"a","title":"A"}]}]}}"#,
        );
        assert!(message.is_ok(), "{message:?}");
    }

    /// The bug that writing the first extension found, pinned in both
    /// directions so it cannot come back.
    ///
    /// Nesting is not decoration. An internally tagged enum inside another one
    /// serialises to an object with the same key twice, which no parser
    /// accepts and no author can produce by hand.
    #[test]
    fn no_message_has_two_type_keys() {
        let messages = [
            ToExtension::Run {
                command: "x".into(),
            },
            ToExtension::Event {
                event: Event::Dismissed,
            },
            ToExtension::Event {
                event: Event::ActionInvoked {
                    item: Some("a".into()),
                    action: "open".into(),
                },
            },
            ToExtension::Response {
                id: 1,
                result: Answer::Ok { value: None },
            },
        ];

        for message in messages {
            let json = serde_json::to_string(&message).unwrap();
            // Round-tripping is the real assertion: a duplicated key survives
            // serialisation and fails here.
            let parsed: ToExtension = serde_json::from_str(&json)
                .unwrap_or_else(|error| panic!("{json} did not parse back: {error}"));
            assert_eq!(parsed, message);
        }
    }

    /// What an extension author actually reads, spelled out, because the whole
    /// point of nesting is that this stays obvious.
    #[test]
    fn an_event_arrives_under_a_field_an_author_can_reach() {
        let json = serde_json::to_string(&ToExtension::Event {
            event: Event::ActionInvoked {
                item: Some("tokyo".into()),
                action: "copy".into(),
            },
        })
        .unwrap();

        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["type"], "event");
        assert_eq!(value["event"]["type"], "action-invoked");
        assert_eq!(value["event"]["action"], "copy");
    }

    #[test]
    fn a_capability_request_round_trips() {
        let message = FromExtension::Request {
            id: 7,
            capability: Capability::Open {
                target: "https://example.com".into(),
            },
        };

        let json = serde_json::to_string(&message).unwrap();
        assert_eq!(parse(&json).unwrap(), message);
    }

    /// The shape an extension author writes by hand, pinned. Nesting a
    /// `capability` object inside a `capability` field is the kind of thing
    /// that reads fine in Rust and is quietly awful to produce from a shell.
    #[test]
    fn a_request_is_flat_on_the_wire() {
        let json = serde_json::to_string(&FromExtension::Request {
            id: 1,
            capability: Capability::CopyToClipboard {
                text: "18:04".into(),
            },
        })
        .unwrap();

        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["type"], "request");
        assert_eq!(value["id"], 1);
        assert_eq!(value["capability"], "copy-to-clipboard");
        assert_eq!(value["text"], "18:04");
        assert!(value["capability"].is_string(), "not nested: {json}");
    }

    /// A capability that returns nothing still has to parse from the shape an
    /// author will write, which is the tag and no other field at all.
    #[test]
    fn a_request_with_no_arguments_needs_no_arguments() {
        let message = parse(r#"{"type":"request","id":2,"capability":"read-clipboard"}"#).unwrap();

        assert_eq!(
            message,
            FromExtension::Request {
                id: 2,
                capability: Capability::ReadClipboard,
            }
        );
    }

    #[test]
    fn an_answer_with_nothing_to_return_omits_the_value() {
        let json = serde_json::to_string(&Answer::Ok { value: None }).unwrap();
        assert_eq!(json, r#"{"status":"ok"}"#);
    }

    #[test]
    fn every_message_to_an_extension_ends_in_a_newline() {
        let message = ToExtension::Run {
            command: "prs".into(),
        };
        let line = line(&message).unwrap();

        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
    }

    /// An extension that sends nonsense must produce an error the host can
    /// report, not a panic and not a silent nothing.
    #[test]
    fn a_malformed_line_is_an_error() {
        assert!(parse("{ not json").is_err());
        assert!(parse(r#"{"type":"explode"}"#).is_err());
    }
}
