//! The native HTTP transport the WebTransport server installs (ledger #184), against a
//! stub on 127.0.0.1. No test here reaches a real host.
//!
//! `ikigai-http`'s contract is that a transport returns a 3xx rather than following it:
//! the ENDPOINT follows, re-running the `urn:cap:net:*` ACL against every hop, so a granted
//! host cannot 302 a request to an ungranted one. The stub records every path it is asked
//! for, and that log is the witness: a transport that followed on its own would show the
//! redirect target in it.

mod common;

use std::sync::Arc;

use common::Stub;
use ikigai_core::{ArgRef, Capability, Error, Iri, Kernel, Request, Verb};
use ikigai_http::{HttpRequest, HttpTransport, Method};
use ikigai_web_demo::{build_kernel_with_transport, NativeFetchTransport};

/// The server's kernel, as `ikigai-net-server` builds it, over a scratch jail.
fn server_kernel() -> (tempfile::TempDir, Kernel) {
    let jail = tempfile::tempdir().expect("tempdir");
    let kernel = build_kernel_with_transport(
        "Remote (native fetch test)",
        jail.path(),
        Arc::new(NativeFetchTransport::default()),
    );
    (jail, kernel)
}

fn get(kernel: &Kernel, url: &str, capability: &Capability) -> Result<String, Error> {
    let request = Request::new(Verb::Source, Iri::parse("urn:httpGet").expect("iri"))
        .with_arg("url", ArgRef::Inline(url.as_bytes().to_vec()));
    futures::executor::block_on(kernel.issue(request, capability))
        .map(|repr| String::from_utf8(repr.bytes).expect("UTF-8"))
}

#[test]
fn the_transport_returns_a_redirect_rather_than_following_it() {
    let stub = Stub::start();
    let response = futures::executor::block_on(NativeFetchTransport::default().send(HttpRequest {
        method: Method::Get,
        url: stub.url("/hop"),
        headers: vec![],
        body: vec![],
    }))
    .expect("the stub answers");
    assert_eq!(response.status, 302);
    assert_eq!(response.header("location"), Some("/elsewhere"));
    assert_eq!(stub.hits(), vec!["/hop"], "the redirect was not followed");
}

#[test]
fn a_granted_fetch_reaches_the_stub() {
    let stub = Stub::start();
    let (_jail, kernel) = server_kernel();
    let body = get(&kernel, &stub.url("/ok"), &stub.grant(&["/ok"])).expect("granted");
    assert_eq!(body, "stub ok");
    assert_eq!(stub.hits(), vec!["/ok"]);
}

#[test]
fn a_fetch_outside_the_grant_is_refused_before_the_network() {
    let stub = Stub::start();
    let (_jail, kernel) = server_kernel();
    match get(&kernel, &stub.url("/elsewhere"), &stub.grant(&["/ok"])) {
        Err(Error::Denied(_)) => {}
        other => panic!("a path outside the grant is Denied, got {other:?}"),
    }
    // Another port on the same host is outside a port-scoped grant too.
    let other_port = format!("http://127.0.0.1:{}/ok", stub.port.wrapping_add(1).max(1));
    match get(&kernel, &other_port, &stub.grant(&["/ok"])) {
        Err(Error::Denied(_)) => {}
        other => panic!("another port is Denied, got {other:?}"),
    }
    assert!(
        stub.hits().is_empty(),
        "nothing was sent: {:?}",
        stub.hits()
    );
}

#[test]
fn a_redirect_to_an_ungranted_path_is_denied_at_the_hop_and_never_fetched() {
    let stub = Stub::start();
    let (_jail, kernel) = server_kernel();
    match get(&kernel, &stub.url("/hop"), &stub.grant(&["/hop"])) {
        Err(Error::Denied(e)) => assert!(e.contains("redirect target"), "{e}"),
        other => panic!("the hop to an ungranted path is Denied, got {other:?}"),
    }
    assert_eq!(
        stub.hits(),
        vec!["/hop"],
        "the transport returned the 302; the endpoint refused the hop before sending it"
    );
}

#[test]
fn a_redirect_inside_the_grant_is_followed_by_the_endpoint() {
    let stub = Stub::start();
    let (_jail, kernel) = server_kernel();
    let body = get(
        &kernel,
        &stub.url("/hop"),
        &stub.grant(&["/hop", "/elsewhere"]),
    )
    .expect("both hops granted");
    assert_eq!(body, "followed");
    assert_eq!(stub.hits(), vec!["/hop", "/elsewhere"]);
}
