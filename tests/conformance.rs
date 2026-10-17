//! The module recipe as one test: `ikigai-conformance` walks every endpoint the in-page
//! kernel binds and reports every violation at once — run NATIVELY over the same
//! [`build_kernel_in`] the browser composes. The wasm face is a build target the shared
//! CI checks (`wasm-lib`); the suite does not run under wasm, so the three browser-only
//! cards this crate authors (`xslt-transform`, `jsonld-*`, `shacl-validate` for both browser
//! engines) are typed by hand in `src/lib.rs` and never walked here — on native the same
//! IRIs are served by the linked crates, whose cards are theirs.
//!
//! ## The walk is scoped to this crate's endpoints (`Suite::own`)
//!
//! This host composes eight other crates' endpoints. Those answer for themselves (their
//! own repos run the suite against their own releases), so here the walk is scoped with
//! `Suite::own` to the ids this crate binds ([`OURS`]) and every other id the kernel
//! lists is INHERITED: listed, never read, never fired. [`conforms`] holds the walk to
//! ZERO findings (every own id is kebab-case, so not even NAMES is waived) and pins the
//! inherited list EXACTLY to [`INHERITED`], so a dependency that quietly gains an
//! endpoint turns this red rather than joining the pile unseen, and an own binding
//! missing from [`OURS`] does the same. (Up to `ikigai-conformance` 0.6.1 this file wrote
//! that partition by hand and walked the dependencies too, asserting only that their
//! findings were against their ids; at 0.8.0 that walk reported 50 findings, 15 of them
//! the new REQUIRED check's, all inherited.)
//!
//! Not walking the dependencies also means the walk fires no `Sink` or `Delete` at all:
//! this crate binds none. The kernel is still built over a scratch tempdir (the file
//! module's native backend is `std::fs`), and [`conforms`] asserts the seeded file was
//! never touched.
//!
//! Declared to the suite, stated once:
//!
//! - `pure` + `cacheable` on the greeter and every page shape (`web-cli`, `page`, `docs`,
//!   `demo`, `control`, `control-cards`, `about`, `catalog-cards-xsl`): constants embedded
//!   at build time, cached forever correctly.
//! - `pure` on `clock-now`: its result is a function of the clock minute, bounded by
//!   `cacheable_until` (`Expiry::At`); [`clock_now_serves_both_faces_until_the_minute`]
//!   pins the expiry by hand.
//! - `pure` on `catalog-rdf`: exactly as cacheable as core's `urn:kernel:catalog`, which is
//!   `Never` with no thread (the bound space is fixed for the kernel's life); the cache
//!   keys on the capability fingerprint. Pinned in [`catalog_rdf_declares_the_scope_it_reaches`].
//!
//! REQUIRED (0.8.0) has nothing to drop here: no own action declares a required by-value
//! input (`clock-now`'s `html` is optional; the rest take none). The browser-only cards
//! in `src/lib.rs` do declare required inputs, but they are not bound on native.
//!
//! ## What the suite cannot hold and this file pins by hand
//!
//! - **The home page composes, and is a cache hit** ([`the_home_page_composes_and_is_cached`]):
//!   the demo's central claim, including the `urn:fn:` alias exhibit two levels down.
//! - **Live host state is never cached** ([`live_host_state_is_never_cached`]): PENDING #22
//!   has no `live(id)` spelling, so `host-info`, `host-identity`, `runbook-identity` and
//!   `runbook-timer` are held to `Expiry::Always` here.
//! - **Declared outputs are the media types served** — 0.1.0 compares only RDF faces
//!   (PENDING #11/#31/#79); [`declared_outputs_are_the_media_types_served`] covers every
//!   own action.
//!
//! The `/k/` adapter — runbook steps run under the session capability — has its own file,
//! `tests/k_adapter.rs`.

use std::collections::BTreeSet;
use std::path::Path;

use ikigai_conformance::{rdf, Suite};
use ikigai_core::{
    ArgRef, Capability, Error, Expiry, Iri, Kernel, Representation, Request, Result, Verb,
};
use ikigai_web_demo::build_kernel_in;

/// A file seeded in the jail; nothing in the walk may touch it.
const SCRATCH_FILE: &str = "conformance.txt";
/// What [`SCRATCH_FILE`] holds, before and after the walk.
const SEED: &str = "scratch";

