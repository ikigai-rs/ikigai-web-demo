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
//! ikigai-net-server [PORT] [--root DIR]
//! ```
//!
//! `PORT` defaults to 4433. `--root` names the file module's jail (`urn:file:*`); without
//! it the jail is `<data home>/web-demo/ws` (`~/.ikigai/web-demo/ws`). Either way the
//! server resolves the jail to an ABSOLUTE, canonical path once, at startup, creates it if
//! it is missing, and prints it. It used to be the relative `ws`, so the files a client
//! read and wrote depended on the directory the server happened to be launched from
//! (ledger #183). A path-ACL scope sent with a capability is spelled against that printed
//! path (`urn:cap:fs:read:<root>/…`), the spelling `ikigai-fs` documents.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ikigai_core::{Capability, Kernel};
use ikigai_resolve::{Resolver, SpanCollector};
use ikigai_wire::{decode, encode, Call, Reply, WireError};
use tokio::io::AsyncReadExt;
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
    let port = args.port;
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

    // Self-signed cert valid for localhost; the browser pins its SHA-256.
    let identity = Identity::self_signed(["localhost", "127.0.0.1", "::1"])?;
    let cert_hash = identity.certificate_chain().as_slice()[0].hash();
    let hash_hex: String = cert_hash
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    println!("ikigai WebTransport server  →  https://127.0.0.1:{port}");
    println!("cert sha-256: {hash_hex}");
    println!("open the network demo page with  #cert={hash_hex}  in the URL");
    println!("file jail (urn:file:*): {}", root.display());

    let kernel = Arc::new(ikigai_web_demo::build_kernel_in(
        "Remote (WebTransport)",
        root,
    ));

    let config = ServerConfig::builder()
        .with_bind_default(port)
        .with_identity(identity)
        .keep_alive_interval(Some(Duration::from_secs(3)))
        .build();
    let server = Endpoint::server(config)?;

    loop {
        let incoming = server.accept().await;
        let kernel = Arc::clone(&kernel);
        tokio::spawn(async move {
            if let Err(e) = serve(incoming, kernel).await {
                eprintln!("session ended: {e}");
            }
        });
    }
}

const USAGE: &str = "usage: ikigai-net-server [PORT] [--root DIR]\n  \
    PORT        UDP port to serve WebTransport on (default 4433)\n  \
    --root DIR  the file jail for urn:file:* (default <data home>/web-demo/ws, \
    i.e. ~/.ikigai/web-demo/ws)";

/// The command line, parsed. `Ok(None)` is a request for the usage text.
#[derive(Debug, PartialEq)]
struct Args {
    port: u16,
    root: Option<PathBuf>,
}

impl Args {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Args>, String> {
        let mut port = None;
        let mut root = None;
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            if arg == "-h" || arg == "--help" {
                return Ok(None);
            } else if arg == "--root" {
                let dir = args.next().ok_or("--root needs a directory")?;
                root = Some(PathBuf::from(dir));
            } else if let Some(dir) = arg.strip_prefix("--root=") {
                root = Some(PathBuf::from(dir));
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
/// the client disconnects.
async fn serve(
    incoming: IncomingSession,
    kernel: Arc<Kernel>,
) -> Result<(), Box<dyn std::error::Error>> {
    let connection = incoming.await?.accept().await?;
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
        // three transports answer a failure identically.
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
        assert_eq!(
            parse(&[]).unwrap(),
            Some(Args {
                port: 4433,
                root: None
            })
        );
        assert_eq!(
            parse(&["47433", "--root", "/srv/ws"]).unwrap(),
            Some(Args {
                port: 47433,
                root: Some("/srv/ws".into())
            })
        );
        assert_eq!(
            parse(&["--root=/srv/ws"]).unwrap(),
            Some(Args {
                port: 4433,
                root: Some("/srv/ws".into())
            })
        );
        assert_eq!(parse(&["--help"]).unwrap(), None);
        for bad in [
            &["44x3"][..],
            &["--root"],
            &["--root="],
            &["--jail", "x"],
            &["1", "2"],
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should be refused");
        }
    }
}
