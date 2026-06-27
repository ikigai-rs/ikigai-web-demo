//! The ikigai kernel — and the ikigai *CLI* — running in the browser via WebAssembly.
//!
//! One in-page [`Kernel`] binds the demo endpoints, the `compose` builtin, and the
//! page shapes, behind a meta renderer that also projects each endpoint's
//! self-description to `application/json`. The CLI's renderer-agnostic
//! [`Engine`](ikigai_engine::Engine) drives that kernel; [`evalLine`] runs one REPL
//! line through it. Both the composed page and the in-page terminal go through the
//! same Engine, so they share one resource space and one content-addressed cache.
//!
//! The Engine is synchronous — it hides a `block_on` — which is fine on wasm because
//! the kernel's futures are immediately-ready (no parking). It lives in a
//! `thread_local` (the browser is single-threaded and the Engine isn't `Sync`).

use std::sync::Arc;

use async_trait::async_trait;
use ikigai_core::{
    ArgRef, Capability, Clock, Description, Endpoint, Error, Exact, Fallback, FnEndpoint,
    Invocation, Iri, Kernel, MetaRenderer, ReprType, Representation, Request, Result, Space, Time,
    UriTemplate, Verb,
};
use ikigai_vocab::TurtleRenderer;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

/// A demo endpoint with a rich self-description. It greets you from the browser.
struct Greeter;

#[async_trait]
impl Endpoint for Greeter {
    async fn invoke(&self, _inv: &Invocation<'_>) -> Result<Representation> {
        Ok(Representation::new(
            ReprType::new("text/plain").with_param("charset", "utf-8"),
            "Hello from the ikigai kernel — resolved in your browser via WebAssembly."
                .as_bytes()
                .to_vec(),
        )
        .cacheable())
    }

    fn name(&self) -> &str {
        "greeter"
    }

    fn describe(&self) -> Description {
        Description::new("greeter")
            .title("Greeter")
            .summary("A demo endpoint that greets you from inside the browser tab.")
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .output("text/plain")
    }
}

/// `urn:data:page` — the page *shape*. A `compose` source: HTML whose `$a{<iri>}`
/// markers transclude other resources in this kernel (one of them, `urn:demo:web-cli`,
/// mounts the live terminal). Composition recurses — `urn:data:about` is itself a
/// shape with its own marker.
const PAGE_HTML: &str = r##"
<nav class="ik-toolbar" aria-label="pages">
  <button class="ik-nav selected" hx-get="/k/source urn:fn:compose src=urn:data:page" hx-target="#app" hx-swap="innerHTML" aria-current="page">Home</button>
  <button class="ik-nav" hx-get="/k/source urn:fn:compose src=urn:data:docs" hx-target="#app" hx-swap="innerHTML">Catalog</button>
  <button class="ik-nav" hx-get="/k/source urn:fn:compose src=urn:data:demo" hx-target="#app" hx-swap="innerHTML">Demo</button>
</nav>
<h1>A page assembled by ikigai</h1>
<p class="sub">This whole page is <b>one resource</b>. The browser issued a single
   <code>compose(urn:data:page)</code>; the in-browser kernel resolved the page shape and
   recursively expanded every <code>$$a{…}</code> marker — no fetch, no server, no per-slot
   JavaScript. Resolution, the endpoints, and the cache all run here in WebAssembly.</p>

<article>
  <p>$a{urn:demo:greeter}</p>

  <p>Shout it (<code>urn:fn:toUpper</code>):
     <b>$a{urn:fn:toUpper?in="resource-oriented computing"}</b></p>

  $a{urn:data:about}

  <p class="literal">A literal marker — written with a doubled <code>$</code> — survives
     unexpanded: <code>$$a{urn:fn:toUpper?in=x}</code></p>
</article>

$a{urn:demo:web-cli}
"##;

/// `urn:data:docs` — a second page, reached from the toolbar. It carries the same
/// toolbar (HATEOAS: the nav lives in the representation, so "which page" is the HTML
/// the kernel returns, not client state) and renders the kernel's own **catalog** as an
/// HTML table. The table is loaded with a single htmx `hx-trigger="load"` that resolves
/// `urn:kernel:catalog | urn:rdf:transrept as=text/html` through the in-page kernel — so
/// the catalog arrives as *rendered* HTML, not escaped source, with no bespoke JS. This
/// is the inter-page-linking testbed; `urn:data:cache` / `urn:data:trace` slot in later,
/// and become real HTTP resources once ikigai is served externally.
const DOCS_HTML: &str = r##"
<nav class="ik-toolbar" aria-label="pages">
  <button class="ik-nav" hx-get="/k/source urn:fn:compose src=urn:data:page" hx-target="#app" hx-swap="innerHTML">Home</button>
  <button class="ik-nav selected" hx-get="/k/source urn:fn:compose src=urn:data:docs" hx-target="#app" hx-swap="innerHTML" aria-current="page">Catalog</button>
  <button class="ik-nav" hx-get="/k/source urn:fn:compose src=urn:data:demo" hx-target="#app" hx-swap="innerHTML">Demo</button>
</nav>
<h1>The catalog</h1>
<p class="sub">The kernel describes itself. <code>urn:kernel:catalog</code> emits every bound
   endpoint as RDF; here it's transrepted to RDF/XML and styled into cards by an
   <code>urn:xslt:transform</code> — <b>src</b> and <b>stylesheet</b> both cacheable resource
   references, all client-side in WebAssembly. Swap the stylesheet, restyle the same graph.
   Turtle all the way down.</p>
<div id="ik-docs-catalog" class="cards-pane"
     hx-get="/k/source urn:xslt:transform src=urn:data:catalog.rdf stylesheet=urn:style:catalog-cards"
     hx-trigger="load" hx-swap="innerHTML">resolving the catalog…</div>
"##;

