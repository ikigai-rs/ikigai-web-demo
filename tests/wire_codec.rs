//! The network demo's wire codec, run natively (the lib is also an rlib): what `net.html`
//! sends and what it shows (ledger #1546, wire v9).
//!
//! The page does not host the kernel: `encodeIssue` and its siblings turn a terminal line
//! into `ikigai-wire` bytes, and `decodeReply` turns the server's `Reply` into the
//! `{ kind, text, cache }` the terminal prints. Two things hold the demo together across
//! wire versions, and both are pinned here rather than in the smoke suite because the
//! smoke runs no server (Pages is static):
//!
//! - **What the page sends predates v7.** `Issue`, `IsCached` and `Entries` are the
//!   wire's first three variants, so a v8 server, and any older one back to the codec's
//!   first release, decodes every call this page makes: the demo still works against a v8
//!   server.
//! - **What the page shows is core's display.** A typed failure crosses as a `WireError`
//!   and is rendered as the `ikigai_core::Error` it mirrors, so the five classes wire v9
//!   added (`DepthExceeded`, `RetryAfter`, `OverBudget`, `Refused`, `Unsupported`) read in
//!   the page exactly as the shared failure-class vectors say
//!   (`ikigai-vocab` 0.1.108, `tests/vectors/failure_class_vectors.json`, the `display`
//!   column; each case below names its row). On main before wire 0.1.66 those five tags
//!   did not decode at all, and the terminal printed `decode failed: …`.

use ikigai_core::Verb;
use ikigai_web_demo::{
    decode_reply, encode_compose_call, encode_entries, encode_is_cached, encode_issue,
};
use ikigai_wire::{decode, encode, Call, Reply, WireError};

/// What the terminal prints for `reply`: `(kind, text)`.
fn shown(reply: Reply) -> (String, String) {
    let json = decode_reply(encode(&reply).expect("encode the reply"));
    let v: serde_json::Value = serde_json::from_str(&json).expect("decodeReply answers JSON");
    let field = |name: &str| v[name].as_str().unwrap_or_default().to_string();
    (field("kind"), field("text"))
}

#[test]
fn the_page_shows_every_wire_v9_failure_class_as_the_vectors_display_it() {
    let cases: [(WireError, &str); 7] = [
        // depth-exceeded
        (
            WireError::DepthExceeded {
                depth: 65,
                target: "urn:x:self".into(),
            },
            "nesting budget exceeded: sub-request for urn:x:self would run at depth 65",
        ),
        // retry-after/seconds
        (
            WireError::RetryAfter {
                delay_ms: Some(30_000),
                detail: "rate-limited: 5 requests per minute".into(),
            },
            "retry after 30000 ms: rate-limited: 5 requests per minute",
        ),
        // retry-after/no-delay
        (
            WireError::RetryAfter {
                delay_ms: None,
                detail: "not now".into(),
            },
            "retry later: not now",
        ),
        // over-budget
        (
            WireError::OverBudget("the query ran past its 5000 ms budget".into()),
            "over budget: the query ran past its 5000 ms budget",
        ),
        // refused
        (
            WireError::Refused("api.example.com answered 401".into()),
            "refused: api.example.com answered 401",
        ),
        // unsupported/a-verb
        (
            WireError::Unsupported {
                verb: Some(Verb::Sink),
                detail: "the log is append-only".into(),
            },
            "unsupported: the log is append-only",
        ),
        // unsupported/a-host-facility
        (
            WireError::Unsupported {
                verb: None,
                detail: "the host configured no clock".into(),
            },
            "unsupported: the host configured no clock",
        ),
    ];
    for (error, display) in cases {
        let (kind, text) = shown(Reply::ErrorTyped(error.clone()));
        assert_eq!(kind, "error", "{error:?}");
        assert_eq!(text, display, "{error:?}");
    }
}

#[test]
fn the_page_still_shows_what_a_v8_server_sends() {
    // The v7 taxonomy and v8's `Conflict`, as core displays them (the same vectors), and
    // the flat pre-v7 string, kept decodable forever.
    let typed: [(WireError, &str); 4] = [
        (
            WireError::Denied("`urn:cap:fs:read` is required".into()),
            "denied: `urn:cap:fs:read` is required",
        ),
        (
            WireError::NotFound("no such item".into()),
            "not found: no such item",
        ),
        (
            WireError::Timeout("the origin did not answer in 30 s".into()),
            "timeout: the origin did not answer in 30 s",
        ),
        (
            WireError::Conflict("1,1 is taken".into()),
            "conflict: 1,1 is taken",
        ),
    ];
    for (error, display) in typed {
        assert_eq!(
            shown(Reply::ErrorTyped(error.clone())),
            ("error".to_string(), display.to_string()),
            "{error:?}"
        );
    }
    assert_eq!(
        shown(Reply::Error("boom".into())),
        ("error".to_string(), "boom".to_string())
    );
}

#[test]
fn every_call_the_page_sends_is_one_a_v8_server_decodes() {
    // Variants 0, 1 and 2 of `Call`: the codec's first release had them, so no server this
    // page can reach is too old for them. `IssueAs`, `IssueTraced` and the v9 `IssueWith`
    // are the server's to receive from OTHER kernels; the page sends none of them.
    let sent = [
        encode_compose_call("urn:data:page".into()),
        encode_issue("urn:iki:fn:toUpper".into(), r#"{"in":"hi"}"#.into()),
        encode_is_cached("urn:iki:fn:toUpper".into(), r#"{"in":"hi"}"#.into()),
        encode_entries(),
    ];
    for bytes in sent {
        assert!(!bytes.is_empty(), "the codec encoded the call");
        let call = decode::<Call>(&bytes).expect("the page's call decodes");
        assert!(
            matches!(call, Call::Issue(_) | Call::IsCached(_) | Call::Entries),
            "a pre-v7 variant: {call:?}"
        );
        // The discriminant is the first byte (postcard, a varint); 0, 1 or 2.
        assert!(bytes[0] <= 2, "variant {} in {bytes:?}", bytes[0]);
    }
}
