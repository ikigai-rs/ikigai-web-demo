//! A WebTransport server for the *network* demo.
//!
//! Runs the same kernel the in-browser demo composes (`ikigai_web_demo::build_kernel`),
//! but answers over the network instead of in memory. The browser opens a
//! WebTransport (HTTP/3 over QUIC) connection, sends an `ikigai-wire` `Call` on a
//! bidirectional stream, and gets a `Reply` back — the exact same protocol
//! `ikigai-ipc` and `ikigai-quic` speak. The recursive `compose` happens here,
//! server-side; the browser just renders the assembled HTML.
//!
//! TLS is a self-signed cert; the browser trusts it via WebTransport's
//! `serverCertificateHashes` (no CA). The server prints the hash to paste/pass
//! to the page.
//!
//! ```text
//! ikigai-net-server [PORT] [--root DIR] [--bind ADDR] [--allow-origin ORIGIN]…
//! ```
//!
//! `PORT` defaults to 4433. The server binds **loopback** (`127.0.0.1`) unless `--bind`
//! names another address, and says so at start: it authenticates no client and resolves
//! every call as root, so a wider bind hands root over the file jail, and over the server's
//! outbound fetch, to anyone who can reach the port with a QUIC client (ledger #1089). It
//! used to bind every interface.
//! A browser session must also come from an allowed `Origin` (`--allow-origin`, repeatable,
//! `*` for any; by default the origins the bundled pages are served from, listed in
//! [`DEFAULT_ORIGINS`]); one from any other origin is refused with 403 at the handshake.
//! That is defense in depth against a page on another site, not authentication: a
//! non-browser client writes whatever `Origin` it likes, or none, and a session with no
//! `Origin` is admitted for that reason.
//!
//! The file jail: `--root` names the file module's jail (`urn:file:*`); without
//! it the jail is `<data home>/web-demo/ws` (`~/.ikigai/web-demo/ws`). Either way the
//! server resolves the jail to an ABSOLUTE, canonical path once, at startup, creates it if
//! it is missing, and prints it. It used to be the relative `ws`, so the files a client
//! read and wrote depended on the directory the server happened to be launched from
//! (ledger #183). A path-ACL scope sent with a capability is spelled against that printed
//! path (`urn:cap:fs:read:<root>/…`), the spelling `ikigai-fs` documents.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ikigai_core::{Capability, Clock, Kernel, SystemClock};
use ikigai_resolve::{Resolver, SpanCollector};
use ikigai_wire::{decode, encode, Call, Reply, WireError};
use tokio::io::AsyncReadExt;
use wtransport::endpoint::endpoint_side::Server;
use wtransport::endpoint::IncomingSession;
use wtransport::{Endpoint, Identity, ServerConfig};

