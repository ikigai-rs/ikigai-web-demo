//! The kernel behind `<ikigai-cell>`: the embeddable cell a slide deck, a course page or
//! the Book drops into a page (`dist/ikigai-cell.js`, packaged by `scripts/package-cell.sh`).
//!
//! Every cell on a page runs on ONE kernel, built from the page's **declaration**: a JSON
//! object in a `<script type="application/ikigai-kernel+json">` block. It names the modules
//! the kernel holds, the resources the page itself serves, and the capability every cell
//! runs under. Nothing else is bound or granted:
//!
//! ```json
//! {
//!   "modules": ["fn"],
//!   "resources": [
//!     { "iri": "urn:deck:greeting", "type": "text/plain", "text": "hello, slides" },
//!     { "iri": "urn:deck:note", "writable": true }
//!   ],
//!   "capability": []
//! }
//! ```
//!
//! * `modules` (default none): any of [`MODULES`]. `fs` is the file module jailed to its
//!   own root (`cell`, a segment of `localStorage` in the page, apart from the demo's `ws`),
//!   and `http` is the HTTP module on the browser's `fetch`. Each still checks its own
//!   `urn:cap:fs:*` or `urn:cap:net:*` grant, so naming a module does not grant it.
//! * `resources` (default none): resources the page serves from memory, readable by every
//!   cell. `type` defaults to `text/plain;charset=utf-8`, `text` to empty. A resource is
//!   written (`sink`) only where the page declared `"writable": true`, so an earlier cell
//!   can leave something for a later one to read.
//! * `capability` (default none): the `urn:cap:` scopes the session holds. The engine is
//!   SEALED at it, so `cap reset`, `login` and `logout` can never rise above what the page
//!   declared; `cap <scope…>` can still narrow it, for the rest of that cell's run only.
//!
//! The cells share the KERNEL (its declared resources, its file jail and its cache), so a
//! later cell reads what an earlier one wrote. They do not share the engine SESSION: each
//! run starts a fresh one at the declared capability ([`Cells::begin_run`]), so a cell's
//! `cap` narrowing, `cap seal` or open `:lisp` buffer ends with its run and never changes
//! what the next cell does.
//!
//! Unknown keys, unknown modules and malformed entries are refused with a message, never
//! ignored: a typo (`"capabilities"`) must not silently change what the cells may do.

use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ikigai_core::{
    ArgSpec, Capability, Description, Endpoint, EndpointSpace, Error, Exact, Fallback, Invocation,
    Iri, Kernel, ReprType, Representation, Result, Space, Spawner, UriTemplate, Verb,
};

/// The modules a declaration may name, in the order the kernel consults them.
pub const MODULES: [&str; 6] = ["fn", "rdf", "sniff", "vocab", "fs", "http"];

/// The `type` a declared resource has when the declaration names none.
pub const DEFAULT_TYPE: &str = "text/plain;charset=utf-8";

/// The file module's jail in a cell kernel: the `cell` segment of `localStorage` in the
/// page (`ikigai:fs:cell/<path>`), kept apart from the demo page's `ws`.
pub const CELL_FS_ROOT: &str = "cell";

/// The most bytes one `sink` may write to a declared resource.
pub const MAX_WRITE: usize = 1024 * 1024;

/// A resource the page declares and serves from memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclaredResource {
    /// Its name, as the cells write it (`urn:deck:greeting`).
    pub iri: String,
    /// The media type its representation carries.
    pub media_type: String,
    /// Its initial content.
    pub text: String,
    /// Whether a cell may `sink` a new value into it.
    pub writable: bool,
}

/// A page's kernel declaration, parsed and checked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Declaration {
    /// The modules the kernel holds, from [`MODULES`], in declaration order.
    pub modules: Vec<String>,
    /// The resources the page serves.
    pub resources: Vec<DeclaredResource>,
    /// The `urn:cap:` scopes every cell runs under.
    pub capability: Vec<String>,
}