/// `urn:data:demo` — the guided-demos page, reached from the toolbar. Carries the same
/// toolbar (Demo active) and hosts the runbook tab strip. The `#runbook` section
/// bootstraps via `hx-trigger="load"` (like the Catalog cards-pane), and the runbook's
/// own tabs/steps swap `#runbook` from there. Moving the runbook here keeps the Home page
/// about composition and gives the demos their own page.
const DEMO_HTML: &str = r##"
<nav class="ik-toolbar" aria-label="pages">
  <button class="ik-nav" hx-get="/k/source urn:fn:compose src=urn:data:page" hx-target="#app" hx-swap="innerHTML">Home</button>
  <button class="ik-nav" hx-get="/k/source urn:fn:compose src=urn:data:docs" hx-target="#app" hx-swap="innerHTML">Catalog</button>
  <button class="ik-nav selected" hx-get="/k/source urn:fn:compose src=urn:data:demo" hx-target="#app" hx-swap="innerHTML" aria-current="page">Demo</button>
</nav>
<h1>Guided demos</h1>
<p class="sub">Runnable walkthroughs — each tab is a <code>urn:runbook:*</code> resource rendered
   as htmx (HATEOAS); switching tabs and running steps are both just "resolve a resource."</p>
<section id="runbook" aria-label="ikigai runbook"
     hx-get="/k/source urn:runbook:basics as=text/html" hx-trigger="load" hx-swap="innerHTML">loading runbook…</section>
"##;

/// `urn:style:catalog-cards` — the XSLT stylesheet (a resource) that styles the catalog
/// RDF/XML into a grid of endpoint cards. One `xsl:template match="ik:Endpoint"` emits a
/// card per endpoint; XSLT does the per-endpoint iteration. Swapping this resource
/// restyles the same cached graph — the reuse the XSLT module buys.
const CATALOG_CARDS_XSL: &str = r#"<xsl:stylesheet version="1.0"
  xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
  xmlns:ik="https://ikigai-rs.dev/ns#">
  <xsl:output method="html"/>
  <xsl:template match="/">
    <div class="cards"><xsl:apply-templates select="//ik:Endpoint"/></div>
  </xsl:template>
  <xsl:template match="ik:Endpoint | ik:Transreptor">
    <article class="card">
      <div class="card-head">
        <h3 class="card-title">
          <xsl:choose>
            <xsl:when test="ik:title"><xsl:value-of select="ik:title"/></xsl:when>
            <xsl:otherwise><xsl:value-of select="ik:id"/></xsl:otherwise>
          </xsl:choose>
        </h3>
        <code class="card-id"><xsl:value-of select="ik:id"/></code>
        <xsl:if test="ik:transreptsTo"><span class="card-kind">transreptor</span></xsl:if>
      </div>
      <xsl:if test="ik:summary"><p class="card-summary"><xsl:value-of select="ik:summary"/></p></xsl:if>
      <xsl:if test="ik:transreptsTo">
        <div class="card-conv">
          <span class="conv-label">from</span>
          <xsl:for-each select="ik:transreptsFrom"><code class="conv-type"><xsl:value-of select="."/></code></xsl:for-each>
          <span class="conv-arrow">&#8594;</span>
          <span class="conv-label">to</span>
          <xsl:for-each select="ik:transreptsTo"><code class="conv-type"><xsl:value-of select="."/></code></xsl:for-each>
        </div>
      </xsl:if>
      <xsl:if test="ik:verb or ik:output">
        <div class="card-meta">
          <span class="card-verbs"><xsl:for-each select="ik:verb"><span class="verb"><xsl:value-of select="."/></span></xsl:for-each></span>
          <xsl:if test="ik:output"><code class="card-out"><xsl:value-of select="ik:output"/></code></xsl:if>
        </div>
      </xsl:if>
    </article>
  </xsl:template>
</xsl:stylesheet>"#;

/// `urn:data:about` — a nested shape the page transcludes, which itself transcludes
/// another resource. Proof that composition recurses.
const ABOUT_HTML: &str = r#"<aside class="about">
  <h3>Composition recurses</h3>
  <p>This box is a separate resource (<code>urn:data:about</code>) the page pulled in — and
     it pulled in another:
     <b>$a{urn:fn:toUpper?in="even this nested shape was composed"}</b>.</p>
</aside>"#;

/// `urn:demo:web-cli` — the terminal mount. Transcluded into the page; the
/// `<ikigai-cli>` custom element self-wires on insertion and drives [`evalLine`].
const WEB_CLI_HTML: &str = r#"<section class="cli-mount">
  <h3>The same CLI, in your browser</h3>
  <p class="sub">This terminal runs the very same renderer-agnostic Engine as the desktop
     <code>ikigai</code> REPL, on this page's kernel — pipelines <code>|</code>, map
     <code>..</code>, fork <code>( a ; b )</code>, named <code>key=value</code> args, plus
     <code>compose</code>, <code>cache</code>, <code>cap</code>, and <code>list</code>. Try
     <code>list</code>, or <code>source urn:fn:compose src=urn:data:page</code> to compose this
     very page. The <b>ZeroTrust</b> buttons below walk the capability story — narrow the
     session with <code>cap read-only</code> and watch a write get refused, while the jail
     refuses to escape even at full authority.</p>
  <ikigai-cli></ikigai-cli>
</section>"#;

/// A `text/html` shape endpoint returning a fixed body (which carries `$a{}` markers).
/// The `title`/`summary` give it a real self-description, so it shows up as a proper
/// card in `urn:kernel:catalog` (and in `list`/Meta) rather than a bare id.
fn shape(
    name: &'static str,
    title: &'static str,
    summary: &'static str,
    html: &'static str,
) -> FnEndpoint {
    FnEndpoint::new(name, move |_inv: &Invocation<'_>| {
        Ok(Representation::new(
            ReprType::new("text/html").with_param("charset", "utf-8"),
            html.as_bytes().to_vec(),
        )
        .cacheable())
    })
    .with_description(
        Description::new(name)
            .title(title)
            .summary(summary)
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .output("text/html;charset=utf-8"),
    )
}

/// Turtle / plain-text self-descriptions, plus an `application/json` projection —
/// which the Engine reads to learn each endpoint's argument contract for `source`
/// routing (the same renderer the desktop CLI uses).
struct JsonOrTurtle;

impl MetaRenderer for JsonOrTurtle {
    fn render(&self, description: &Description, target: &ReprType) -> Result<Representation> {
        if target.media_type == "application/json" {
            let json = serde_json::to_vec(description)
                .map_err(|e| Error::Endpoint(format!("describe as json: {e}")))?;
            return Ok(Representation::new(ReprType::new("application/json"), json));
        }
        TurtleRenderer.render(description, target)
    }
}