/// This crate's own description ids, in the kernel the page composes: the `Suite::own`
/// scope. A new own binding without a line here is inherited, and fails [`conforms`].
const OURS: &[&str] = &[
    "greeter",
    "clock-now",
    "web-cli",
    "page",
    "docs",
    "demo",
    "control",
    "control-cards",
    "about",
    "catalog-rdf",
    "catalog-cards-xsl",
    "host-info",
    "host-identity",
    "runbook-identity",
    "runbook-timer",
];

/// The dependencies' ids the kernel also binds: inherited, not walked, pinned EXACTLY so a
/// new one is red. Their conformance is their own repos' (each runs the suite).
const INHERITED: &[&str] = &[
    // ikigai-fn
    "toUpper",
    "reverseList",
    "compose",
    "conditional",
    "wrap",
    "split",
    "greet",
    "echo",
    // ikigai-fs
    "file",
    // ikigai-http
    "http-get",
    "http-head",
    "http-post",
    "http-put",
    "http-patch",
    "http-delete",
    // ikigai-rdf
    "rdf-transrept",
    "rdf-union",
    "rdf-diff",
    // ikigai-xslt (native; the browser card is this crate's, typed by hand)
    "xslt-transform",
    // ikigai-jsonld (native; likewise)
    "jsonld-expand",
    "jsonld-flatten",
    "jsonld-compact",
    "jsonld-prune", // since ikigai-jsonld 0.2.2 (ledger #1180); the browser card is in src/lib.rs
    // ikigai-shacl (native; likewise)
    "shacl-validate",
    // ikigai-runbook
    "runbook-basics",
    "runbook-piping",
    "runbook-http",
    "runbook-constraints",
    "runbook-zerotrust",
    "runbook-linkeddata",
    "runbook-transrept",
    "runbook-sniff",
    "runbook-jsonld",
    "runbook-selection",
    "runbook-shacl",
    "runbook-lisp",
    "ik-context",
    "alignment",
    "data-account-shape",
    "data-account-ok",
    "data-account-bad",
    "data-endpoint-shape",
    "action-greet",
    "action-geocode",
    "action-mail",
    // ikigai-vocab
    "ikigai-vocab",
    // ikigai-sniff
    "sniff",
    "transrept-auto",
];

/// The scratch jail: the kernel, and the directory it writes into.
struct Jail {
    dir: tempfile::TempDir,
    kernel: Kernel,
}

fn jail() -> Jail {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join(SCRATCH_FILE), SEED).expect("seed");
    let kernel = build_kernel_in("Embedded (conformance)", dir.path());
    Jail { dir, kernel }
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap_or_else(|e| panic!("`{s}` is a valid IRI: {e}"))
}

fn request(verb: Verb, target: &str, args: &[(&str, &str)]) -> Request {
    let mut request = Request::new(verb, iri(target));
    for (name, value) in args {
        request = request.with_arg(*name, ArgRef::Inline(value.as_bytes().to_vec()));
    }
    request
}

fn issue(kernel: &Kernel, request: Request, capability: &Capability) -> Result<Representation> {
    futures::executor::block_on(kernel.issue(request, capability))
}

fn resolve(kernel: &Kernel, request: Request) -> Representation {
    issue(kernel, request, &Capability::root()).unwrap_or_else(|e| panic!("resolution failed: {e}"))
}

fn source(kernel: &Kernel, target: &str, args: &[(&str, &str)]) -> Representation {
    resolve(kernel, request(Verb::Source, target, args))
}

fn text(repr: &Representation) -> String {
    String::from_utf8(repr.bytes.clone()).expect("UTF-8")
}

fn no_grants() -> Capability {
    Capability::scoped(Vec::<String>::new())
}

/// The suite, configured for this host: scoped to [`OURS`], with the declarations the
/// module docs state.
fn suite() -> Suite {
    let suite = OURS.iter().fold(Suite::new(), |suite, id| suite.own(*id));
    suite
        .pure("greeter")
        .cacheable("greeter")
        .pure("clock-now")
        .pure("catalog-rdf")
        .cacheable("catalog-rdf")
        .pure("web-cli")
        .cacheable("web-cli")
        .pure("page")
        .cacheable("page")
        .pure("docs")
        .cacheable("docs")
        .pure("demo")
        .cacheable("demo")
        .pure("control")
        .cacheable("control")
        .pure("control-cards")
        .cacheable("control-cards")
        .pure("about")
        .cacheable("about")
        .pure("catalog-cards-xsl")
        .cacheable("catalog-cards-xsl")
}

