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

use std::sync::Arc;
use std::time::Duration;

use ikigai_core::Kernel;
use ikigai_resolve::Resolver;
use ikigai_wire::{decode, encode, Call, Reply};
use tokio::io::AsyncReadExt;
use wtransport::endpoint::IncomingSession;
use wtransport::{Endpoint, Identity, ServerConfig};

/// Largest `Call` we'll read off a stream — a guard against a runaway client.
const MAX_CALL: usize = 8 * 1024 * 1024;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(4433);

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

    let kernel = Arc::new(ikigai_web_demo::build_kernel("Remote (WebTransport)"));

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
        Ok(Call::Issue(request)) => match Resolver::issue(kernel, request) {
            Ok((representation, status)) => Reply::Resolved(representation, status),
            Err(e) => Reply::Error(e),
        },
        Ok(Call::IsCached(request)) => Reply::Cached(Resolver::is_cached(kernel, &request)),
        Ok(Call::Entries) => Reply::Entries(Resolver::entries(kernel)),
        Err(e) => Reply::Error(format!("undecodable call: {e}")),
    };
    encode(&reply).unwrap_or_default()
}