/// `urn:host:info` — reports the host's `nature` (set by whoever builds the
/// kernel: `Embedded (Browser)` in the page, `Remote (WebTransport)` on the
/// server) and runtime, so `source urn:host:info` shows what differs between the
/// in-browser and over-the-wire situations. Uncacheable — a live host fact.
fn host_info(nature: &'static str) -> FnEndpoint {
    FnEndpoint::new("host-info", move |_inv: &Invocation<'_>| {
        let runtime = if cfg!(target_family = "wasm") {
            "browser · wasm32".to_string()
        } else {
            format!(
                "native · {}/{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        };
        let body = format!(
            "ikigai host\n  nature    {nature}\n  runtime   {runtime}\n  \
             space     ikigai-fn (toUpper · reverseList · wrap · split · greet · echo · compose) + greeter + files (urn:file:* → localStorage)\n"
        );
        Ok(Representation::new(
            ReprType::new("text/plain").with_param("charset", "utf-8"),
            body.into_bytes(),
        ))
    })
    .with_description(
        Description::new("host-info")
            .title("Host info")
            .summary("Reports the kernel host's nature (embedded/remote + transport) and runtime.")
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .output("text/plain;charset=utf-8"),
    )
}

/// `urn:host:identity` — the login affordance, rendered **from the session capability**
/// (the capability *is* the identity). Composed into the top of the page via a
/// `$a{urn:host:identity}` marker, so the sign-in/out link is part of the page resource
/// (HATEOAS), not bolted-on chrome. After a passkey login the page re-resolves this one
/// resource and swaps it in. Uncacheable — it reflects live session state.
///
/// Anonymous (root authority) renders a "Sign in with passkey" link; a session holding
/// `urn:cap:fs:read:ws/<id>` renders the `ws/<id>` identity chip plus "Sign out". The
/// imperative WebAuthn ceremony behind `#ik-login`/`#ik-logout` lives in the page's one
/// glue bridge (index.html), the same shape as the htmx `/k/<cmd>` adapter.
/// The workspace segment id from a session capability's `urn:cap:fs:read:ws/<id>` scope
/// — the logged-in identity, or `None` when anonymous (root) / unscoped. The capability
/// *is* the identity, so the affordance and the walkthrough both read it from here.
fn session_client_id(cap: &Capability) -> Option<String> {
    cap.scopes().and_then(|scopes| {
        scopes
            .iter()
            .find_map(|s| s.strip_prefix("urn:cap:fs:read:ws/"))
            .map(str::to_string)
    })
}

/// The sign-in / signed-in affordance HTML, from the session identity. `#ik-login` runs
/// the passkey ceremony (the page's one glue bridge); `#ik-logout` sinks
/// `urn:host:logout`.
fn affordance_html(client: Option<&str>) -> String {
    match client {
        Some(id) => format!(
            "<nav class=\"ik-id ik-id-in\">signed in · <code>ws/{id}</code> \
             <a href=\"#\" id=\"ik-logout\" class=\"ik-id-link\">Sign out</a></nav>"
        ),
        None => "<nav class=\"ik-id\">\
             <a href=\"#\" id=\"ik-login\" class=\"ik-id-link\">🔑 Sign in with a passkey</a> \
             <span class=\"ik-id-hint\">— scopes a private workspace segment to you</span>\
             </nav>"
            .to_string(),
    }
}

/// `urn:host:identity` — the session identity as a resource, rendered from the session
/// capability. Sourced in-page after a login to refresh the affordance, and (once the
/// QUIC server resolves each connection under its cert-derived capability) over the wire
/// to report who the remote server thinks you are. Uncacheable — live session state.
fn host_identity() -> FnEndpoint {
    FnEndpoint::new("host-identity", move |inv: &Invocation<'_>| {
        let client = session_client_id(inv.capability);
        Ok(Representation::new(
            ReprType::new("text/html").with_param("charset", "utf-8"),
            affordance_html(client.as_deref()).into_bytes(),
        ))
    })
    .with_description(
        Description::new("host-identity")
            .title("Identity")
            .summary("The session identity, rendered from the session capability.")
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .output("text/html;charset=utf-8"),
    )
}

/// `urn:runbook:identity` — the browser-only **Identity** runbook tab. Renders the shared
/// runbook tab strip (with Identity marked active) followed by a panel: the passkey
/// affordance plus, when signed in, a walkthrough whose step commands have the live
/// client id **baked in** (read from the capability) — so the segment-isolation demo is
/// pure HATEOAS, the only JavaScript being the WebAuthn ceremony behind `#ik-login`.
fn runbook_identity() -> FnEndpoint {
    FnEndpoint::new("runbook-identity", move |inv: &Invocation<'_>| {
        let client = session_client_id(inv.capability);
        let affordance = affordance_html(client.as_deref());
        let walkthrough = match client.as_deref() {
            Some(id) => format!(
                "<ol class=\"rb-steps\">\
                 <li><button class=\"rb-step\" hx-get=\"/k/sink urn:file:{id}/secret.txt mine only\" \
                   hx-target=\"#rb-out\" hx-swap=\"beforeend\">write a private note</button> \
                   <span class=\"rb-note\">lands — you hold write on <code>ws/{id}</code></span></li>\
                 <li><button class=\"rb-step\" hx-get=\"/k/source urn:file:{id}/secret.txt\" \
                   hx-target=\"#rb-out\" hx-swap=\"beforeend\">read it back</button> \
                   <span class=\"rb-note\">resolves under your capability</span></li>\
                 <li><button class=\"rb-step\" hx-get=\"/k/sink urn:file:someone-else/secret.txt nope\" \
                   hx-target=\"#rb-out\" hx-swap=\"beforeend\">write outside your segment</button> \
                   <span class=\"rb-note\">refused — your capability grants only <code>ws/{id}</code></span></li>\
                 </ol>"
            ),
            None => String::new(),
        };
        let body = format!(
            "{strip}<section class=\"rb-panel\" role=\"tabpanel\">\
             <p class=\"rb-intro\">A passkey establishes a client <b>identity</b> that scopes a \
             private workspace segment (<code>ws/&lt;id&gt;</code>) to you: while signed in, files \
             resolve under — and are confined to — your segment, and another identity's segment is \
             refused by the resolver. The boundary is the capability/resource model (logical, not \
             yet cryptographic).</p>\
             {affordance}{walkthrough}\
             <pre id=\"rb-out\" class=\"rb-out\" aria-live=\"polite\"></pre></section>",
            strip = ikigai_runbook::render_tab_strip("identity"),
        );
        Ok(Representation::new(
            ReprType::new("text/html").with_param("charset", "utf-8"),
            body.into_bytes(),
        ))
    })
    .with_description(
        Description::new("runbook-identity")
            .title("Identity")
            .summary("The passkey identity runbook tab — sign in and watch the segment boundary.")
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .output("text/html;charset=utf-8"),
    )
}

