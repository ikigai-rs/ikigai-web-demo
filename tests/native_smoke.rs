//! The native smoke (ledger #186): start the real `ikigai-net-server` binary on loopback
//! and resolve through it over WebTransport, the way the network demo page does.
//!
//! Why the BINARY, when `tests/conformance.rs` already walks the same kernel in process:
//! two native-only panics (a `js_sys` clock and a `spawn_local` fetch, both fixed in
//! PR 57) sat in this server unhit because nothing ever asked a native kernel for the time
//! or a granted fetch, and the first `Expiry::At` would have taken the server task down.
//! A walk of `build_kernel_in` cannot see what `main` wires (its clock, its transport, its
//! bind), and only a reply that crossed the wire proves the server answered.
//!
//! It resolves `urn:time:now` (computed, then again within the minute, when the cached
//! answer's `Expiry::At` is judged against the server's clock), and one granted
//! `urn:httpGet` against a loopback stub, which only a server with a native transport
//! installed can answer. Every step is bounded: a server that never prints, never binds or
//! never answers fails here, loudly, with what it did print.

mod common;

use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use common::Stub;
use ikigai_core::{ArgRef, Capability, Iri, Request, Verb};
use ikigai_wire::{decode, encode, Call, Reply};
use tokio::io::AsyncReadExt;
use wtransport::tls::Sha256Digest;
use wtransport::{ClientConfig, Endpoint};

/// The whole smoke's bound, start to last reply. A cold CI runner starts the binary in well
/// under a second; this is the "it did not answer" line, not a performance budget.
const BOUND: Duration = Duration::from_secs(60);

/// The server process, killed on drop so a failed assertion never leaks it.
struct Server {
    child: Child,
    addr: SocketAddr,
    cert: Sha256Digest,
    _jail: tempfile::TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Start the binary on an ephemeral loopback port over a scratch jail, and wait for it to
/// report the cert hash and the BOUND address (printed only after the socket binds).
fn start() -> Server {
    let jail = tempfile::tempdir().expect("tempdir");
    let mut child = Command::new(env!("CARGO_BIN_EXE_ikigai-net-server"))
        .arg("0")
        .arg("--root")
        .arg(jail.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn ikigai-net-server");
    let stdout = child.stdout.take().expect("stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut printed = Vec::new();
    let mut cert = None;
    let deadline = std::time::Instant::now() + BOUND;
    let addr = loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let line = match rx.recv_timeout(left) {
            Ok(line) => line,
            Err(e) => {
                let status = child.try_wait().ok().flatten();
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "ikigai-net-server never reported a bound address ({e}; exit {status:?}); \
                     it printed:\n{}",
                    printed.join("\n")
                );
            }
        };
        if let Some(hex) = line.strip_prefix("cert sha-256: ") {
            cert = Some(digest(hex));
        }
        if let Some(url) = line.strip_prefix("listening on https://") {
            break url.parse::<SocketAddr>().expect("a socket address");
        }
        printed.push(line);
    };
    assert!(
        addr.ip().is_loopback(),
        "the default bind is loopback: {addr}"
    );
    assert_ne!(
        addr.port(),
        0,
        "the bound port is reported, not the requested 0"
    );
    Server {
        child,
        addr,
        cert: cert.expect("the cert hash is printed before the server listens"),
        _jail: jail,
    }
}

/// The server prints its cert hash as 64 bare hex digits (what the page's `#cert=` takes),
/// which neither of `Sha256Digest`'s string forms reads.
fn digest(hex: &str) -> Sha256Digest {
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
        .collect();
    Sha256Digest::new(bytes.try_into().expect("a 32-byte sha-256 digest"))
}

/// One `Call` on its own bidirectional stream, as the page's codec sends it.
async fn call(connection: &wtransport::Connection, call: &Call) -> Reply {
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .expect("open a stream")
        .await
        .expect("the stream opens");
    send.write_all(&encode(call).expect("encode the call"))
        .await
        .expect("send the call");
    send.finish().await.expect("finish the call");
    let mut bytes = Vec::new();
    recv.read_to_end(&mut bytes)
        .await
        .expect("the server answers on the stream");
    assert!(
        !bytes.is_empty(),
        "the server closed the stream without a reply to {call:?} (a server panic, if that \
         is what happened, is printed above)"
    );
    decode::<Reply>(&bytes).expect("a decodable reply")
}

fn request(target: &str, args: &[(&str, &str)]) -> Request {
    let mut request = Request::new(Verb::Source, Iri::parse(target).expect("iri"));
    for (name, value) in args {
        request = request.with_arg(*name, ArgRef::Inline(value.as_bytes().to_vec()));
    }
    request
}

fn text(reply: Reply, what: &str) -> String {
    match reply {
        Reply::Resolved(repr, _) => String::from_utf8(repr.bytes).expect("UTF-8"),
        other => panic!("{what}: the server did not resolve it: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_native_server_resolves_the_clock_and_a_granted_fetch() {
    let server = start();
    let stub = Stub::start();
    let addr = server.addr;
    let cert = server.cert.clone();
    let smoke = async move {
        let client = Endpoint::client(
            ClientConfig::builder()
                .with_bind_default()
                .with_server_certificate_hashes([cert])
                .build(),
        )
        .expect("client");
        let connection = client
            .connect(format!("https://{addr}/"))
            .await
            .expect("a WebTransport session with the server");

        // The clock: computed, then again — the second read judges the cached answer's
        // `Expiry::At` against the server's clock, the path the js_sys clock panicked on.
        for attempt in ["computed", "within the minute"] {
            let now = text(
                call(&connection, &Call::Issue(request("urn:time:now", &[]))).await,
                "urn:time:now",
            );
            let digits = now.chars().filter(char::is_ascii_digit).count();
            assert!(
                now.len() == 5 && &now[2..3] == ":" && digits == 4,
                "urn:time:now ({attempt}) is HH:MM, got {now:?}"
            );
        }

        // A granted fetch: only a server with a native transport installed answers it.
        let fetched = text(
            call(
                &connection,
                &Call::IssueAs(
                    request("urn:httpGet", &[("url", &stub.url("/ok"))]),
                    stub.grant(&["/ok"]),
                ),
            )
            .await,
            "a granted urn:httpGet",
        );
        assert_eq!(fetched, "stub ok");

        // And one outside the grant is refused before it reaches the stub.
        match call(
            &connection,
            &Call::IssueAs(
                request("urn:httpGet", &[("url", &stub.url("/elsewhere"))]),
                Capability::scoped([format!("urn:cap:net:127.0.0.1:{}/ok", stub.port)]),
            ),
        )
        .await
        {
            Reply::ErrorTyped(ikigai_wire::WireError::Denied(_)) => {}
            other => panic!("a fetch outside the grant is a typed denial, got {other:?}"),
        }
        assert_eq!(
            stub.hits(),
            vec!["/ok"],
            "only the granted path was fetched"
        );
    };
    tokio::time::timeout(BOUND, smoke)
        .await
        .expect("the native server answered within the bound");
    drop(server);
}