/// Largest `Call` we'll read off a stream — a guard against a runaway client.
const MAX_CALL: usize = 8 * 1024 * 1024;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(Some(args)) => args,
        Ok(None) => {
            println!("{USAGE}");
            return Ok(());
        }
        Err(e) => {
            eprintln!("ikigai-net-server: {e}\n{USAGE}");
            std::process::exit(2);
        }
    };
    let addr = SocketAddr::new(args.bind, args.port);
    let origins = Arc::new(OriginPolicy::from_flags(args.origins));
    // The jail, made absolute and canonical ONCE, here: the endpoint opens its root on
    // every request, so a relative root would follow the process's working directory.
    let root = jail_root(
        args.root,
        ikigai_core::config::data_home(),
        &std::env::current_dir()?,
    )?;
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("cannot create the file jail {}: {e}", root.display()))?;
    let root = std::fs::canonicalize(&root)?;

    // Self-signed cert valid for localhost (and a named bind address); the browser pins
    // its SHA-256.
    let mut sans = vec!["localhost".to_string(), "127.0.0.1".into(), "::1".into()];
    if !args.bind.is_unspecified() && !args.bind.is_loopback() {
        sans.push(args.bind.to_string());
    }
    let identity = Identity::self_signed(sans)?;
    let cert_hash = identity.certificate_chain().as_slice()[0].hash();
    let hash_hex: String = cert_hash
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    println!("ikigai WebTransport server  →  https://{addr}");
    if args.bind.is_loopback() {
        println!("bound to {addr} (loopback only; --bind ADDR to widen)");
    } else {
        println!(
            "⚠ bound to {addr}, BEYOND LOOPBACK: this server authenticates no client and \
             resolves every call as root, so anyone who reaches this address with a QUIC \
             client can read and write its file jail and make it fetch any URL"
        );
    }
    println!("allowed origins: {origins}");
    println!("cert sha-256: {hash_hex}");
    println!("open the network demo page with  #cert={hash_hex}  in the URL");
    println!("file jail (urn:file:*): {}", root.display());

    // The one host here that reaches the network: a granted `urn:http*` fetches through
    // a native transport that returns a redirect rather than following it, so the HTTP
    // endpoint re-runs the `urn:cap:net:*` ACL on every hop (ledger #184). Without it every
    // fetch answered "browser fetch is wasm-only".
    let kernel = Arc::new(ikigai_web_demo::build_kernel_with_transport(
        "Remote (WebTransport)",
        root,
        Arc::new(ikigai_web_demo::NativeFetchTransport::default()),
    ));

    let server = endpoint(addr, identity)?;
    // Printed only once the socket is bound, so a supervisor (or the native smoke,
    // tests/native_smoke.rs) can wait for it instead of guessing; the address is the
    // BOUND one, which differs from `addr` when PORT is 0.
    println!("listening on https://{}", server.local_addr()?);
    accept_loop(server, kernel, origins).await;
    Ok(())
}

/// The server endpoint, bound to exactly `addr` — never the any-address default.
fn endpoint(addr: SocketAddr, identity: Identity) -> std::io::Result<Endpoint<Server>> {
    let config = ServerConfig::builder()
        .with_bind_address(addr)
        .with_identity(identity)
        .keep_alive_interval(Some(Duration::from_secs(3)))
        .build();
    Endpoint::server(config)
}

/// Accept sessions forever, each on its own task.
async fn accept_loop(server: Endpoint<Server>, kernel: Arc<Kernel>, origins: Arc<OriginPolicy>) {
    loop {
        let incoming = server.accept().await;
        let kernel = Arc::clone(&kernel);
        let origins = Arc::clone(&origins);
        tokio::spawn(async move {
            if let Err(e) = serve(incoming, kernel, &origins).await {
                eprintln!("session ended: {e}");
            }
        });
    }
}

const USAGE: &str = "usage: ikigai-net-server [PORT] [--root DIR] [--bind ADDR] \
    [--allow-origin ORIGIN]...\n  \
    PORT                 UDP port to serve WebTransport on (default 4433)\n  \
    --root DIR           the file jail for urn:file:* (default <data home>/web-demo/ws, \
    i.e. ~/.ikigai/web-demo/ws)\n  \
    --bind ADDR          the IP address to listen on (default 127.0.0.1); anything wider \
    exposes root (the file jail, outbound fetch) to whoever reaches it\n  \
    --allow-origin O     a browser origin allowed to open a session (repeatable; `*` for \
    any; default http://127.0.0.1:8087, http://localhost:8087, https://ikigai-rs.github.io)";

/// The origins the bundled pages are served from: `dist/` served locally as the README
/// and DEMO say (`python3 -m http.server 8087`), and GitHub Pages, which publishes all of
/// `dist/` including `net.html`.
const DEFAULT_ORIGINS: &[&str] = &[
    "http://127.0.0.1:8087",
    "http://localhost:8087",
    "https://ikigai-rs.github.io",
];

/// Which browser origins may open a session.
#[derive(Debug, PartialEq)]
enum OriginPolicy {
    Any,
    Only(Vec<String>),
}

