//! The ikigai kernel, running in the browser via WebAssembly.
//!
//! A single in-page [`Kernel`] binds a few endpoints and answers requests.
//! `issue` (SOURCE) and `describe` (META) are `async` — wasm-bindgen turns them
//! into JS `Promise`s, and the browser's event loop is the executor (no threads,
//! no tokio: the executor-agnostic-async design). The kernel persists, so its
//! content-addressed cache persists across calls. The page declares which
//! resource fills each slot; the kernel assembles the page.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use ikigai_core::{
    builtins, ArgRef, Capability, Description, Endpoint, EndpointSpace, Exact, Invocation, Iri,
    Kernel, ReprType, Representation, Request, Result, Verb,
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

static KERNEL: OnceLock<Kernel> = OnceLock::new();

fn kernel() -> &'static Kernel {
    KERNEL.get_or_init(|| {
        let space = EndpointSpace::new()
            .bind(Exact::new("urn:fn:toUpper"), builtins::to_upper())
            .bind(Exact::new("urn:fn:reverseList"), builtins::reverse_list())
            .bind(Exact::new("urn:demo:greeter"), Greeter);
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
