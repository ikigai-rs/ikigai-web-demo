//! The module recipe as one test: `ikigai-conformance` walks every endpoint the in-page
//! kernel binds and reports every violation at once — run NATIVELY over the same
//! [`build_kernel_in`] the browser composes. The wasm face is a build target the shared
//! CI checks (`wasm-lib`); the suite does not run under wasm, so the three browser-only
//! cards this crate authors (`xslt-transform`, `jsonld-*`, `shacl-validate` over the JS
//! engines) are typed by hand in `src/lib.rs` and never walked here — on native the same
//! IRIs are served by the linked crates, whose cards are theirs.
//!
//! ## The fixture is a scratch jail
//!
//! The walk FIRES `Sink` and `Delete` on `urn:file:*`, and the native file backend is
//! `std::fs`, so the kernel is built over a tempdir, never the working tree. Two files are
//! seeded: one for the `file` binding (`Source` reads it, `Sink` overwrites it with `x`:
//! bindings are per entry, the suite's PENDING #2) and a `flag.txt` holding `true` so
//! `urn:iki:fn:conditional` has a boolean resource to branch on.
//!
//! ## What the walk sees, and what this file partitions
//!
//! This host composes eight other crates' endpoints, and the suite walks everything bound
//! (PENDING #17). Several of those have adopted the suite in their own repos, but the
//! pinned releases predate it, so at the versions the lock holds they report findings this
//! crate cannot fix. [`assert_findings`] therefore holds THIS crate's ids ([`OURS`]) to
//! ZERO findings — every id here is kebab-case, so not even NAMES is waived — and every
//! other finding to an id in the pinned [`INHERITED`] set; a finding against an id neither
//! list knows is red, and so is an endpoint the catalog lists that neither list names.
//! When a dependency is bumped to its conformant release, the inherited count in the
//! printed report drops on its own — all but two lines, which are this FIXTURE's doing and
//! will outlive every bump: `conditional` branches on `urn:file:flag.txt` and
//! `xslt-transform` reads `urn:data:catalog.rdf`, so under no grants each is refused by the
//! gate of the resource it reaches (`urn:cap:fs:read:*`, `urn:cap:kernel:inspect`) and
//! ENFORCED reports the declarer's scope against the caller (the suite's PENDING #87). True
//! of the composed host; the message names the declarer; not a defect of either crate.
//!
//! Declared to the suite, stated once:
//!
//! - `pure` + `cacheable` on the greeter and every page shape (`web-cli`, `page`, `docs`,
//!   `demo`, `control`, `control-cards`, `about`, `catalog-cards-xsl`): constants embedded
//!   at build time, cached forever correctly.
//! - `pure` on `clock-now`: its result is a function of the clock minute, bounded by
//!   `cacheable_until` (`Expiry::At`) rather than a thread — the spelling the suite has no
//!   declaration for (PENDING #86); [`clock_now_serves_both_faces_until_the_minute`] pins
//!   the expiry by hand.
//! - `pure` on `catalog-rdf`: exactly as cacheable as core's `urn:kernel:catalog`, which is
//!   `Never` with no thread (the bound space is fixed for the kernel's life); the cache
//!   keys on the capability fingerprint. Pinned in [`catalog_rdf_declares_the_scope_it_reaches`].
//! - `pure` on the constants other crates serve here (`ikigai-vocab`, the runbook's
//!   `ik-context`, `alignment` and `data-*`): a suite declaration about behavior over THIS
//!   kernel, not a fix to those crates.
//! - `opt_out` of `httpGet` Source and `httpHead` Exists: the fetch transport this host
//!   installs is the browser's, and its native stub reaches no origin, so the cache probe
//!   cannot resolve them. The opt-out also drops ENFORCED (PENDING #21);
//!   [`the_http_facades_are_denied_before_the_transport`] pins it by hand.
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

use ikigai_conformance::{rdf, Fixture, Report, Suite};
use ikigai_core::{
    ArgRef, Capability, Error, Expiry, Iri, Kernel, Representation, Request, Result, Verb,
};
use ikigai_web_demo::build_kernel_in;

/// The file the `file` fixture binds; seeded, then overwritten by the walk's Sink.
const SCRATCH_FILE: &str = "conformance.txt";
/// A boolean resource for `urn:iki:fn:conditional`'s `if`.
const FLAG_FILE: &str = "flag.txt";