impl OriginPolicy {
    /// `--allow-origin` values REPLACE the defaults; any `*` admits every origin.
    fn from_flags(flags: Vec<String>) -> Self {
        if flags.iter().any(|o| o.trim() == "*") {
            OriginPolicy::Any
        } else if flags.is_empty() {
            OriginPolicy::Only(DEFAULT_ORIGINS.iter().map(|o| normalize(o)).collect())
        } else {
            OriginPolicy::Only(flags.iter().map(|o| normalize(o)).collect())
        }
    }

    /// Whether a session whose request carries `origin` may proceed. No `Origin` at all
    /// is admitted: browsers always send one for WebTransport, so its absence means a
    /// non-browser client, which could as easily have sent an allowed value.
    fn allows(&self, origin: Option<&str>) -> bool {
        match (self, origin) {
            (OriginPolicy::Any, _) | (_, None) => true,
            (OriginPolicy::Only(list), Some(origin)) => list.contains(&normalize(origin)),
        }
    }
}

impl std::fmt::Display for OriginPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OriginPolicy::Any => f.write_str("* (any)"),
            OriginPolicy::Only(list) => f.write_str(&list.join(", ")),
        }
    }
}

/// An origin as compared: scheme and host are case-insensitive, and a trailing `/`
/// (which an `Origin` header never carries, but an operator might type) is dropped.
fn normalize(origin: &str) -> String {
    origin.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// The command line, parsed. `Ok(None)` is a request for the usage text.
#[derive(Debug, PartialEq)]
struct Args {
    port: u16,
    root: Option<PathBuf>,
    bind: IpAddr,
    origins: Vec<String>,
}

impl Args {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Args>, String> {
        let mut port = None;
        let mut root = None;
        let mut bind = None;
        let mut origins = Vec::new();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            if arg == "-h" || arg == "--help" {
                return Ok(None);
            } else if arg == "--root" {
                let dir = args.next().ok_or("--root needs a directory")?;
                root = Some(PathBuf::from(dir));
            } else if let Some(dir) = arg.strip_prefix("--root=") {
                root = Some(PathBuf::from(dir));
            } else if arg == "--bind" || arg.starts_with("--bind=") {
                let addr = match arg.strip_prefix("--bind=") {
                    Some(addr) => addr.to_string(),
                    None => args.next().ok_or("--bind needs an IP address")?,
                };
                bind = Some(
                    addr.parse()
                        .map_err(|_| format!("`{addr}` is not an IP address"))?,
                );
            } else if arg == "--allow-origin" || arg.starts_with("--allow-origin=") {
                let origin = match arg.strip_prefix("--allow-origin=") {
                    Some(origin) => origin.to_string(),
                    None => args.next().ok_or("--allow-origin needs an origin")?,
                };
                if origin.trim().is_empty() {
                    return Err("--allow-origin needs an origin".into());
                }
                origins.push(origin);
            } else if arg.starts_with('-') {
                return Err(format!("unknown option `{arg}`"));
            } else if port.is_none() {
                // Refused rather than defaulted: a typo here used to serve on 4433
                // without a word.
                port = Some(
                    arg.parse()
                        .map_err(|_| format!("`{arg}` is not a port number"))?,
                );
            } else {
                return Err(format!("unexpected argument `{arg}`"));
            }
        }
        if root.as_deref().is_some_and(|r| r.as_os_str().is_empty()) {
            return Err("--root needs a directory".into());
        }
        Ok(Some(Args {
            port: port.unwrap_or(4433),
            root,
            bind: bind.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            origins,
        }))
    }
}

/// The file jail as an absolute path: `--root` if given (a relative one is taken
/// against `cwd`, the directory the operator typed it in, and fixed from then on),
/// else `<data home>/web-demo/ws`. With neither a `--root` nor a known data home
/// there is no answer, and guessing one relative to `cwd` is the bug this replaces.
fn jail_root(
    flag: Option<PathBuf>,
    data_home: Option<PathBuf>,
    cwd: &Path,
) -> Result<PathBuf, String> {
    match flag {
        Some(dir) if dir.is_absolute() => Ok(dir),
        Some(dir) => Ok(cwd.join(dir)),
        None => data_home
            .map(|home| home.join("web-demo").join("ws"))
            .ok_or_else(|| "no data home (HOME is unset): pass --root DIR".to_string()),
    }
}