/// `urn:data:catalog.rdf` — the kernel's catalog as RDF/XML, a resource. It resolves
/// `urn:kernel:catalog` (Turtle) through the kernel and transrepts it, so this resource
/// depends on the catalog's golden thread and is cacheable. The Catalog page's XSLT cards
/// reference it as their `src` — both src and stylesheet are named, cacheable resources.
struct CatalogRdf;

#[async_trait]
impl Endpoint for CatalogRdf {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        let catalog = inv
            .source(&Iri::parse("urn:kernel:catalog").expect("valid iri"))
            .await?;
        // Transrept the Turtle to RDF/XML through the kernel (composes the cache: this
        // resource is cacheable and invalidates with the catalog).
        let transrept = Request::new(
            Verb::Source,
            Iri::parse("urn:rdf:transrept").expect("valid iri"),
        )
        .with_arg("content", ArgRef::Inline(catalog.bytes))
        .with_arg("as", ArgRef::Inline(b"application/rdf+xml".to_vec()));
        inv.issue(transrept).await
    }

    fn name(&self) -> &str {
        "catalog-rdf"
    }

    fn describe(&self) -> Description {
        Description::new("catalog-rdf")
            .title("Catalog (RDF/XML)")
            .summary(
                "The kernel's catalog transrepted to RDF/XML — the src for the cards stylesheet.",
            )
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .output("application/rdf+xml")
    }
}

/// The XSLT module as a **dynamically-loaded** wasm artifact. On wasm the host doesn't link
/// xrust at all: `urn:xslt:*` routes to a generic [`WasmModuleSpace`](ikigai_module) over a
/// browser transport, which lazy-loads `ikigai_xslt.wasm` (xrust, ~2.4 MB, fetched on first
/// use) and drives the real `ModuleCall`/`ModuleReply` session. The module pulls its
/// `src`/`stylesheet` back as `HostCall`s, which the host's `hostCall` export services with
/// [`serve_host_call`](ikigai_module) — provenance and all. The native server keeps xslt
/// linked (no lazy-load machinery there), the same split as CLI-links / browser-loads.
#[cfg(target_family = "wasm")]
mod xslt_module {
    use super::*;
    use ikigai_core::ArgSpec;
    use ikigai_module::{ModuleSessionTransport, WasmModuleSpace};

    // The lazy module's session entry, a global the loader wires to `invoke_session` on the
    // module wasm (index.html → dist/xslt-loader.js). The heavy module wasm loads inside it
    // on first call.
    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(catch, js_name = "xsltInvokeSession")]
        async fn xslt_invoke_session(invoke: Vec<u8>) -> std::result::Result<JsValue, JsValue>;
    }

    /// The browser transport for the lazy XSLT module: ferry the encoded session bytes to
    /// the global `xsltInvokeSession` and back. `JsValue` is `!Send`, so confine the call to
    /// a `spawn_local` task and bridge the (`Send`) bytes back through a oneshot — exactly
    /// like `BrowserFetchTransport`. (The module's `hostCall` callbacks run on the event loop
    /// while this awaits.) Everything else — the session, the provenance sink — is generic in
    /// `ikigai-module`.
    struct BrowserXsltTransport;

    #[async_trait]
    impl ModuleSessionTransport for BrowserXsltTransport {
        async fn invoke_session(&self, invoke: Vec<u8>) -> std::result::Result<Vec<u8>, String> {
            let (tx, rx) = futures::channel::oneshot::channel();
            wasm_bindgen_futures::spawn_local(async move {
                let result = match xslt_invoke_session(invoke).await {
                    Ok(value) => Ok(js_sys::Uint8Array::new(&value).to_vec()),
                    Err(e) => Err(e
                        .as_string()
                        .unwrap_or_else(|| "xslt module session failed".to_string())),
                };
                let _ = tx.send(result);
            });
            rx.await
                .map_err(|_| "xslt module task was dropped".to_string())?
        }
    }

    /// The module endpoint's catalog card — handed to [`WasmModuleSpace`] so the lazy module
    /// shows a rich description without being instantiated.
    fn describe() -> Description {
        Description::new("xslt-transform")
            .title("XSLT transform")
            .summary(
                "Apply an XSLT stylesheet to a source document, both as cacheable resource \
                 references. Runs in a dynamically-loaded wasm module (xrust), fetched on \
                 first use.",
            )
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .input(ArgSpec::new("src").summary("the source XML/RDF-XML resource IRI"))
            .input(ArgSpec::new("stylesheet").summary("the XSLT stylesheet resource IRI"))
            .input(ArgSpec::new("as").summary("output media type (default text/html)"))
            .output("text/html;charset=utf-8")
            // Mirror the module's own marking (ikigai-xslt): a parameterized
            // ik:Transreptor — needs a `stylesheet`, so it's not auto-invocable.
            .transreptor(
                ["application/xml", "text/xml", "application/rdf+xml"],
                ["text/html", "text/plain"],
            )
    }

    /// The `urn:xslt:*` space: a generic `WasmModuleSpace` over the browser transport.
    pub fn space() -> WasmModuleSpace {
        WasmModuleSpace::new(["urn:xslt:"], Arc::new(BrowserXsltTransport), describe())
    }
}