fn sorted(ids: &[impl AsRef<str>]) -> BTreeSet<String> {
    ids.iter().map(|s| s.as_ref().to_string()).collect()
}

#[test]
fn conforms() {
    let jail = jail();
    let report = suite().run_blocking(&jail.kernel);
    // Printed even when clean (`--nocapture`): the report is the record.
    eprintln!("{report}");
    report.assert_clean();
    assert_eq!(
        sorted(&report.walked),
        sorted(OURS),
        "walked exactly this crate's ids"
    );
    assert_eq!(report.endpoints, OURS.len(), "{report}");
    assert_eq!(
        sorted(&report.declared.inherited),
        sorted(INHERITED),
        "the inherited list is pinned: a new dependency endpoint is a decision"
    );
    assert_eq!(
        report.declared.inherited.len(),
        INHERITED.len(),
        "no inherited id listed twice"
    );
    assert_eq!(
        report.checks.skipped().count(),
        0,
        "every check runs: {report}"
    );
    // Nothing the walk fired reached the file module: it is inherited, not walked.
    assert_eq!(
        std::fs::read_to_string(jail.dir.path().join(SCRATCH_FILE)).expect("scratch"),
        SEED,
        "the walk fired nothing at the file module"
    );
    assert!(
        !Path::new(SCRATCH_FILE).exists(),
        "nothing was written beside the sources"
    );
}

/// The demo's central claim, natively: `compose(urn:data:page)` assembles the home page
/// from every marker — the greeter, `urn:iki:fn:toUpper`, the nested about box, and the
/// about box's OLD-name marker (`urn:fn:toUpper`) through the alias table — and, being
/// composed only of build-time constants, is `Expiry::Never` and a cache hit the second
/// time. That last half is a fact about THIS page, not about `compose`: a page that
/// transcluded a file would carry the file's thread.
#[test]
fn the_home_page_composes_and_is_cached() {
    let jail = jail();
    let kernel = &jail.kernel;
    let page = request(
        Verb::Source,
        "urn:iki:fn:compose",
        &[("src", "urn:data:page")],
    );
    let first = resolve(kernel, page.clone());
    let html = text(&first);
    assert_eq!(
        rdf::bare_media_type(&first.repr_type.media_type),
        "text/html"
    );
    for expected in [
        "Hello from the ikigai kernel",
        "RESOURCE-ORIENTED COMPUTING",
        "EVEN THIS NESTED SHAPE WAS COMPOSED",
        "THE OLD NAME STILL RESOLVES",
        "<ikigai-cli></ikigai-cli>",
        "$a{urn:iki:fn:toUpper?in=x}",
    ] {
        assert!(
            html.contains(expected),
            "the composed page carries `{expected}`:\n{html}"
        );
    }
    assert!(
        !html.contains("$a{urn:demo:greeter}"),
        "every marker expanded:\n{html}"
    );
    assert_eq!(first.expiry, Expiry::Never);
    assert!(
        kernel.is_cached(&page, &Capability::root()),
        "composed once, cached"
    );
    assert_eq!(resolve(kernel, page).bytes, first.bytes);
}

/// `urn:time:now`: plain `HH:MM` by default, the blinking-colon markup under `html=true`,
/// plain again under `html=false` (the input is a boolean, not a presence flag), a
/// refusal for anything else — and `Expiry::At` the next minute, the spelling the suite
/// has no declaration for (PENDING #86), so `pure` is declared and the expiry is held here.
#[test]
fn clock_now_serves_both_faces_until_the_minute() {
    let jail = jail();
    let kernel = &jail.kernel;
    let plain = source(kernel, "urn:time:now", &[]);
    assert_eq!(
        rdf::bare_media_type(&plain.repr_type.media_type),
        "text/plain"
    );
    let hhmm = text(&plain);
    assert_eq!(hhmm.len(), 5, "{hhmm}");
    assert_eq!(&hhmm[2..3], ":");
    assert!(matches!(plain.expiry, Expiry::At(_)), "{:?}", plain.expiry);

    let html = source(kernel, "urn:time:now", &[("html", "true")]);
    assert_eq!(
        rdf::bare_media_type(&html.repr_type.media_type),
        "text/html"
    );
    assert!(text(&html).contains("<span class=\"ik-clock-colon\">:</span>"));

    let off = source(kernel, "urn:time:now", &[("html", "false")]);
    assert_eq!(
        rdf::bare_media_type(&off.repr_type.media_type),
        "text/plain"
    );

    match issue(
        kernel,
        request(Verb::Source, "urn:time:now", &[("html", "yes")]),
        &Capability::root(),
    ) {
        Err(Error::InvalidArgument { name, .. }) => assert_eq!(name, "html"),
        other => panic!("a non-boolean is InvalidArgument(\"html\"), got {other:?}"),
    }
}