impl Declaration {
    /// Parse a declaration from the JSON text of the page's
    /// `<script type="application/ikigai-kernel+json">` block. An empty (or whitespace)
    /// text is the empty declaration: no modules, no resources, no authority.
    pub fn parse(json: &str) -> std::result::Result<Self, String> {
        if json.trim().is_empty() {
            return Ok(Self::default());
        }
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("declaration is not JSON: {e}"))?;
        let object = value
            .as_object()
            .ok_or("declaration must be a JSON object")?;
        for key in object.keys() {
            if !["modules", "resources", "capability"].contains(&key.as_str()) {
                return Err(format!(
                    "declaration has an unknown key `{key}`; it takes `modules`, `resources` \
                     and `capability`"
                ));
            }
        }
        let mut declaration = Self::default();
        for module in strings(object.get("modules"), "modules")? {
            if !MODULES.contains(&module.as_str()) {
                return Err(format!(
                    "unknown module `{module}`; a cell kernel can hold {}",
                    MODULES.join(", ")
                ));
            }
            if declaration.modules.contains(&module) {
                return Err(format!("module `{module}` is declared twice"));
            }
            declaration.modules.push(module);
        }
        for scope in strings(object.get("capability"), "capability")? {
            if !scope.starts_with("urn:cap:") || scope.len() == "urn:cap:".len() {
                return Err(format!(
                    "capability scope `{scope}` is not a `urn:cap:` scope; a declaration \
                     grants named scopes only, never root"
                ));
            }
            declaration.capability.push(scope);
        }
        let mut seen = BTreeSet::new();
        let resources = match object.get("resources") {
            None => Vec::new(),
            Some(serde_json::Value::Array(items)) => items.clone(),
            Some(_) => return Err("`resources` must be an array".to_string()),
        };
        for (index, item) in resources.iter().enumerate() {
            let resource = resource(item).map_err(|e| format!("resources[{index}]: {e}"))?;
            if !seen.insert(resource.iri.clone()) {
                return Err(format!("resource `{}` is declared twice", resource.iri));
            }
            declaration.resources.push(resource);
        }
        Ok(declaration)
    }

    /// Whether the declaration names `module`.
    pub fn holds(&self, module: &str) -> bool {
        self.modules.iter().any(|m| m == module)
    }

    /// The declaration as the JSON the element reports in its `ikigai-kernel-ready` event:
    /// every default filled in, so a host reads what the kernel actually holds.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "modules": self.modules,
            "resources": self.resources.iter().map(|r| serde_json::json!({
                "iri": r.iri,
                "type": r.media_type,
                "writable": r.writable,
            })).collect::<Vec<_>>(),
            "capability": self.capability,
        })
    }
}

fn strings(
    value: Option<&serde_json::Value>,
    key: &str,
) -> std::result::Result<Vec<String>, String> {
    match value {
        None => Ok(Vec::new()),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("`{key}` must be an array of strings"))
            })
            .collect(),
        Some(_) => Err(format!("`{key}` must be an array of strings")),
    }
}

fn resource(item: &serde_json::Value) -> std::result::Result<DeclaredResource, String> {
    let object = item.as_object().ok_or("must be an object")?;
    for key in object.keys() {
        if !["iri", "type", "text", "writable"].contains(&key.as_str()) {
            return Err(format!(
                "unknown key `{key}`; a resource takes `iri`, `type`, `text` and `writable`"
            ));
        }
    }
    let iri = object
        .get("iri")
        .and_then(|v| v.as_str())
        .ok_or("needs an `iri` string")?;
    Iri::parse(iri).map_err(|e| format!("`{iri}` is not an IRI: {e}"))?;
    let text_of = |key: &str, default: &str| match object.get(key) {
        None => Ok(default.to_string()),
        Some(serde_json::Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(format!("`{key}` must be a string")),
    };
    let media_type = text_of("type", DEFAULT_TYPE)?;
    let media_type = media_type_of(&media_type)
        .ok_or_else(|| format!("`{media_type}` is not a media type (`type/subtype;k=v`)"))?
        .canonical();
    let writable = match object.get("writable") {
        None => false,
        Some(serde_json::Value::Bool(b)) => *b,
        Some(_) => return Err("`writable` must be true or false".to_string()),
    };
    Ok(DeclaredResource {
        iri: iri.to_string(),
        media_type,
        text: text_of("text", "")?,
        writable,
    })
}

/// `type/subtype;k=v;…` as a [`ReprType`], or `None` when it is not one. Lower-cases the
/// type and parameter names, as media types compare.
fn media_type_of(text: &str) -> Option<ReprType> {
    let mut parts = text.split(';').map(str::trim);
    let essence = parts.next()?.to_ascii_lowercase();
    let (kind, sub) = essence.split_once('/')?;
    let token = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "!#$&-^_.+".contains(c))
    };
    if !token(kind) || !token(sub) {
        return None;
    }
    let mut repr = ReprType::new(essence.clone());
    for param in parts {
        let (key, value) = param.split_once('=')?;
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
        if !token(&key) || !token(value) {
            return None;
        }
        repr = repr.with_param(key, value);
    }
    Some(repr)
}