/// The `urn:xslt:*` space: a dynamically-loaded module on wasm (xrust kept out of the
/// host wasm, lazy-fetched), linked directly on native (the server can't lazy-load wasm).
#[cfg(target_family = "wasm")]
fn xslt_space() -> Arc<dyn Space> {
    Arc::new(xslt_module::space())
}
#[cfg(not(target_family = "wasm"))]
fn xslt_space() -> Arc<dyn Space> {
    Arc::new(ikigai_xslt::space())
}

/// Build the in-page kernel with the host `nature` reported by `urn:host:info`:
/// the demo endpoints, `compose`, and the page shapes, behind the JSON-or-Turtle
/// meta renderer. One kernel drives both the composed page and the terminal, so
/// they share a space and a cache. Public so the WebTransport server
/// (`src/bin/server.rs`) resolves against the same space — with its own nature.
pub fn build_kernel(nature: &'static str) -> Kernel {
    // Register the browser-only Identity tab so the shared runbook strip lists it
    // (idempotent). Its panel is `urn:runbook:identity`, bound below.
    ikigai_runbook::add_tab("identity", "Identity");
    // The reusable functions come from the linked `ikigai-fn` module crate
    // (compiled to wasm32 alongside this lib); this host chains its own page
    // shapes, the in-page terminal mount, the greeter, and `urn:host:info`.
    let space = ikigai_fn::space()
        .bind(Exact::new("urn:demo:greeter"), Greeter)
        .bind(
            Exact::new("urn:demo:web-cli"),
            shape(
                "web-cli",
                "In-page terminal",
                "Mounts the CLI Engine on this page's kernel — the same REPL, in the browser.",
                WEB_CLI_HTML,
            ),
        )
        .bind(
            Exact::new("urn:data:page"),
            shape(
                "page",
                "Home page",
                "The composed demo page — one resource, assembled from $a{} markers in WebAssembly.",
                PAGE_HTML,
            ),
        )
        .bind(
            Exact::new("urn:data:docs"),
            shape(
                "docs",
                "Catalog page",
                "The kernel's catalog, transrepted to RDF/XML and styled into endpoint cards by an XSLT.",
                DOCS_HTML,
            ),
        )
        .bind(
            Exact::new("urn:data:demo"),
            shape(
                "demo",
                "Demo page",
                "The guided runbook demos (urn:runbook:*) on their own page, reached from the toolbar.",
                DEMO_HTML,
            ),
        )
        .bind(
            Exact::new("urn:data:about"),
            shape(
                "about",
                "About box",
                "A nested shape the page transcludes — proof that composition recurses.",
                ABOUT_HTML,
            ),
        )
        // The Catalog page's cards: a `src` resource (catalog → RDF/XML) and a
        // `stylesheet` resource (the XSLT), both cacheable, transformed by urn:xslt:*.
        .bind(Exact::new("urn:data:catalog.rdf"), CatalogRdf)
        .bind(
            Exact::new("urn:style:catalog-cards"),
            shape(
                "catalog-cards-xsl",
                "Catalog cards stylesheet",
                "The XSLT that styles the catalog RDF/XML into the endpoint cards on the Catalog page.",
                CATALOG_CARDS_XSL,
            ),
        )
        .bind(Exact::new("urn:host:info"), host_info(nature))
        .bind(Exact::new("urn:host:identity"), host_identity())
        .bind(Exact::new("urn:runbook:identity"), runbook_identity())
        // The capability-gated file module on its browser backend: `urn:file:{path}`
        // resolves to `localStorage` (keyed `ikigai:fs:ws/<path>`), jailed to the
        // virtual `ws` root. Same module, same `file:` contract as the native CLI —
        // only the backend differs. Files persist across reloads and are shared with
        // page JavaScript through the same store.
        .bind(
            UriTemplate::parse(ikigai_fs::FILE_TEMPLATE).expect("valid file template"),
            // Cacheable: a localStorage read caches under a golden thread, and a
            // `sink` (kernel auto-cut) invalidates it — so the browser shows the
            // golden thread, the same as the native CLI.
            ikigai_fs::FileEndpoint::new("ws").cacheable(),
        );
    // The root is a Fallback over the local space, then the HTTP module on a
    // `fetch`-backed transport — so `urn:httpGet url=…` resolves against the live web
    // from inside the tab, the same resource model the native CLI drives with ureq.
    // A `Date`-backed clock lets the kernel honour `Cache-Control: max-age` (and feeds
    // `urn:kernel:constraint` timing), exactly as `SystemClock` does natively.
    let root: Arc<dyn Space> = Arc::new(Fallback::new(vec![
        Arc::new(space) as Arc<dyn Space>,
        Arc::new(ikigai_http::space(Arc::new(BrowserFetchTransport))) as Arc<dyn Space>,
        // RDF transreption (`urn:rdf:transrept`) — parses RDF and re-serializes to another
        // syntax (or an HTML table), client-side. The Catalog page and the Linked Data tab
        // pipe `urn:kernel:catalog` / a live FOAF fetch through it.
        Arc::new(ikigai_rdf::space()) as Arc<dyn Space>,
        // XSLT (`urn:xslt:transform`) — styles RDF/XML (the catalog) into HTML cards.
        // On wasm it's a dynamically-loaded module (xrust lazy-fetched, not in the host
        // wasm); on native it's linked directly. See `xslt_space()`.
        xslt_space(),
        // The interactive runbook (`urn:runbook:*`) — the same module the native CLI
        // links, rendered here as htmx (HATEOAS) HTML.
        Arc::new(ikigai_runbook::space()) as Arc<dyn Space>,
        // The ikigai vocabulary as a resolvable resource: `source urn:ikigai:vocab`
        // returns the `ns#` ontology Turtle (ik:Transreptor rdfs:subClassOf ik:Endpoint
        // + the property defs) — the same bytes served externally at
        // https://ikigai-rs.dev/ns. Lists in the catalog like any endpoint.
        Arc::new(ikigai_vocab::space()) as Arc<dyn Space>,
        // Content sniffing + sniff-and-dispatch: `urn:sniff` classifies opaque bytes,
        // `urn:transrept:auto` sniffs then routes them to the matching transreptor — so a
        // fetched graph the server mislabels `application/octet-stream` transrepts in the
        // tab without the caller asserting its input type.
        Arc::new(ikigai_sniff::space()) as Arc<dyn Space>,
    ]));
    Kernel::with_meta_renderer(root, Arc::new(JsonOrTurtle)).with_clock(Arc::new(BrowserClock))
}

