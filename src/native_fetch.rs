//! The native HTTP transport: what lets the WebTransport server's `urn:http*` reach the
//! web (ledger #184). Native only — the page has `fetch` instead (`BrowserFetchTransport`
//! in `lib.rs`).
//!
//! It is a blocking `ureq` client, as `ikigai-cli`'s is, and like that one it honors the
//! [`HttpTransport`] contract that matters most: **a redirect is returned, never
//! followed.** `ikigai-http`'s endpoint follows a 3xx itself, re-running the
//! `urn:cap:net:*` ACL against every hop; a transport that followed on its own would let a
//! granted host 302 the request to an ungranted one behind the capability's back.
//! `tests/native_fetch.rs` pins that against a loopback stub.
//!
//! Installed only where a host asks for it: [`crate::build_kernel_in`] builds a native
//! kernel with NO network transport, so a test that walks the kernel never reaches a real
//! host by accident, and the server passes this one to
//! [`crate::build_kernel_with_transport`].

use std::io::Read;
use std::time::Duration;

use async_trait::async_trait;
use ikigai_http::{HttpRequest, HttpResponse, HttpTransport, Method};

/// The largest response body accepted. A larger one is REFUSED, not truncated: a cut-off
/// body would reach a caller as if it were the resource.
pub const MAX_BODY: u64 = 16 * 1024 * 1024;

/// A blocking `ureq` transport that never follows a redirect.
pub struct NativeFetchTransport {
    agent: ureq::Agent,
}

impl NativeFetchTransport {
    /// A transport whose every request (connect, send, read) is bounded by `timeout`.
    pub fn new(timeout: Duration) -> Self {
        NativeFetchTransport {
            agent: ureq::AgentBuilder::new()
                // The contract: a 3xx comes back as the response, for the endpoint to
                // judge against the capability.
                .redirects(0)
                .timeout(timeout)
                .build(),
        }
    }
}

impl Default for NativeFetchTransport {
    /// Thirty seconds: a slow host fails as `Unavailable` rather than holding a server
    /// task forever.
    fn default() -> Self {
        NativeFetchTransport::new(Duration::from_secs(30))
    }
}

#[async_trait]
impl HttpTransport for NativeFetchTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, String> {
        let mut req = self.agent.request(request.method.as_str(), &request.url);
        for (name, value) in &request.headers {
            req = req.set(name, value);
        }
        let outcome = if request.body.is_empty() {
            req.call()
        } else {
            req.send_bytes(&request.body)
        };
        // A 4xx/5xx is still a response — the endpoint maps it onto the error taxonomy.
        let resp = match outcome {
            Ok(resp) => resp,
            Err(ureq::Error::Status(_, resp)) => resp,
            Err(e) => return Err(e.to_string()),
        };
        let status = resp.status();
        let headers = resp
            .headers_names()
            .into_iter()
            .filter_map(|name| resp.header(&name).map(|v| (name.clone(), v.to_string())))
            .collect();
        let mut body = Vec::new();
        if request.method != Method::Head {
            resp.into_reader()
                .take(MAX_BODY + 1)
                .read_to_end(&mut body)
                .map_err(|e| format!("reading the response body: {e}"))?;
            if body.len() as u64 > MAX_BODY {
                return Err(format!(
                    "the response body from {} exceeds {MAX_BODY} bytes",
                    request.url
                ));
            }
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}