/// `urn:data:catalog.rdf` declares `urn:cap:kernel:inspect` — the scope the catalog it
/// reads enforces — so it is `Denied` under no grants BEFORE anything runs, serves the
/// declared RDF/XML under root, and is exactly as cacheable as the catalog: `Never`, the
/// catalog's threads and no foreign one, a hit the second time.
#[test]
fn catalog_rdf_declares_the_scope_it_reaches() {
    let jail = jail();
    let kernel = &jail.kernel;
    let spec = kernel
        .describe(&iri("urn:data:catalog.rdf"))
        .expect("describes itself")
        .action_specs()
        .into_iter()
        .find(|a| a.verb == Verb::Source)
        .expect("Source is declared");
    assert_eq!(spec.requires, vec!["urn:cap:kernel:inspect".to_string()]);
    match issue(
        kernel,
        request(Verb::Source, "urn:data:catalog.rdf", &[]),
        &no_grants(),
    ) {
        Err(Error::Denied(_)) => {}
        other => panic!("under no grants: Denied, got {other:?}"),
    }
    let probe = request(Verb::Source, "urn:data:catalog.rdf", &[]);
    let first = resolve(kernel, probe.clone());
    assert_eq!(
        rdf::bare_media_type(&first.repr_type.media_type),
        "application/rdf+xml"
    );
    assert!(text(&first).contains("ik:Endpoint"), "{}", text(&first));
    assert_eq!(first.expiry, Expiry::Never);
    // "Exactly as cacheable as the catalog", spelled so it holds across core's caret. Up to
    // 0.1.72 a cacheable answer carried no thread, so all three sets below were empty. Since
    // 0.1.73 every cacheable answer hangs on its own canonical name (ledger #549, #512 hole
    // A), so this resource carries its own thread and the transrept step's; and the catalog
    // itself hangs on `urn:kernel:bindings` (ledger #510), which this resource must INHERIT —
    // a binding change has to reach the cards. Anything else is a foreign thread: refused.
    let catalog = resolve(kernel, request(Verb::Source, "urn:kernel:catalog", &[]));
    let threads = first.threads();
    assert!(
        catalog.threads().is_subset(threads),
        "inherits every catalog thread {:?}, got {threads:?}",
        catalog.threads()
    );
    let own_steps = ["urn:data:catalog.rdf", "urn:rdf:transrept"];
    let foreign: Vec<_> = threads
        .iter()
        .filter(|t| !catalog.threads().contains(*t))
        .filter(|t| !own_steps.contains(&t.as_str()))
        .collect();
    assert!(
        foreign.is_empty(),
        "carries only the catalog's threads and its own steps' names, foreign: {foreign:?}"
    );
    assert!(kernel.is_cached(&probe, &Capability::root()));
}

/// The HTTP facades, by hand (ikigai-http's, inherited and not walked): every one is
/// `Denied` under no grants — the kernel's floor on the declared `urn:cap:net:*` — before
/// the transport is reached; and
/// under a grant it IS reached, which on a native `build_kernel_in` kernel is the missing
/// transport's refusal, not a denial (and never a real host).
#[test]
fn the_http_facades_are_denied_before_the_transport() {
    let jail = jail();
    let kernel = &jail.kernel;
    let url = [("url", "https://example.com/")];
    for (target, verb) in [
        ("urn:http:get", Verb::Source),
        ("urn:http:head", Verb::Exists),
        ("urn:http:post", Verb::Sink),
        ("urn:http:put", Verb::Sink),
        ("urn:http:patch", Verb::Sink),
        ("urn:http:delete", Verb::Delete),
    ] {
        match issue(kernel, request(verb, target, &url), &no_grants()) {
            Err(Error::Denied(_)) => {}
            other => panic!("{target} under no grants is Denied, got {other:?}"),
        }
    }
    match issue(
        kernel,
        request(Verb::Source, "urn:http:get", &url),
        &Capability::root(),
    ) {
        Err(Error::Denied(e)) => panic!("root reaches the transport, got denied: {e}"),
        Err(e) => assert!(e.to_string().contains("no network transport"), "{e}"),
        Ok(_) => panic!("a build_kernel_in kernel has no network transport"),
    }
}