/// The browser's [`Spawner`](ikigai_core::Spawner): runs each fanned-out task on the
/// JS event loop via `spawn_local`, so re-entrant compose fan-out and engine fork/map
/// run **concurrently** in the tab (not sequentially). The task is `!Send` here (the
/// wasm `BoxFuture` drops the bound), which is exactly why ikigai-core relaxes `Send`
/// on wasm32. A oneshot bridges completion back so the joiner can park on it.
#[cfg(target_family = "wasm")]
struct WasmSpawner;

#[cfg(target_family = "wasm")]
impl ikigai_core::Spawner for WasmSpawner {
    fn spawn(&self, task: ikigai_core::BoxFuture<()>) -> ikigai_core::BoxFuture<()> {
        let (tx, rx) = futures::channel::oneshot::channel();
        wasm_bindgen_futures::spawn_local(async move {
            task.await;
            let _ = tx.send(());
        });
        Box::pin(async move {
            let _ = rx.await;
        })
    }
}

/// A `Date.now()`-backed [`Clock`] for the browser kernel — the wasm analogue of the
/// native `SystemClock` (`std::time` panics on `wasm32-unknown-unknown`). Lets HTTP
/// `max-age` deadlines and the constraint readout's timing work in the tab.
struct BrowserClock;
impl Clock for BrowserClock {
    fn now(&self) -> Time {
        Time::from_millis(js_sys::Date::now() as u64)
    }
}

/// The browser's [`HttpTransport`](ikigai_http::HttpTransport): performs requests with
/// the Fetch API. `fetch` is `!Send` (it touches `JsValue`), but the trait requires a
/// `Send` future — so `send` confines the fetch to a `spawn_local` task and bridges the
/// (`Send`) result back through a oneshot channel, keeping `send`'s own future `Send`.
struct BrowserFetchTransport;

#[async_trait]
impl ikigai_http::HttpTransport for BrowserFetchTransport {
    async fn send(
        &self,
        request: ikigai_http::HttpRequest,
    ) -> std::result::Result<ikigai_http::HttpResponse, String> {
        let (tx, rx) = futures::channel::oneshot::channel();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = tx.send(browser_fetch(request).await);
        });
        rx.await.map_err(|_| "fetch task was dropped".to_string())?
    }
}

/// The actual Fetch-API call — `!Send` (holds `JsValue` across awaits), run on the JS
/// event loop via `spawn_local`. Captures content-type and cache-control (all
/// ikigai-http reads) and the body bytes.
#[cfg(target_family = "wasm")]
async fn browser_fetch(
    request: ikigai_http::HttpRequest,
) -> std::result::Result<ikigai_http::HttpResponse, String> {
    use wasm_bindgen::JsCast;
    let jserr = |e: JsValue| format!("{e:?}");

    let opts = web_sys::RequestInit::new();
    opts.set_method(request.method.as_str());
    if !request.body.is_empty() {
        let body = js_sys::Uint8Array::from(request.body.as_slice());
        opts.set_body(&body);
    }
    let req = web_sys::Request::new_with_str_and_init(&request.url, &opts).map_err(jserr)?;
    for (name, value) in &request.headers {
        req.headers().set(name, value).map_err(jserr)?;
    }

    let window = web_sys::window().ok_or("no window")?;
    // A rejected fetch is almost always a browser security block, and the browser
    // deliberately reports it only as an opaque `TypeError` (it won't say why a
    // cross-origin request failed). Translate that into the actual cause rather than
    // surfacing `TypeError: Load failed`.
    let resp: web_sys::Response =
        wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&req))
            .await
            .map_err(|_| {
                format!(
                    "the browser blocked this fetch. It can only reach an origin that \
                 returns CORS headers (Access-Control-Allow-Origin), and an https page \
                 cannot fetch http. `{}` likely does neither — the native CLI has no \
                 such limit. (A CORS-enabled https URL works, e.g. https://httpbin.org/uuid.)",
                    request.url
                )
            })?
            .dyn_into()
            .map_err(|_| "fetch did not return a Response".to_string())?;

    let status = resp.status();
    let mut headers = Vec::new();
    if let Ok(Some(ct)) = resp.headers().get("content-type") {
        headers.push(("content-type".to_string(), ct));
    }
    if let Ok(Some(cc)) = resp.headers().get("cache-control") {
        headers.push(("cache-control".to_string(), cc));
    }

    let buffer = wasm_bindgen_futures::JsFuture::from(resp.array_buffer().map_err(jserr)?)
        .await
        .map_err(jserr)?;
    let body = js_sys::Uint8Array::new(&buffer).to_vec();

    Ok(ikigai_http::HttpResponse {
        status,
        headers,
        body,
    })
}

/// Non-wasm stub so the crate's lib still type-checks for the native server target
/// (the browser transport is never used there).
#[cfg(not(target_family = "wasm"))]
async fn browser_fetch(
    _request: ikigai_http::HttpRequest,
) -> std::result::Result<ikigai_http::HttpResponse, String> {
    Err("browser fetch is wasm-only".to_string())
}

// A `Resolver` handle to the *same* kernel the Engine drives, kept so the XSLT module's
// `hostCall` session callbacks resolve to full `Representation`s (golden threads + expiry) —
// the host folds those into the transform's cache provenance so a change to the module's
// `src`/`stylesheet` invalidates the cached transform. (wasm only: the native server links
// xslt directly and never calls `hostCall`.)
#[cfg(target_family = "wasm")]
thread_local! {
    static RESOLVER: std::cell::RefCell<Option<Arc<dyn ikigai_resolve::Resolver>>> =
        std::cell::RefCell::new(None);
}