/// Accept one WebTransport session and answer `Call`s on its bidi streams until
/// the client disconnects. A session from an origin the policy does not allow is
/// refused with 403 before any `Call` is read.
async fn serve(
    incoming: IncomingSession,
    kernel: Arc<Kernel>,
    origins: &OriginPolicy,
) -> Result<(), Box<dyn std::error::Error>> {
    let request = incoming.await?;
    if !origins.allows(request.origin()) {
        eprintln!(
            "refused a session from origin {:?} (allowed: {origins})",
            request.origin().unwrap_or_default()
        );
        request.forbidden().await;
        return Ok(());
    }
    let connection = request.accept().await?;
    loop {
        let (mut send, recv) = match connection.accept_bi().await {
            Ok(stream) => stream,
            Err(_) => return Ok(()), // client closed the connection
        };
        let mut bytes = Vec::new();
        recv.take(MAX_CALL as u64).read_to_end(&mut bytes).await?;
        let reply = dispatch(&kernel, &bytes);
        send.write_all(&reply).await?;
        send.finish().await?;
    }
}

/// Decode a `Call`, resolve it against the kernel (recursive `compose` runs here),
/// and encode the `Reply`. The stream boundary frames the message.
fn dispatch(kernel: &Kernel, bytes: &[u8]) -> Vec<u8> {
    let reply = match decode::<Call>(bytes) {
        // Failures go back as `ErrorTyped`, not the flat `Error(String)`: the wire
        // taxonomy is what lets the client keep a remote `Denied` permanent and a
        // remote `Timeout` transient. Same choice ikigai-ipc/ikigai-quic make, so all
        // three transports answer a failure identically. Unlike those two, this transport
        // negotiates no wire version (a browser cannot choose an ALPN), so a failure crosses
        // at THIS crate's wire (v9 since 0.1.66: `RetryAfter`, `OverBudget`, `Refused`,
        // `Unsupported` and `DepthExceeded` typed, never downgraded with `for_peer`). The
        // page's codec is the same build of the same crate, which is what makes that safe.
        Ok(Call::Issue(request)) => match Resolver::issue(kernel, request) {
            Ok((representation, status)) => Reply::Resolved(representation, status),
            Err(e) => Reply::ErrorTyped(WireError::from(&e)),
        },
        // Capability-on-the-wire. This demo server authenticates no client principal
        // (self-signed cert, no client auth), so its effective entitlement is root and
        // the carried capability is already ≤ root — resolving under it *is* the clamp,
        // exactly as the peercred-owned IPC path does. A future authenticated principal
        // would intersect the carried capability with its entitlement here.
        Ok(Call::IssueAs(request, capability)) => {
            match Resolver::issue_as(kernel, request, &capability) {
                Ok((representation, status)) => Reply::Resolved(representation, status),
                Err(e) => Reply::ErrorTyped(WireError::from(&e)),
            }
        }
        // Trace-over-the-wire: resolve with a PER-CALL collector so concurrent traced
        // calls can't interleave, and ship the recorded spans back with the answer.
        // `_ctx.parent_span` would re-parent this subtree under a caller's span when a
        // mount stitches two kernels; a bare `--connect` trace has no parent to adopt.
        Ok(Call::IssueTraced(request, capability, _ctx)) => {
            let collector = Arc::new(SpanCollector::default());
            match ikigai_resolve::issue_traced_as(kernel, request, &capability, collector.clone()) {
                Ok((representation, status)) => {
                    Reply::ResolvedTraced(representation, status, collector.take())
                }
                Err(e) => Reply::ErrorTyped(WireError::from(&e)),
            }
        }
        // Wire v9 (ledger #1546): a peer's RUN CONTEXT rides with the request — what is
        // left of its retry pool and of its deadline, the deadline as time LEFT so two
        // clocks need not agree. The page's own codec never sends it (`encodeIssue` and
        // its siblings in src/lib.rs send `Issue`), so this is another kernel resolving
        // through the demo. The context is rebuilt at THIS server's now (`SystemClock`,
        // the clock the native kernel runs on), and the kernel narrows the retries to the
        // carried capability's entitlement, which on a kernel with no retry budget — this
        // one — is none: a peer can shorten its own run here and never lengthen it. The
        // capability is served as `IssueAs` serves it (root is the ceiling, so resolving
        // under it is the clamp), and a recording peer is answered with its spans, as
        // `IssueTraced` is.
        Ok(Call::IssueWith(request, capability, with)) => {
            let context = with.issue_context(SystemClock.now());
            match with.trace {
                Some(_ctx) => {
                    let collector = Arc::new(SpanCollector::default());
                    match ikigai_resolve::issue_traced_as_with_context(
                        kernel,
                        request,
                        &capability,
                        collector.clone(),
                        context,
                    ) {
                        Ok((representation, status)) => {
                            Reply::ResolvedTraced(representation, status, collector.take())
                        }
                        Err(e) => Reply::ErrorTyped(WireError::from(&e)),
                    }
                }
                None => match ikigai_resolve::issue_as_with_context(
                    kernel,
                    request,
                    &capability,
                    context,
                ) {
                    Ok((representation, status)) => Reply::Resolved(representation, status),
                    Err(e) => Reply::ErrorTyped(WireError::from(&e)),
                },
            }
        }
        // Trusted in-process path (like `issue` above, which resolves under root), so
        // probe the cache under the root capability.
        Ok(Call::IsCached(request)) => {
            Reply::Cached(Resolver::is_cached(kernel, &request, &Capability::root()))
        }
        Ok(Call::Entries) => Reply::Entries(Resolver::entries(kernel)),
        Err(e) => Reply::Error(format!("undecodable call: {e}")),
    };
    encode(&reply).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Option<Args>, String> {
        Args::parse(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn the_jail_is_never_relative_to_the_working_directory() {
        let cwd = Path::new("/launched/from/here");
        let home = Some(PathBuf::from("/home/b/.ikigai"));
        // No flag: the data home, wherever the server was launched.
        let default = jail_root(None, home.clone(), cwd).unwrap();
        assert_eq!(default, PathBuf::from("/home/b/.ikigai/web-demo/ws"));
        assert!(!default.starts_with(cwd));
        // An absolute flag is taken as given.
        assert_eq!(
            jail_root(Some("/srv/ws".into()), home.clone(), cwd).unwrap(),
            PathBuf::from("/srv/ws")
        );
        // A relative flag is resolved once, against where it was typed.
        assert_eq!(
            jail_root(Some("ws".into()), home, cwd).unwrap(),
            PathBuf::from("/launched/from/here/ws")
        );
        // No flag and no data home: refused, not guessed.
        assert!(jail_root(None, None, cwd).is_err());
        for root in [
            jail_root(None, Some("/h/.ikigai".into()), cwd).unwrap(),
            jail_root(Some("ws".into()), None, cwd).unwrap(),
        ] {
            assert!(root.is_absolute(), "{}", root.display());
        }
    }

    #[test]
    fn the_command_line_takes_a_port_and_a_root() {
        let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert_eq!(
            parse(&[]).unwrap(),
            Some(Args {
                port: 4433,
                root: None,
                bind: loopback,
                origins: vec![],
            })
        );
        assert_eq!(
            parse(&["47433", "--root", "/srv/ws"]).unwrap(),
            Some(Args {
                port: 47433,
                root: Some("/srv/ws".into()),
                bind: loopback,
                origins: vec![],
            })
        );
        assert_eq!(
            parse(&[
                "--root=/srv/ws",
                "--bind",
                "0.0.0.0",
                "--allow-origin",
                "https://a.example",
                "--allow-origin=http://b.example:8080",
            ])
            .unwrap(),
            Some(Args {
                port: 4433,
                root: Some("/srv/ws".into()),
                bind: "0.0.0.0".parse().unwrap(),
                origins: vec!["https://a.example".into(), "http://b.example:8080".into()],
            })
        );
        assert_eq!(
            parse(&["--bind=::1"]).unwrap().unwrap().bind,
            "::1".parse::<IpAddr>().unwrap()
        );
        assert_eq!(parse(&["--help"]).unwrap(), None);
        for bad in [
            &["44x3"][..],
            &["--root"],
            &["--root="],
            &["--jail", "x"],
            &["1", "2"],
            &["--bind"],
            &["--bind", "localhost:1"],
            &["--allow-origin"],
            &["--allow-origin="],
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should be refused");
        }
    }
}

#[cfg(test)]
mod edge {
    //! The network edge, end to end over a real WebTransport handshake on loopback.

    use super::*;
    use wtransport::endpoint::ConnectOptions;
    use wtransport::error::ConnectingError;
    use wtransport::ClientConfig;

    #[test]
    fn the_default_bind_is_loopback() {
        let args = Args::parse(Vec::<String>::new()).unwrap().unwrap();
        assert!(args.bind.is_loopback(), "{}", args.bind);
    }

    #[test]
    fn the_default_origins_are_the_bundled_pages_and_a_flag_replaces_them() {
        let default = OriginPolicy::from_flags(vec![]);
        for ok in DEFAULT_ORIGINS {
            assert!(default.allows(Some(ok)), "{ok}");
        }
        assert!(default.allows(Some("HTTP://LocalHost:8087/")));
        for no in [
            "https://evil.example",
            "http://127.0.0.1:8088",
            "https://ikigai-rs.github.io.evil.example",
            "null",
        ] {
            assert!(!default.allows(Some(no)), "{no}");
        }
        assert!(default.allows(None), "a non-browser client sends no Origin");
        let only = OriginPolicy::from_flags(vec!["https://a.example".into()]);
        assert!(only.allows(Some("https://a.example")));
        assert!(!only.allows(Some("http://127.0.0.1:8087")));
        assert_eq!(
            OriginPolicy::from_flags(vec!["https://a.example".into(), "*".into()]),
            OriginPolicy::Any
        );
    }

    /// Start a server on an ephemeral loopback port, and connect to it with `origin`.
    async fn connect_with(origin: Option<&str>) -> Result<(), ConnectingError> {
        let jail = tempfile::tempdir().expect("tempdir");
        let kernel = Arc::new(ikigai_web_demo::build_kernel_in(
            "Remote (test)",
            jail.path().to_path_buf(),
        ));
        let identity = Identity::self_signed(["localhost", "127.0.0.1"]).expect("identity");
        let hash = identity.certificate_chain().as_slice()[0].hash();
        let args = Args::parse(vec!["0".to_string()]).unwrap().unwrap();
        let server = endpoint(SocketAddr::new(args.bind, 0), identity).expect("bind");
        let addr = server.local_addr().expect("addr");
        assert!(addr.ip().is_loopback(), "{addr}");
        let task = tokio::spawn(accept_loop(
            server,
            kernel,
            Arc::new(OriginPolicy::from_flags(vec![])),
        ));

        let client = Endpoint::client(
            ClientConfig::builder()
                .with_bind_default()
                .with_server_certificate_hashes([hash])
                .build(),
        )
        .expect("client");
        let mut options = ConnectOptions::builder(format!("https://{addr}/"));
        if let Some(origin) = origin {
            options = options.add_header("origin", origin);
        }
        let result = client.connect(options.build()).await.map(|_| ());
        task.abort();
        result
    }

    /// Wire v9's `IssueWith` (ledger #1546) is served as the `IssueAs` it extends, under the
    /// run context it carries: a plain one answers `Resolved` byte-for-byte as `Issue` does,
    /// a recording one answers `ResolvedTraced` with the spans as `IssueTraced` does, and a
    /// capability that does not cover the target is refused typed, as it is without the
    /// context. On main before this arm the match did not cover the variant at all
    /// (E0004 under wire 0.1.66; under 0.1.63 the call did not decode).
    #[test]
    fn a_wire_v9_issue_with_is_served_as_the_issue_as_it_extends() {
        use ikigai_core::{ArgRef, Iri, Request, Verb};
        use ikigai_wire::IssueWith;

        let jail = tempfile::tempdir().expect("tempdir");
        let kernel = ikigai_web_demo::build_kernel_in("Remote (test)", jail.path().to_path_buf());
        let upper = || {
            Request::new(Verb::Source, Iri::parse("urn:iki:fn:toUpper").unwrap())
                .with_arg("in", ArgRef::Inline(b"hi".to_vec()))
        };
        let answer = |call: &Call| {
            decode::<Reply>(&dispatch(&kernel, &encode(call).unwrap())).expect("a Reply")
        };
        let with = IssueWith {
            trace: None,
            retries: Some(2),
            time_left_ms: Some(30_000),
        };

        // The context changes nothing about the answer: the same bytes as a plain `Issue`.
        let plain = answer(&Call::Issue(upper()));
        let carried = answer(&Call::IssueWith(upper(), Capability::root(), with));
        match (&plain, &carried) {
            (Reply::Resolved(a, _), Reply::Resolved(b, _)) => {
                assert_eq!(a.bytes, b"HI");
                assert_eq!(a.bytes, b.bytes);
            }
            other => panic!("toUpper resolves both ways: {other:?}"),
        }

        // A recording peer gets its spans back, exactly as `IssueTraced` answers.
        let ctx = ikigai_wire::TraceContext {
            trace_id: 7,
            parent_span: None,
        };
        let recording = IssueWith {
            trace: Some(ctx),
            ..with
        };
        match answer(&Call::IssueWith(upper(), Capability::root(), recording)) {
            Reply::ResolvedTraced(repr, _, events) => {
                assert_eq!(repr.bytes, b"HI");
                assert!(
                    events.iter().any(|e| e.target == "urn:iki:fn:toUpper"),
                    "the recorded spans name the resolution: {events:?}"
                );
            }
            other => panic!("a recording IssueWith answers ResolvedTraced: {other:?}"),
        }

        // A gated target under a capability that does not cover it (a fetch with no
        // `urn:cap:net:*` grant, refused by the ACL before any transport runs) is a typed
        // denial, the same one as without the context. `toUpper` would not do here: it
        // requires nothing, so an empty scope resolves it.
        let fetch = || {
            Request::new(Verb::Source, Iri::parse("urn:http:get").unwrap())
                .with_arg("url", ArgRef::Inline(b"http://127.0.0.1:9/never".to_vec()))
        };
        let none = Capability::scoped(Vec::<String>::new());
        let refused = answer(&Call::IssueWith(fetch(), none.clone(), with));
        let legacy = answer(&Call::IssueAs(fetch(), none));
        assert!(
            matches!(refused, Reply::ErrorTyped(WireError::Denied(_))),
            "{refused:?}"
        );
        assert_eq!(refused, legacy);
    }

    #[tokio::test]
    async fn a_disallowed_origin_is_refused_at_the_handshake() {
        assert!(matches!(
            connect_with(Some("https://evil.example")).await,
            Err(ConnectingError::SessionRejected)
        ));
    }

    #[tokio::test]
    async fn an_allowed_origin_and_no_origin_are_admitted() {
        connect_with(Some("http://127.0.0.1:8087"))
            .await
            .expect("an allowed origin connects");
        connect_with(None)
            .await
            .expect("a client without an Origin connects");
    }
}