/// Live host state: `Expiry::Always`, every read — the suite's CACHEABLE probe returns
/// early on an uncacheable result, so a `.cacheable()` added to one of these by mistake
/// would go unreported there (PENDING #22); it fails here. And the identity affordance is
/// rendered from the capability it is resolved under: anonymous offers sign-in, a session
/// scoped to `ws/<id>` shows the chip and bakes the id into every step.
#[test]
fn live_host_state_is_never_cached() {
    let jail = jail();
    let kernel = &jail.kernel;
    for target in [
        "urn:host:info",
        "urn:host:identity",
        "urn:runbook:identity",
        "urn:runbook:timer",
    ] {
        let repr = source(kernel, target, &[]);
        assert_eq!(repr.expiry, Expiry::Always, "{target} is live");
        assert!(!kernel.is_cached(&request(Verb::Source, target, &[]), &Capability::root()));
    }
    let anonymous = text(&source(kernel, "urn:host:identity", &[]));
    assert!(anonymous.contains("id=\"ik-login\""), "{anonymous}");
    assert!(!anonymous.contains("ik-logout"));

    let session = Capability::scoped(["urn:cap:fs:read:ws/abc", "urn:cap:fs:write:ws/abc"]);
    let signed_in = text(
        &issue(
            kernel,
            request(Verb::Source, "urn:host:identity", &[]),
            &session,
        )
        .expect("the affordance needs no grant"),
    );
    assert!(signed_in.contains("<code>ws/abc</code>"), "{signed_in}");
    assert!(signed_in.contains("id=\"ik-logout\""));
    let tab = text(
        &issue(
            kernel,
            request(Verb::Source, "urn:runbook:identity", &[]),
            &session,
        )
        .expect("the tab needs no grant"),
    );
    assert!(
        tab.contains("/k/sink urn:file:abc/secret.txt mine only"),
        "{tab}"
    );
}

/// Every own action: the bare media type it serves with minimal inputs is one of its
/// declared outputs, and it declares at least one. The suite compares only RDF faces
/// (PENDING #11/#31/#79); `as`-selected faces are probed per declared value (#79).
#[test]
fn declared_outputs_are_the_media_types_served() {
    let jail = jail();
    let kernel = &jail.kernel;
    let calls: &[(&str, &[(&str, &str)])] = &[
        ("urn:demo:greeter", &[]),
        ("urn:time:now", &[]),
        ("urn:time:now", &[("html", "true")]),
        ("urn:demo:web-cli", &[]),
        ("urn:data:page", &[]),
        ("urn:data:docs", &[]),
        ("urn:data:demo", &[]),
        ("urn:data:control", &[]),
        ("urn:data:control-cards", &[]),
        ("urn:data:about", &[]),
        ("urn:data:catalog.rdf", &[]),
        ("urn:style:catalog-cards", &[]),
        ("urn:host:info", &[]),
        ("urn:host:identity", &[]),
        ("urn:runbook:identity", &[]),
        ("urn:runbook:timer", &[]),
    ];
    let mut seen = BTreeSet::new();
    for (target, args) in calls {
        let description = kernel
            .describe(&iri(target))
            .unwrap_or_else(|| panic!("{target} describes itself"));
        seen.insert(description.id.clone());
        let spec = description
            .action_specs()
            .into_iter()
            .find(|a| a.verb == Verb::Source)
            .unwrap_or_else(|| panic!("{target} declares Source"));
        let declared: BTreeSet<String> = spec
            .outputs
            .iter()
            .map(|o| rdf::bare_media_type(o))
            .collect();
        assert!(!declared.is_empty(), "{target} declares an output");
        for input in &spec.inputs {
            assert!(input.class.is_some(), "{target}: `{}` is typed", input.name);
        }
        let served = source(kernel, target, args);
        let got = rdf::bare_media_type(&served.repr_type.media_type);
        assert!(
            declared.contains(&got),
            "{target} {args:?} served `{got}`, declared {declared:?}"
        );
    }
    let all: BTreeSet<String> = OURS.iter().map(|s| s.to_string()).collect();
    assert_eq!(seen, all, "every id of this crate was served");
}
