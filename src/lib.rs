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
const PAGE_HTML: &str = r#"
$a{urn:host:identity}

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
"#;

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
fn shape(name: &'static str, html: &'static str) -> FnEndpoint {
    FnEndpoint::new(name, move |_inv: &Invocation<'_>| {
        Ok(Representation::new(
            ReprType::new("text/html").with_param("charset", "utf-8"),
            html.as_bytes().to_vec(),
        )
        .cacheable())
    })
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
fn host_identity() -> FnEndpoint {
    FnEndpoint::new("host-identity", move |inv: &Invocation<'_>| {
        // The per-client file scope a login mints; its `<id>` is the workspace segment.
        let client = inv.capability.scopes().and_then(|scopes| {
            scopes
                .iter()
                .find_map(|s| s.strip_prefix("urn:cap:fs:read:ws/"))
        });
        let body = match client {
            Some(id) => format!(
                "<nav class=\"ik-id ik-id-in\">signed in · <code>ws/{id}</code> \
                 <a href=\"#\" id=\"ik-logout\" class=\"ik-id-link\">Sign out</a></nav>"
            ),
            None => "<nav class=\"ik-id\">\
                 <a href=\"#\" id=\"ik-login\" class=\"ik-id-link\">🔑 Sign in with a passkey</a> \
                 <span class=\"ik-id-hint\">— scopes a private workspace segment to you</span>\
                 </nav>"
                .to_string(),
        };
        Ok(Representation::new(
            ReprType::new("text/html").with_param("charset", "utf-8"),
            body.into_bytes(),
        ))
    })
    .with_description(
        Description::new("host-identity")
            .title("Identity")
            .summary("The passkey sign-in/out affordance, rendered from the session capability.")
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .output("text/html;charset=utf-8"),
    )
}

/// Build the in-page kernel with the host `nature` reported by `urn:host:info`:
/// the demo endpoints, `compose`, and the page shapes, behind the JSON-or-Turtle
/// meta renderer. One kernel drives both the composed page and the terminal, so
/// they share a space and a cache. Public so the WebTransport server
/// (`src/bin/server.rs`) resolves against the same space — with its own nature.
pub fn build_kernel(nature: &'static str) -> Kernel {
    // The reusable functions come from the linked `ikigai-fn` module crate
    // (compiled to wasm32 alongside this lib); this host chains its own page
    // shapes, the in-page terminal mount, the greeter, and `urn:host:info`.
    let space = ikigai_fn::space()
        .bind(Exact::new("urn:demo:greeter"), Greeter)
        .bind(
            Exact::new("urn:demo:web-cli"),
            shape("web-cli", WEB_CLI_HTML),
        )
        .bind(Exact::new("urn:data:page"), shape("page", PAGE_HTML))
        .bind(Exact::new("urn:data:about"), shape("about", ABOUT_HTML))
        .bind(Exact::new("urn:host:info"), host_info(nature))
        .bind(Exact::new("urn:host:identity"), host_identity())
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
        // The interactive runbook (`urn:runbook:*`) — the same module the native CLI
        // links, rendered here as htmx (HATEOAS) HTML.
        Arc::new(ikigai_runbook::space()) as Arc<dyn Space>,
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
    let resp: web_sys::Response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&req))
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
            if let Some(rel) = event.key().as_deref().and_then(|k| k.strip_prefix("ikigai:fs:ws/")) {
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
        Ok(JsValue::from_str(&eval_to_json(engine.eval_async(&line).await)))
    })
}

/// Establish a per-client session identity from a passkey-derived `clientId`, scoping
/// the workspace to `ws/<clientId>`: the session mints `urn:cap:fs:{read,write,delete}:
/// ws/<id>` and resolves under it, so files land under your private segment and another
/// identity's segment is refused by the resolver. Returns the workspace label `ws/<id>`.
///
/// Serverless WebAuthn has no relying party to verify the assertion, so this is identity
/// *selection*, not authenticated login; the isolation is the capability/resource model,
/// not cryptography (the bytes are still visible in devtools). The page calls this after
/// the passkey ceremony, then re-resolves `urn:host:identity` to reflect the new state.
#[wasm_bindgen(js_name = login)]
pub fn login(client_id: String) -> String {
    // The id comes from a hex digest, but defend the path segment regardless — only
    // `[a-z0-9]`, so a crafted id can't smuggle `/`, `..`, or whitespace into a scope.
    let id: String = client_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(32)
        .collect::<String>()
        .to_ascii_lowercase();
    let id = if id.is_empty() { "anon".to_string() } else { id };
    let scopes = [
        format!("urn:cap:fs:read:ws/{id}"),
        format!("urn:cap:fs:write:ws/{id}"),
        format!("urn:cap:fs:delete:ws/{id}"),
    ];
    let cap = Capability::root().attenuate(scopes);
    ENGINE.with(|engine| engine.login(cap));
    format!("ws/{id}")
}

/// Drop back to the anonymous (root) session — the state before any [`login`]. The page
/// calls this on "Sign out", then re-resolves `urn:host:identity`.
#[wasm_bindgen(js_name = logout)]
pub fn logout() {
    ENGINE.with(|engine| engine.logout());
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
