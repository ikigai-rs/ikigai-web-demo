//! The ikigai kernel, running in the browser via WebAssembly.
//!
//! A single in-page [`Kernel`] binds a few endpoints and answers requests.
//! `issue` (SOURCE) and `describe` (META) are `async` — wasm-bindgen turns them
//! into JS `Promise`s, and the browser's event loop is the executor (no threads,
//! no tokio: the executor-agnostic-async design). The kernel persists, so its
//! content-addressed cache persists across calls. The page is itself a resource:
//! one `compose` call resolves the page shape and recursively expands its
//! `$a{<iri>}` markers, so the kernel assembles the whole page from one pull.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use ikigai_core::{
    builtins, ArgRef, Capability, Description, Endpoint, EndpointSpace, Exact, FnEndpoint,
    Invocation, Iri, Kernel, ReprType, Representation, Request, Result, Verb,
};
use ikigai_vocab::TurtleRenderer;
use wasm_bindgen::prelude::*;

/// A demo endpoint with a rich self-description (so the META slot has something
/// to show). It greets you from inside the browser.
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

/// `urn:data:page` — the page *shape*. A `compose` source: HTML whose
/// `$a{<iri>}` markers transclude other resources in this kernel. The browser
/// issues one `compose(urn:data:page)` and the kernel assembles the whole page,
/// recursively — the `urn:data:about` marker is itself a shape with its own marker.
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
"#;

/// `urn:data:about` — a nested shape the page transcludes, which itself
/// transcludes another resource. Proof that composition recurses.
const ABOUT_HTML: &str = r#"<aside class="about">
  <h3>Composition recurses</h3>
  <p>This box is a separate resource (<code>urn:data:about</code>) the page pulled in — and
     it pulled in another:
     <b>$a{urn:fn:toUpper?in="even this nested shape was composed"}</b>.</p>
</aside>"#;

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

static KERNEL: OnceLock<Kernel> = OnceLock::new();

fn kernel() -> &'static Kernel {
    KERNEL.get_or_init(|| {
        let space = EndpointSpace::new()
            .bind(Exact::new("urn:fn:toUpper"), builtins::to_upper())
            .bind(Exact::new("urn:fn:reverseList"), builtins::reverse_list())
            .bind(Exact::new("urn:fn:compose"), builtins::compose())
            .bind(Exact::new("urn:demo:greeter"), Greeter)
            .bind(Exact::new("urn:data:page"), shape("page", PAGE_HTML))
            .bind(Exact::new("urn:data:about"), shape("about", ABOUT_HTML));
        // A Meta renderer makes `describe(... , "text/plain"|"text/turtle")` work.
        Kernel::with_meta_renderer(Arc::new(space), Arc::new(TurtleRenderer))
    })
}

/// Set a readable panic hook so Rust panics show up in the browser console.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// SOURCE `target` with `input` as the `in` argument; return the representation.
#[wasm_bindgen]
pub async fn issue(target: String, input: String) -> std::result::Result<String, JsValue> {
    let iri = Iri::parse(&target).map_err(js_err)?;
    let request =
        Request::new(Verb::Source, iri).with_arg("in", ArgRef::Inline(input.into_bytes()));
    run(request).await
}

/// META `target`, rendered to `as_type` (e.g. `text/plain`, `text/turtle`).
#[wasm_bindgen]
pub async fn describe(target: String, as_type: String) -> std::result::Result<String, JsValue> {
    let iri = Iri::parse(&target).map_err(js_err)?;
    let request = Request::new(Verb::Meta, iri).with_arg("as", ArgRef::Inline(as_type.into_bytes()));
    run(request).await
}

/// Compose the resource named by `src`: resolve it, then recursively expand its
/// `$a{<iri>}` markers through the kernel. One pull assembles a whole page —
/// `compose("urn:data:page")` (Brian's `apply:compose+src=urn:data:page`).
#[wasm_bindgen]
pub async fn compose(src: String) -> std::result::Result<String, JsValue> {
    let request = Request::new(Verb::Source, Iri::parse("urn:fn:compose").map_err(js_err)?)
        .with_arg("src", ArgRef::Inline(src.into_bytes()));
    run(request).await
}

async fn run(request: Request) -> std::result::Result<String, JsValue> {
    let representation = kernel()
        .issue(request, &Capability::root())
        .await
        .map_err(js_err)?;
    String::from_utf8(representation.bytes).map_err(js_err)
}

fn js_err(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}