thread_local! {
    // `Rc` so an async eval (`evalLineAsync`) can clone a handle and own it across
    // `.await` points — a `LocalKey::with` borrow can't span an await. Sync callers
    // deref through the `Rc` unchanged.
    static ENGINE: Rc<ikigai_engine::Engine> = {
        // On wasm, drive concurrency on the JS event loop via a spawn_local Spawner:
        // into_scheduled feeds the kernel's compose fan-out, with_spawner feeds the
        // engine's fork/map — so `( a ; b )`, `..`, and compose's markers run
        // concurrently instead of sequentially. (The native server target builds this
        // lib unscheduled; it never touches this thread-local.)
        #[cfg(target_family = "wasm")]
        let kernel = build_kernel("Embedded (Browser)").into_scheduled(Arc::new(WasmSpawner));
        #[cfg(not(target_family = "wasm"))]
        let kernel = Arc::new(build_kernel("Embedded (Browser)"));
        // Stash a Resolver handle to the same kernel for the module callback path before
        // the Engine takes ownership (see `RESOLVER` above and `host_resolve`).
        #[cfg(target_family = "wasm")]
        RESOLVER.with(|r| {
            *r.borrow_mut() = Some(kernel.clone() as Arc<dyn ikigai_resolve::Resolver>);
        });
        let engine = ikigai_engine::Engine::new(kernel);
        #[cfg(target_family = "wasm")]
        let engine = engine.with_spawner(Arc::new(WasmSpawner));
        // Friendly capability profiles, so the in-page terminal reads like the
        // desktop CLI: `cap read-only` attenuates the session to a *read* scope on
        // the file module's jail root (`ws`). The session starts at root identity,
        // so writes work until you narrow; `cap reset` returns to root. With only a
        // read scope held, `sink urn:file:…` is then refused by the file endpoint's
        // path-ACL (`urn:cap:fs:write:…` is not granted) — capability attenuation,
        // enforced in the browser. The jail (`..` segments) is the harder floor
        // beneath it, refused even at root.
        engine.define_cap_profile("read-only", ["urn:cap:fs:read:ws"]);
        Rc::new(engine)
    };
}

/// Set a readable panic hook so Rust panics show up in the browser console.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    #[cfg(target_family = "wasm")]
    install_storage_watcher();
}

/// Watch `localStorage` for **out-of-band** workspace-file changes — another tab of
/// this origin, or page JavaScript — and cut the corresponding golden thread, so a
/// cached read (and any composite over it) recomputes. The browser's analogue of the
/// CLI's filesystem watcher.
///
/// The cross-tab `storage` event fires in *other* documents of the origin (same-tab
/// writes already go through the kernel's auto-cut). The cut itself goes through
/// `urn:kernel:cut` — the cut-as-resource — by evaluating one engine command, so no
/// direct kernel handle is needed.
#[cfg(target_family = "wasm")]
fn install_storage_watcher() {
    use wasm_bindgen::JsCast;
    let Some(window) = web_sys::window() else {
        return;
    };
    let on_storage = wasm_bindgen::closure::Closure::<dyn Fn(web_sys::StorageEvent)>::new(
        |event: web_sys::StorageEvent| {
            // Keys are `ikigai:fs:ws/<path>`; the thread is `urn:file:<path>`.
            if let Some(rel) = event
                .key()
                .as_deref()
                .and_then(|k| k.strip_prefix("ikigai:fs:ws/"))
            {
                let cut = format!("sink urn:kernel:cut urn:file:{rel}");
                ENGINE.with(|engine| {
                    let _ = engine.eval(&cut);
                });
            }
        },
    );
    let _ = window.add_event_listener_with_callback("storage", on_storage.as_ref().unchecked_ref());
    on_storage.forget(); // keep the listener alive for the page's lifetime
}

/// Evaluate one REPL line through the CLI Engine — the full grammar (pipelines `|`,
/// map `..`, fork `( a ; b )`, named `key=value` args, `compose`, `cache`, `list`, …).
/// Returns a JSON string `{ kind, text, cache }`: `kind` is
/// `output` | `error` | `help` | `quit` | `noop`; `cache` is the hit/miss tag
/// (`computed`, `cached`, …) or empty. The page bootstrap calls this once with
/// `source urn:fn:compose src=urn:data:page`; the `<ikigai-cli>` terminal calls it per line.
#[wasm_bindgen(js_name = evalLine)]
pub fn eval(line: String) -> String {
    ENGINE.with(|engine| eval_to_json(engine.eval(&line)))
}

/// Async sibling of [`eval`], returning a `Promise<string>`. This is the path that
/// **unblocks the browser**: it drives the Engine's `eval_async` on the JS event
/// loop (via `future_to_promise`) rather than `block_on`, so a resolution that
/// awaits a JS `Promise` — `fetch`, WebTransport, a timer — parks and lets the loop
/// run instead of deadlocking the thread. In-memory commands resolve immediately;
/// the page's terminal `await`s this for every line.
#[wasm_bindgen(js_name = evalLineAsync)]
pub fn eval_line_async(line: String) -> js_sys::Promise {
    let engine = ENGINE.with(Rc::clone);
    wasm_bindgen_futures::future_to_promise(async move {
        Ok(JsValue::from_str(&eval_to_json(
            engine.eval_async(&line).await,
        )))
    })
}

/// The host end of a dynamically-loaded **module's session**: service one `HostCall`. The
/// XSLT module calls this back across the wasm boundary — passing an encoded
/// [`ModuleReply::HostCall`](ikigai_module::ModuleReply) — to fetch its `src`/`stylesheet`
/// from this kernel *while* the original `urn:xslt:transform` is in flight, and we answer
/// with an encoded [`ModuleCall::HostResult`](ikigai_module::ModuleCall). That re-entrancy
/// is safe: the session capability is cloned (no borrow held across the await) and the outer
/// request is parked (no kernel lock held) while this resolves.
///
/// It resolves through the [`Resolver`](ikigai_resolve::Resolver) handle so the full
/// `Representation` — its golden threads + expiry — reaches [`serve_host_call`], which
/// records it against the in-flight transform's sink before the wire drops the threads
/// (`serde(skip)`), so the transform inherits its `src`/`stylesheet` dependencies (the
/// browser realization of `ikigai-module`'s `HostBridge`).
///
/// wasm only: the module (and thus this callback) exists only in the browser; the native
/// server links xslt directly.
#[cfg(target_family = "wasm")]
#[wasm_bindgen(js_name = hostCall)]
pub fn host_call(reply: Vec<u8>) -> js_sys::Promise {
    use ikigai_resolve::Resolver;
    let resolver = RESOLVER.with(|r| r.borrow().clone());
    wasm_bindgen_futures::future_to_promise(async move {
        let Some(resolver) = resolver else {
            return Err(JsValue::from_str(
                "host_call: kernel resolver not initialised",
            ));
        };
        // The whole session protocol — decode the HostCall, record provenance, encode the
        // HostResult — lives in ikigai-module; the host only supplies how to resolve a
        // sub-request on its kernel (under the carried capability).
        let bytes = ikigai_module::serve_host_call(&reply, move |request, capability| async move {
            resolver
                .issue_as_async(request, &capability)
                .await
                .map(|(representation, _status)| representation)
        })
        .await;
        Ok(js_sys::Uint8Array::from(&bytes[..]).into())
    })
}

