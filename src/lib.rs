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
    builtins, ArgSpec, Description, Endpoint, EndpointSpace, Error, Exact, FnEndpoint, Invocation,
    Kernel, MetaRenderer, ReprType, Representation, Result, Verb,
};
use ikigai_vocab::TurtleRenderer;
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
     <code>compose</code>, <code>cache</code>, and <code>list</code>. Try <code>list</code>,
     or <code>source urn:fn:compose src=urn:data:page</code> to compose this very page.</p>
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

/// `urn:demo:split` — splits the `in` argument on commas into a newline list:
/// a *list producer* for the `..` map operator (`split "a,b,c" .. urn:fn:toUpper`).
fn split() -> FnEndpoint {
    FnEndpoint::new("split", |inv: &Invocation<'_>| {
        let items = inv
            .inline_str("in")?
            .split(',')
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Representation::new(
            ReprType::new("text/plain").with_param("charset", "utf-8"),
            items.into_bytes(),
        )
        .cacheable())
    })
    .with_description(
        Description::new("split")
            .title("Split")
            .summary("Splits the `in` argument on commas into newline-separated items.")
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .input(ArgSpec::new("in").summary("comma-separated items"))
            .output("text/plain;charset=utf-8"),
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

/// Build the in-page kernel: the demo endpoints, `compose`, and the page shapes,
/// behind the JSON-or-Turtle meta renderer. One kernel drives both the composed
/// page and the terminal, so they share a space and a cache.
fn build_kernel() -> Kernel {
    let space = EndpointSpace::new()
        .bind(Exact::new("urn:fn:toUpper"), builtins::to_upper())
        .bind(Exact::new("urn:fn:reverseList"), builtins::reverse_list())
        .bind(Exact::new("urn:fn:compose"), builtins::compose())
        .bind(Exact::new("urn:demo:split"), split())
        .bind(Exact::new("urn:demo:greeter"), Greeter)
        .bind(Exact::new("urn:demo:web-cli"), shape("web-cli", WEB_CLI_HTML))
        .bind(Exact::new("urn:data:page"), shape("page", PAGE_HTML))
        .bind(Exact::new("urn:data:about"), shape("about", ABOUT_HTML));
    Kernel::with_meta_renderer(Arc::new(space), Arc::new(JsonOrTurtle))
}

thread_local! {
    static ENGINE: ikigai_engine::Engine = ikigai_engine::Engine::new(build_kernel());
}

/// Set a readable panic hook so Rust panics show up in the browser console.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// Evaluate one REPL line through the CLI Engine — the full grammar (pipelines `|`,
/// map `..`, fork `( a ; b )`, named `key=value` args, `compose`, `cache`, `list`, …).
/// Returns a JSON string `{ kind, text, cache }`: `kind` is
/// `output` | `error` | `help` | `quit` | `noop`; `cache` is the hit/miss tag
/// (`computed`, `cached`, …) or empty. The page bootstrap calls this once with
/// `source urn:fn:compose src=urn:data:page`; the `<ikigai-cli>` terminal calls it per line.
#[wasm_bindgen(js_name = evalLine)]
pub fn eval(line: String) -> String {
    use ikigai_engine::Action;
    let (kind, text, cache) = ENGINE.with(|engine| match engine.eval(&line) {
        Action::Output(entry) => match entry.result {
            Ok(out) => ("output", out, entry.cache.label().unwrap_or_default()),
            Err(err) => ("error", err, String::new()),
        },
        Action::Help => ("help", ikigai_engine::HELP.to_string(), String::new()),
        Action::Quit => ("quit", String::new(), String::new()),
        Action::Noop => ("noop", String::new(), String::new()),
    });
    serde_json::json!({ "kind": kind, "text": text, "cache": cache }).to_string()
}
