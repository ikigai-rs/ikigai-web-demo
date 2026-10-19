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
//! `urn:http:get` against a loopback stub, which only a server with a native transport
//! installed can answer. Every step is bounded: a server that never prints, never binds or
//! never answers fails here, loudly, with what it did print.
//!
//! The second test is the converse: an egress the server must NOT make. A shapes graph sent
//! to `urn:shacl:validate` carrying a SPARQL `SERVICE` reached a loopback stub through
//! rudof's HTTP client until ikigai-shacl 0.3.2 (ledger #1101, #1099); it now reaches it zero
//! times and is refused, typed.

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

/// Wire v9 through the real binary (ledger #1546): a peer that carries its run context
/// (`Call::IssueWith`, variant 5) is answered as the `IssueAs` it extends, with the same
/// bytes a plain `Issue` of the same request gets, and a recording peer gets its spans.
/// The page never sends this call (see `tests/wire_codec.rs`); another kernel resolving
/// through the demo does. On main before this arc the server did not decode the variant.
#[tokio::test(flavor = "multi_thread")]
async fn the_native_server_answers_a_wire_v9_issue_with() {
    use ikigai_wire::{IssueWith, TraceContext};

    let server = start();
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
        let upper = || request("urn:iki:fn:toUpper", &[("in", "hi")]);
        let with = IssueWith {
            trace: None,
            retries: Some(1),
            time_left_ms: Some(30_000),
        };

        let plain = text(
            call(&connection, &Call::Issue(upper())).await,
            "a plain Issue",
        );
        let carried = text(
            call(
                &connection,
                &Call::IssueWith(upper(), Capability::root(), with),
            )
            .await,
            "an IssueWith carrying a run context",
        );
        assert_eq!(plain, "HI");
        assert_eq!(
            carried, plain,
            "the run context changes nothing about the answer"
        );

        let recording = IssueWith {
            trace: Some(TraceContext {
                trace_id: 1,
                parent_span: None,
            }),
            ..with
        };
        match call(
            &connection,
            &Call::IssueWith(upper(), Capability::root(), recording),
        )
        .await
        {
            Reply::ResolvedTraced(repr, _, events) => {
                assert_eq!(String::from_utf8(repr.bytes).expect("UTF-8"), "HI");
                assert!(
                    events.iter().any(|e| e.target == "urn:iki:fn:toUpper"),
                    "a recording peer gets the spans back: {events:?}"
                );
            }
            other => panic!("a recording IssueWith answers ResolvedTraced, got {other:?}"),
        }
    };
    tokio::time::timeout(BOUND, smoke)
        .await
        .expect("the native server answered within the bound");
    drop(server);
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
                    request("urn:http:get", &[("url", &stub.url("/ok"))]),
                    stub.grant(&["/ok"]),
                ),
            )
            .await,
            "a granted urn:http:get",
        );
        assert_eq!(fetched, "stub ok");

        // And one outside the grant is refused before it reaches the stub.
        match call(
            &connection,
            &Call::IssueAs(
                request("urn:http:get", &[("url", &stub.url("/elsewhere"))]),
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

/// A plain-HTTP stub that counts the connections it is sent, answering each with an empty
/// SPARQL result set so a client that does reach it finishes. A connection that opens with
/// [`SENTINEL`] is [`SparqlStub::connections`]' own and is not counted: accepts are served in
/// backlog order, so once it is answered every earlier connection has been counted, and the
/// "zero" below needs no sleep and cannot race. (The pattern is ikigai-shacl's
/// `tests/service_egress.rs`.)
struct SparqlStub {
    addr: SocketAddr,
    seen: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

const SENTINEL: &[u8] = b"SENTINEL\r\n";

impl SparqlStub {
    fn start() -> SparqlStub {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind the stub");
        let addr = listener.local_addr().expect("addr");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = std::sync::Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                // Read the head (or the sentinel), then any body, so closing never resets
                // the client mid-request.
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while stream.read(&mut byte).map(|n| n == 1).unwrap_or(false) {
                    head.push(byte[0]);
                    if head == SENTINEL || head.len() > 64 * 1024 {
                        break;
                    }
                    if head.ends_with(b"\r\n\r\n") {
                        let text = String::from_utf8_lossy(&head).to_ascii_lowercase();
                        let length = text
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|n| n.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        let mut body = vec![0u8; length];
                        let _ = stream.read_exact(&mut body);
                        break;
                    }
                }
                if head == SENTINEL {
                    let _ = stream.write_all(b"ok");
                    continue;
                }
                let line = String::from_utf8_lossy(&head);
                record
                    .lock()
                    .expect("log")
                    .push(line.lines().next().unwrap_or("").to_string());
                let body = r#"{"head":{"vars":["s"]},"results":{"bindings":[]}}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/sparql-results+json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        SparqlStub { addr, seen }
    }

    fn url(&self) -> String {
        format!("http://{}/sparql", self.addr)
    }

    /// The request line of every connection the stub has been sent, after a sentinel round
    /// trip.
    fn connections(&self) -> Vec<String> {
        use std::io::{Read, Write};
        let mut sentinel = std::net::TcpStream::connect(self.addr).expect("sentinel connect");
        sentinel.write_all(SENTINEL).expect("sentinel write");
        let mut ack = Vec::new();
        sentinel.read_to_end(&mut ack).expect("sentinel read");
        assert_eq!(ack, b"ok", "the stub did not answer its sentinel");
        self.seen.lock().expect("log").clone()
    }
}

const SHACL_HEAD: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .\n\
                          @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
                          @prefix ex: <http://example.org/> .\n";

const SHACL_DATA: &str = "@prefix ex: <http://example.org/> .\nex:a a ex:Person ; ex:p ex:b .\n";

/// `text` with every character a SPARQL `IRIREF` forbids written as a Turtle `\uXXXX` escape,
/// so it can sit inside `<…>` in a Turtle document (rudof's lenient reader keeps them).
fn escaped_iri(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '<' | '>' | '"' | '{' | '}' | '|' | '^' | '`' | '\\' | '\0'..=' ' => {
                format!("\\u{:04X}", c as u32)
            }
            c => c.to_string(),
        })
        .collect()
}

/// The SHACL-SPARQL egress (ledger #1101, #1099): this server links `ikigai-shacl` natively,
/// rudof evaluates a shape's `sh:sparql` with oxigraph's HTTP client compiled in, and
/// `urn:shacl:validate` declares no capability, so a `SERVICE` in a shapes graph (or smuggled
/// in through a data IRI rudof splices into `VALUES`) made THIS server connect out with no
/// `urn:cap:net:*` in sight. Under the root capability the server resolves every call as,
/// each probe must reach the stub zero times and come back a typed refusal naming the
/// argument it arrived in. Before ikigai-shacl 0.3.2 the `select` probe connected and
/// answered a report.
#[tokio::test(flavor = "multi_thread")]
async fn the_native_server_never_lets_a_shapes_graph_reach_the_network() {
    let server = start();
    let stub = SparqlStub::start();
    let addr = server.addr;
    let cert = server.cert.clone();
    let url = stub.url();
    let service = format!("SERVICE <{url}> {{ ?s ?p ?o }}");
    let breakout = escaped_iri(&format!(
        "http://example.org/x> }} {service} VALUES ?q {{ <http://example.org/y"
    ));
    let select = |query: &str| {
        format!(
            "{SHACL_HEAD}ex:S a sh:NodeShape ; sh:targetClass ex:Person ;\n  \
             sh:sparql [ sh:select \"\"\"{query}\"\"\" ] .\n"
        )
    };
    // (name, data, shapes, the argument the refusal names)
    let probes: Vec<(&str, String, String, &str)> = vec![
        // The claim's own shape: a node shape's `sh:select` with a SERVICE.
        (
            "select",
            SHACL_DATA.into(),
            select(&format!("SELECT $this WHERE {{ $this a ?t . {service} }}")),
            "shapes",
        ),
        // A `sh:declare` prefix NAME carrying the whole query; no `sh:select` names SERVICE.
        (
            "prefix-name",
            SHACL_DATA.into(),
            format!(
                "{SHACL_HEAD}ex:S a sh:NodeShape ; sh:targetClass ex:Person ;\n  \
                 sh:sparql [ sh:prefixes ex:decls ; sh:select \"# nothing\" ] .\n\
                 ex:decls sh:declare [ sh:prefix \"\"\"a: <http://example.org/a/> SELECT $this WHERE {{ {service} }} #\"\"\" ;\n  \
                 sh:namespace \"http://example.org/b/\"^^xsd:anyURI ] .\n"
            ),
            "shapes",
        ),
        // From the DATA graph: a literal's datatype IRI, which rudof's reader does not
        // re-validate and its evaluator writes into `VALUES ?this { … }` unescaped.
        (
            "data-literal-datatype",
            format!("@prefix ex: <http://example.org/> .\nex:a ex:p \"v\"^^<{breakout}> .\n"),
            format!(
                "{SHACL_HEAD}ex:S a sh:NodeShape ; sh:targetObjectsOf ex:p ;\n  \
                 sh:sparql [ sh:select \"\"\"SELECT $this WHERE {{ OPTIONAL {{ ?s ?p $this }} }}\"\"\" ] .\n"
            ),
            "data",
        ),
    ];
    let probe = async move {
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
        let mut wrong = Vec::new();
        for (name, data, shapes, arg) in probes {
            let before = stub.connections().len();
            let reply = call(
                &connection,
                &Call::Issue(request(
                    "urn:shacl:validate",
                    &[("data", data.as_str()), ("shapes", shapes.as_str())],
                )),
            )
            .await;
            let reached = stub.connections().len() - before;
            if reached != 0 {
                wrong.push(format!(
                    "{name}: the server reached the stub {reached} time(s)"
                ));
            }
            match &reply {
                Reply::ErrorTyped(ikigai_wire::WireError::InvalidArgument {
                    name: got,
                    detail,
                }) if got == arg && detail.contains("SERVICE") => {}
                other => wrong.push(format!(
                    "{name}: expected a refusal naming `{arg}`, got {}",
                    match other {
                        Reply::Resolved(repr, _) =>
                            format!("a report: {}", String::from_utf8_lossy(&repr.bytes)),
                        other => format!("{other:?}"),
                    }
                )),
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    };
    tokio::time::timeout(BOUND, probe)
        .await
        .expect("the native server answered within the bound");
    drop(server);
}

/// A deep query in a level of two shapes (ledger #1121): rudof validates each topological
/// level of shapes with rayon's `par_iter_mut`, and a level of two or more is split onto
/// rayon's global pool, whose workers have the default 2 MiB stack rather than the thread
/// `urn:shacl:validate` sizes for the longest query. Through ikigai-shacl 0.3.2 a flat `||`
/// chain in a `sh:select`, beside a second targeted shape, overflowed a rayon worker and
/// ABORTED this server: the whole process, every connection, from one call that holds no
/// capability at all. 0.3.3 validates on a one-thread pool of the sized stack, so the same
/// call comes back a report. (The chain is the size ikigai-shacl's own
/// `tests/rudof_traps.rs` uses; 3,000 terms already aborted a debug build.)
#[tokio::test(flavor = "multi_thread")]
async fn a_deep_query_in_a_two_shape_level_does_not_abort_the_native_server() {
    let mut server = start();
    let addr = server.addr;
    let cert = server.cert.clone();
    let select = format!(
        "SELECT $this WHERE {{ FILTER(false{}) }}",
        "||false".repeat(30_000)
    );
    let shapes = format!(
        "{SHACL_HEAD}ex:S a sh:NodeShape ; sh:targetClass ex:Person ;\n  \
         sh:sparql [ sh:select \"\"\"{select}\"\"\" ] .\n\
         ex:T a sh:NodeShape ; sh:targetClass ex:Person ; sh:nodeKind sh:IRI .\n"
    );
    let probe = async move {
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
        let call = Call::Issue(request(
            "urn:shacl:validate",
            &[("data", SHACL_DATA), ("shapes", shapes.as_str())],
        ));
        // Not `call()`: a dead server is the outcome under test, so it is a value here,
        // reported with the server's exit status below rather than as a stream panic.
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
        send.write_all(&encode(&call).expect("encode the call"))
            .await
            .map_err(|e| e.to_string())?;
        send.finish().await.map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        recv.read_to_end(&mut bytes)
            .await
            .map_err(|e| e.to_string())?;
        decode::<Reply>(&bytes).map_err(|e| format!("{e} ({} bytes)", bytes.len()))
    };
    let outcome = tokio::time::timeout(BOUND, probe)
        .await
        .expect("the native server answered within the bound");
    let reply = outcome.unwrap_or_else(|e| {
        // Poll for the exit rather than one `try_wait`: a stream that just died races the
        // process reporting its exit (it reads `None` for a server that is about to say
        // SIGABRT), and a bare `wait` would hang on a server that is alive.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let status = loop {
            match server.child.try_wait() {
                Ok(Some(status)) => break format!("it exited {status}"),
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                Ok(None) => break "it is still running".to_string(),
                Err(e) => break format!("its status is unreadable: {e}"),
            }
        };
        panic!("the server did not answer ({e}); {status}")
    });
    let report = text(reply, "a two-shape level with a deep query");
    assert!(
        server.child.try_wait().expect("try_wait").is_none(),
        "the server is still running after the deep query"
    );
    assert!(!report.is_empty(), "a validation report: {report:?}");
    drop(server);
}