/// A small Turtle graph for the RDF operators' `content` fixtures.
const TTL: &str = "<urn:example:conformance> <http://purl.org/dc/terms/title> \"t\" .\n";
/// A JSON-LD document that expands to one triple.
const JSONLD: &str = r#"{"@id":"urn:example:conformance","http://purl.org/dc/terms/title":"t"}"#;

/// This crate's own description ids, in the kernel the page composes. A new binding without
/// a line here fails [`conforms`]: the walk's endpoint set is pinned to the two lists.
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

/// The dependencies' ids the walk also sees, at the versions the lock pins. Their findings
/// are theirs.
const INHERITED: &[&str] = &[
    // ikigai-fn 0.2.0
    "toUpper",
    "reverseList",
    "compose",
    "conditional",
    "wrap",
    "split",
    "greet",
    "echo",
    // ikigai-fs 0.1.5 (0.1.6, the adopted release, is unpublished)
    "file",
    // ikigai-http 0.1.7
    "httpGet",
    "httpHead",
    "httpPost",
    "httpPut",
    "httpPatch",
    "httpDelete",
    // ikigai-rdf 0.1.4
    "rdf-transrept",
    "rdf-union",
    "rdf-diff",
    // ikigai-xslt 0.1.1 (native; the browser card is this crate's, typed by hand)
    "xslt-transform",
    // ikigai-jsonld 0.1.1 (native; likewise)
    "jsonld-expand",
    "jsonld-flatten",
    "jsonld-compact",
    // ikigai-shacl 0.1.1 (native; likewise)
    "shacl-validate",
    // ikigai-runbook 0.1.13
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
    // ikigai-vocab (the lock's)
    "ikigai-vocab",
    // ikigai-sniff 0.1.1
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
    std::fs::write(dir.path().join(SCRATCH_FILE), "scratch").expect("seed");
    std::fs::write(dir.path().join(FLAG_FILE), "true").expect("seed the flag");
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

/// The suite, configured for this host (the declarations the module docs state). The
/// SHACL fixture's data graph is read from the kernel so the walk validates the runbook's
/// own sample rather than a copy of it.
fn suite(kernel: &Kernel) -> Suite {
    let account_ok = text(&source(kernel, "urn:data:account-ok", &[]));
    Suite::new()
        .fixture(Fixture::new("file", Verb::Source).binding("path", SCRATCH_FILE))
        .fixture(Fixture::new("compose", Verb::Source).arg("src", "urn:data:page"))
        .fixture(
            Fixture::new("conditional", Verb::Source)
                .arg("if", format!("urn:file:{FLAG_FILE}"))
                .arg("then", "urn:data:about")
                .arg("else", "urn:demo:greeter"),
        )
        .fixture(
            Fixture::new("rdf-transrept", Verb::Source)
                .arg("content", TTL)
                .arg("as", "text/turtle"),
        )
        .fixture(
            Fixture::new("rdf-union", Verb::Source)
                .arg("content", TTL)
                .arg("with", TTL),
        )
        .fixture(
            Fixture::new("rdf-diff", Verb::Source)
                .arg("content", TTL)
                .arg("with", TTL),
        )
        // The Catalog page's own transform: catalog → RDF/XML → cards.
        .fixture(
            Fixture::new("xslt-transform", Verb::Source)
                .arg("src", "urn:data:catalog.rdf")
                .arg("stylesheet", "urn:style:catalog-cards")
                .arg("as", "text/html"),
        )
        .fixture(
            Fixture::new("jsonld-expand", Verb::Source)
                .arg("content", JSONLD)
                .arg("base", "urn:example:"),
        )
        .fixture(
            Fixture::new("jsonld-flatten", Verb::Source)
                .arg("content", JSONLD)
                .arg("base", "urn:example:"),
        )
        .fixture(
            Fixture::new("jsonld-compact", Verb::Source)
                .arg("content", JSONLD)
                .arg("context", "urn:data:ik-context")
                .arg("base", "urn:example:"),
        )
        .fixture(
            Fixture::new("shacl-validate", Verb::Source)
                .arg("data", account_ok)
                .arg("shapes", "urn:data:account-shape")
                .arg("as", "text/turtle"),
        )
        .fixture(
            Fixture::new("transrept-auto", Verb::Source)
                .arg("content", TTL)
                .arg("as", "text/turtle"),
        )
        .opt_out(
            "httpGet",
            Some(Verb::Source),
            "the fetch transport this host installs is the browser's; its native stub \
             reaches no origin. Denial under no grants pinned by hand",
        )
        .opt_out(
            "httpHead",
            Some(Verb::Exists),
            "same: the browser fetch transport has no native path",
        )
        .pure("greeter")
        .cacheable("greeter")
        .pure("clock-now")
        .pure("catalog-rdf")
        .cacheable("catalog-rdf")
        .pure("ikigai-vocab")
        .pure("ik-context")
        .pure("alignment")
        .pure("data-account-shape")
        .pure("data-account-ok")
        .pure("data-account-bad")
        .pure("data-endpoint-shape")
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

/// Every id the kernel's catalog describes.
fn walked_ids(kernel: &Kernel) -> BTreeSet<String> {
    kernel
        .entries()
        .expect("an enumerable root")
        .iter()
        .filter(|e| !e.pattern.starts_with("urn:kernel:"))
        .map(|e| {
            kernel
                .describe_pattern(&e.pattern)
                .unwrap_or_else(|| panic!("`{}` describes itself", e.pattern))
                .id
        })
        .collect()
}

/// The walk's verdict on THIS crate: no finding of any check against one of `ours`, every
/// other finding against a pinned dependency id, the catalog equal to both lists together,
/// and nothing skipped.
fn assert_findings(report: &Report, kernel: &Kernel, ours: &[&str], inherited: &[&str]) {
    let ours: BTreeSet<&str> = ours.iter().copied().collect();
    let inherited: BTreeSet<&str> = inherited.iter().copied().collect();
    let mut theirs = 0;
    for finding in &report.findings {
        let id = finding.endpoint.as_str();
        assert!(
            !ours.contains(id),
            "a finding against this crate's own endpoint: {finding}"
        );
        assert!(
            inherited.contains(id),
            "a finding against an endpoint this file does not list: {finding}"
        );
        theirs += 1;
    }
    eprintln!(
        "{theirs} inherited finding(s) against {} dependency endpoint(s); 0 against this crate's {}",
        inherited.len(),
        ours.len()
    );
    let expected: BTreeSet<String> = ours.union(&inherited).map(|s| s.to_string()).collect();
    assert_eq!(
        walked_ids(kernel),
        expected,
        "the catalog is exactly the two lists"
    );
    assert_eq!(report.endpoints, expected.len(), "{report}");
    assert_eq!(
        report.checks.skipped().count(),
        0,
        "every check runs: {report}"
    );
}

#[test]
fn conforms() {
    let jail = jail();
    let report = suite(&jail.kernel).run_blocking(&jail.kernel);
    // Printed even when clean (`--nocapture`): the report is the record.
    eprintln!("{report}");
    assert_findings(&report, &jail.kernel, OURS, INHERITED);
    // The walk's Sink landed in the jail, not beside the sources.
    assert_eq!(
        std::fs::read_to_string(jail.dir.path().join(SCRATCH_FILE)).expect("scratch"),
        "x",
        "PIPELINE fired the file Sink with `content=x` into the scratch jail"
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
/// declared RDF/XML under root, and is exactly as cacheable as the catalog: `Never`, no
/// thread, a hit the second time.
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
    assert!(
        first.threads().is_empty(),
        "as threadless as the catalog: {:?}",
        first.threads()
    );
    assert!(kernel.is_cached(&probe, &Capability::root()));
}

/// The opted-out actions, by hand: every HTTP facade is `Denied` under no grants — the
/// kernel's floor on the declared `urn:cap:net:*` — before the (native-stub) transport is
/// reached; and under a grant it IS reached, which is the native stub's error, not a denial.
#[test]
fn the_http_facades_are_denied_before_the_transport() {
    let jail = jail();
    let kernel = &jail.kernel;
    let url = [("url", "https://example.com/")];
    for (target, verb) in [
        ("urn:httpGet", Verb::Source),
        ("urn:httpHead", Verb::Exists),
        ("urn:httpPost", Verb::Sink),
        ("urn:httpPut", Verb::Sink),
        ("urn:httpPatch", Verb::Sink),
        ("urn:httpDelete", Verb::Delete),
    ] {
        match issue(kernel, request(verb, target, &url), &no_grants()) {
            Err(Error::Denied(_)) => {}
            other => panic!("{target} under no grants is Denied, got {other:?}"),
        }
    }
    match issue(
        kernel,
        request(Verb::Source, "urn:httpGet", &url),
        &Capability::root(),
    ) {
        Err(Error::Denied(e)) => panic!("root reaches the transport, got denied: {e}"),
        Err(e) => assert!(e.to_string().contains("wasm-only"), "{e}"),
        Ok(_) => panic!("the native stub cannot fetch"),
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