/// A declared resource, held in memory. A read is cached under the resource's own golden
/// thread, so the kernel's auto-cut on a `sink` to the same name is what makes the next
/// read see the new value.
struct Held {
    iri: String,
    media_type: ReprType,
    writable: bool,
    value: Mutex<Vec<u8>>,
}

#[async_trait]
impl Endpoint for Held {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        match inv.request.verb {
            Verb::Source => {
                let bytes = self.value.lock().expect("held value").clone();
                Ok(Representation::new(self.media_type.clone(), bytes)
                    .cacheable()
                    .depends_on(self.iri.as_str()))
            }
            Verb::Sink if !self.writable => Err(Error::Denied(
                format!(
                    "{} is read-only: the page did not declare it writable",
                    self.iri
                )
                .into(),
            )),
            Verb::Sink => {
                let bytes = inv.collect(inv.content_body("content").await?).await?;
                if bytes.len() > MAX_WRITE {
                    return Err(Error::InvalidArgument {
                        name: "content".to_string(),
                        detail: format!("{} bytes is over the {MAX_WRITE}-byte limit", bytes.len()),
                    });
                }
                let written = bytes.len();
                *self.value.lock().expect("held value") = bytes.to_vec();
                Ok(Representation::new(
                    ReprType::new("text/plain").with_param("charset", "utf-8"),
                    format!("wrote {written} bytes to {}", self.iri).into_bytes(),
                ))
            }
            other => Err(Error::Endpoint(format!(
                "a declared resource does not support the {other:?} verb"
            ))),
        }
    }

    fn name(&self) -> &str {
        "declared-resource"
    }

    fn describe(&self) -> Description {
        let description = Description::new("declared-resource")
            .title("Declared resource")
            .summary(if self.writable {
                "A resource this page declares for its cells; a cell may write it."
            } else {
                "A resource this page declares for its cells; read-only."
            })
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .output(self.media_type.to_string());
        if self.writable {
            description.verb(Verb::Sink).input(
                ArgSpec::new("content")
                    .summary("The new value (Sink only).")
                    .class("http://www.w3.org/2001/XMLSchema#string")
                    .optional(),
            )
        } else {
            description
        }
    }
}

/// The kernel a declaration describes, with the file module (if declared) jailed at
/// `file_root` and the HTTP module (if declared) on `transport`.
pub fn build_kernel_with(
    declaration: &Declaration,
    file_root: impl Into<std::path::PathBuf>,
    transport: Arc<dyn ikigai_http::HttpTransport>,
) -> Kernel {
    let mut held = EndpointSpace::new();
    for r in &declaration.resources {
        held = held.bind(
            Exact::new(r.iri.as_str()),
            Held {
                iri: r.iri.clone(),
                media_type: media_type_of(&r.media_type).expect("checked when parsed"),
                writable: r.writable,
                value: Mutex::new(r.text.clone().into_bytes()),
            },
        );
    }
    // The page's own resources answer first, so a declared name is never shadowed.
    let mut spaces: Vec<Arc<dyn Space>> = vec![Arc::new(held)];
    let file_root = file_root.into();
    for module in &declaration.modules {
        let space: Arc<dyn Space> = match module.as_str() {
            "fn" => Arc::new(ikigai_fn::space()),
            "rdf" => Arc::new(ikigai_rdf::space()),
            "sniff" => Arc::new(ikigai_sniff::space()),
            "vocab" => Arc::new(ikigai_vocab::space()),
            "fs" => Arc::new(EndpointSpace::new().bind(
                UriTemplate::parse(ikigai_fs::FILE_TEMPLATE).expect("valid file template"),
                ikigai_fs::FileEndpoint::new(file_root.clone()).cacheable(),
            )),
            "http" => Arc::new(ikigai_http::space(transport.clone())),
            other => unreachable!("module `{other}` was checked when parsed"),
        };
        spaces.push(space);
    }
    Kernel::with_meta_renderer(
        Arc::new(Fallback::new(spaces)),
        Arc::new(crate::JsonOrTurtle),
    )
    .with_clock(crate::host_clock())
}