/// Encode an [`Action`](ikigai_engine::Action) as the `{ kind, text, cache }` JSON
/// the page's terminal consumes — shared by the sync and async entry points.
fn eval_to_json(action: ikigai_engine::Action) -> String {
    use ikigai_engine::Action;
    let (kind, text, cache) = match action {
        Action::Output(entry) => match entry.result {
            Ok(out) => ("output", out, entry.cache.label().unwrap_or_default()),
            Err(err) => ("error", err, String::new()),
        },
        Action::Help => ("help", ikigai_engine::HELP.to_string(), String::new()),
        Action::Clear => ("clear", String::new(), String::new()),
        Action::Quit => ("quit", String::new(), String::new()),
        Action::Noop => ("noop", String::new(), String::new()),
    };
    serde_json::json!({ "kind": kind, "text": text, "cache": cache }).to_string()
}

// --- Network demo: the WASM wire codec ------------------------------------
//
// The network demo does NOT host the kernel — it talks to a remote one over
// WebTransport. JS does the network I/O; these two functions do the only part
// that must match the server byte-for-byte: the ikigai-wire `Call`/`Reply`
// codec (the same protocol ikigai-ipc and ikigai-quic speak). No kernel here.

/// Build a `verb` request for `iri` with args from a JSON object
/// (`{"name":"value", …}`) — the network terminal's command parser passes args this way.
fn request_from(verb: Verb, iri: &str, args_json: &str) -> std::result::Result<Request, String> {
    let parsed = Iri::parse(iri).map_err(|e| format!("bad iri `{iri}`: {e}"))?;
    let mut request = Request::new(verb, parsed);
    let args: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(args_json).map_err(|e| format!("bad args: {e}"))?;
    for (name, value) in args {
        let bytes = match value {
            serde_json::Value::String(s) => s.into_bytes(),
            other => other.to_string().into_bytes(),
        };
        request = request.with_arg(name, ArgRef::Inline(bytes));
    }
    Ok(request)
}

/// Encode a `compose src=<src>` request (the page pull) — convenience over [`encode_issue`].
#[wasm_bindgen(js_name = encodeComposeCall)]
pub fn encode_compose_call(src: String) -> Vec<u8> {
    let request = Request::new(
        Verb::Source,
        Iri::parse("urn:fn:compose").expect("valid iri"),
    )
    .with_arg("src", ArgRef::Inline(src.into_bytes()));
    ikigai_wire::encode(&ikigai_wire::Call::Issue(request)).expect("encode call")
}

/// Encode a SOURCE request as `Call::Issue` — the terminal's `source <iri> [k=v…]`.
/// `args_json` is `{"name":"value", …}`. Empty bytes on a parse error.
#[wasm_bindgen(js_name = encodeIssue)]
pub fn encode_issue(iri: String, args_json: String) -> Vec<u8> {
    match request_from(Verb::Source, &iri, &args_json) {
        Ok(request) => ikigai_wire::encode(&ikigai_wire::Call::Issue(request)).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Encode an is-cached probe — `Call::IsCached` (`cache <iri>` in the terminal).
#[wasm_bindgen(js_name = encodeIsCached)]
pub fn encode_is_cached(iri: String, args_json: String) -> Vec<u8> {
    match request_from(Verb::Source, &iri, &args_json) {
        Ok(request) => {
            ikigai_wire::encode(&ikigai_wire::Call::IsCached(request)).unwrap_or_default()
        }
        Err(_) => Vec::new(),
    }
}

/// Encode a `list` of the kernel's bindings — `Call::Entries`.
#[wasm_bindgen(js_name = encodeEntries)]
pub fn encode_entries() -> Vec<u8> {
    ikigai_wire::encode(&ikigai_wire::Call::Entries).expect("encode call")
}

/// Decode the server's ikigai-wire `Reply` into `{ kind, text, cache }`.
#[wasm_bindgen(js_name = decodeReply)]
pub fn decode_reply(bytes: Vec<u8>) -> String {
    use ikigai_resolve::CacheStatus;
    let (kind, text, cache) = match ikigai_wire::decode::<ikigai_wire::Reply>(&bytes) {
        Ok(ikigai_wire::Reply::Resolved(repr, status)) => {
            let cache = match status {
                CacheStatus::Hit => "cached",
                CacheStatus::Miss => "computed",
                CacheStatus::Uncacheable => "uncacheable",
            };
            match String::from_utf8(repr.bytes) {
                Ok(text) => ("output", text, cache.to_string()),
                Err(_) => (
                    "error",
                    "reply was not UTF-8 text".to_string(),
                    String::new(),
                ),
            }
        }
        Ok(ikigai_wire::Reply::Cached(hit)) => (
            "output",
            if hit { "cached" } else { "not cached" }.to_string(),
            String::new(),
        ),
        Ok(ikigai_wire::Reply::Entries(Some(entries))) => {
            let text = entries
                .iter()
                .map(|e| format!("{}  → {}", e.pattern, e.endpoint))
                .collect::<Vec<_>>()
                .join("\n");
            (
                "output",
                if text.is_empty() {
                    "(no bindings)".to_string()
                } else {
                    text
                },
                String::new(),
            )
        }
        Ok(ikigai_wire::Reply::Entries(None)) => {
            ("output", "(listing unsupported)".to_string(), String::new())
        }
        Ok(ikigai_wire::Reply::Error(e)) => ("error", e, String::new()),
        Err(e) => ("error", format!("decode failed: {e}"), String::new()),
    };
    serde_json::json!({ "kind": kind, "text": text, "cache": cache }).to_string()
}