/// [`build_kernel_with`] on the target's defaults: the `cell` jail and the default
/// transport (`fetch` in the page; none on native).
pub fn build_kernel(declaration: &Declaration) -> Kernel {
    build_kernel_with(declaration, CELL_FS_ROOT, crate::default_transport())
}

/// A session for one cell's run: the declared scopes, sealed, so nothing typed into a
/// cell can regain more.
pub fn engine(
    resolver: impl ikigai_resolve::Resolver + 'static,
    declaration: &Declaration,
) -> ikigai_engine::Engine {
    let engine = ikigai_engine::Engine::with_identity(
        resolver,
        Capability::scoped(declaration.capability.clone()),
    );
    engine.seal();
    engine
}

/// The command lines a cell's code holds, in order: each non-blank line that is not a
/// `#` comment, trimmed.
pub fn commands(code: &str) -> Vec<String> {
    code.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !ikigai_engine::script::is_comment(line))
        .map(str::to_string)
        .collect()
}

/// The page's cells: ONE kernel, shared by every cell, and the session the current run
/// evaluates in. [`begin_run`](Self::begin_run) replaces the session with a fresh one at
/// the declared capability, so what one cell does to its session (`cap` narrowing,
/// `cap seal`, an open `:lisp` buffer) lasts for its own run only, while what it does to
/// the kernel (a `sink` to a declared resource or a file) is there for the next cell.
pub struct Cells {
    kernel: Arc<Kernel>,
    declaration: Declaration,
    spawner: Option<Arc<dyn Spawner>>,
    session: std::cell::RefCell<Rc<ikigai_engine::Engine>>,
}

impl Cells {
    /// The cells over `kernel`, built from `declaration`; `spawner`, if any, drives the
    /// engine's forks and maps (the kernel's own fan-out is the kernel's to schedule).
    pub fn new(
        kernel: Arc<Kernel>,
        declaration: Declaration,
        spawner: Option<Arc<dyn Spawner>>,
    ) -> Self {
        let session = Rc::new(Self::session_for(&kernel, &declaration, &spawner));
        Self {
            kernel,
            declaration,
            spawner,
            session: std::cell::RefCell::new(session),
        }
    }

    fn session_for(
        kernel: &Arc<Kernel>,
        declaration: &Declaration,
        spawner: &Option<Arc<dyn Spawner>>,
    ) -> ikigai_engine::Engine {
        let engine = engine(Arc::clone(kernel), declaration);
        match spawner {
            Some(spawner) => engine.with_spawner(Arc::clone(spawner)),
            None => engine,
        }
    }

    /// Start a cell's run: a fresh session at the declared capability, with no Lisp
    /// buffer open. Returns it, as [`session`](Self::session) does until the next run.
    pub fn begin_run(&self) -> Rc<ikigai_engine::Engine> {
        let fresh = Rc::new(Self::session_for(
            &self.kernel,
            &self.declaration,
            &self.spawner,
        ));
        *self.session.borrow_mut() = Rc::clone(&fresh);
        fresh
    }

    /// The session the current run evaluates in.
    pub fn session(&self) -> Rc<ikigai_engine::Engine> {
        Rc::clone(&self.session.borrow())
    }

    /// The declaration the kernel was built from.
    pub fn declaration(&self) -> &Declaration {
        &self.declaration
    }

    /// A `localStorage` key changed outside this document (another tab of the origin):
    /// when it is a file in the cell jail (`ikigai:fs:cell/<path>`), cut that file's thread
    /// so a cached read, and anything over it, recomputes. Returns the thread it cut, if
    /// any. Any other key (the demo's `ws` files among them) is not the cells' and is left
    /// alone. The cut is the host's, made on the kernel directly: a cell's sealed session
    /// does not hold `urn:cap:kernel:cut`, and must not need it.
    pub fn storage_changed(&self, key: &str) -> Option<String> {
        if !self.declaration.holds("fs") {
            return None;
        }
        let thread = file_thread(key, CELL_FS_ROOT)?;
        self.kernel.cut(thread.as_str());
        Some(thread)
    }
}

/// The thread (`urn:file:<path>`) of the file a `localStorage` key holds under the jail
/// `root` (`ikigai:fs:<root>/<path>`), or `None` for a key outside that jail.
pub fn file_thread(key: &str, root: &str) -> Option<String> {
    let rel = key
        .strip_prefix("ikigai:fs:")?
        .strip_prefix(root)?
        .strip_prefix('/')?;
    (!rel.is_empty()).then(|| format!("urn:file:{rel}"))
}
