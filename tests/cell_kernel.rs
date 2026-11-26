//! The kernel behind `<ikigai-cell>`, natively: the page's declaration
//! (`<script type="application/ikigai-kernel+json">`) is the WHOLE of what the cells get.
//!
//! Each test parses a declaration with `cell::Declaration::parse`, builds the kernel with
//! the same `cell::build_kernel_with` the page's `cellKernel` export calls, and holds it in
//! the same `cell::Cells`. A run starts with `Cells::begin_run`, as `cellBeginRun` does,
//! and its lines go through `Engine::eval`, the synchronous twin of the `cellEvalAsync` the
//! element drives. One `Cells` across several runs is the page's one shared kernel.
//!
//! The declaration's FORM is pinned twice over: against the example the README documents
//! for a host, and against the one the packaged example page (`cell/example.html`) ships,
//! so neither can drift from what the parser takes.

use std::sync::Arc;

use ikigai_engine::{Action, Engine};
use ikigai_web_demo::cell::{self, Cells, Declaration, DeclaredResource, DEFAULT_TYPE};

/// A transport that must never be reached: a declaration that does not grant the net
/// is refused before any fetch.
struct Unreachable;

#[async_trait::async_trait]
impl ikigai_http::HttpTransport for Unreachable {
    async fn send(
        &self,
        request: ikigai_http::HttpRequest,
    ) -> std::result::Result<ikigai_http::HttpResponse, String> {
        panic!("a cell reached the network: {}", request.url);
    }
}

/// One page's cells: the kernel every cell shares, over a scratch jail.
struct Page {
    cells: Cells,
    jail: tempfile::TempDir,
}

fn page(json: &str) -> Page {
    let declaration = Declaration::parse(json).expect("declaration parses");
    let jail = tempfile::tempdir().expect("tempdir");
    let kernel = cell::build_kernel_with(&declaration, jail.path(), Arc::new(Unreachable));
    Page {
        cells: Cells::new(Arc::new(kernel), declaration, None),
        jail,
    }
}

/// One line in a session: its result, or an error naming a line that answered no output.
fn line_result(engine: &Engine, line: &str) -> Result<String, String> {
    match engine.eval(line) {
        Action::Output(entry) => entry.result,
        _ => Err(format!("`{line}` answered no output")),
    }
}

/// Run one cell's code as the element does: a fresh run, then each command line in turn,
/// stopping at the first error. The result of each line that ran.
fn run(page: &Page, code: &str) -> Vec<Result<String, String>> {
    let session = page.cells.begin_run();
    let mut results = Vec::new();
    for line in cell::commands(code) {
        let result = line_result(&session, &line);
        let failed = result.is_err();
        results.push(result);
        if failed {
            break;
        }
    }
    results
}

fn ok(page: &Page, line: &str) -> String {
    match run(page, line).pop().expect("one line") {
        Ok(text) => text,
        Err(err) => panic!("`{line}` failed: {err}"),
    }
}

fn err(page: &Page, line: &str) -> String {
    match run(page, line).pop().expect("one line") {
        Ok(text) => panic!("`{line}` answered `{text}`, expected a refusal"),
        Err(err) => err,
    }
}

/// The ```json block that follows `marker` in `text`.
fn json_after<'a>(text: &'a str, marker: &str) -> &'a str {
    let at = text.find(marker).unwrap_or_else(|| panic!("no `{marker}`"));
    let rest = &text[at..];
    let open = rest
        .find("```json\n")
        .expect("a ```json block after the marker")
        + 8;
    let close = rest[open..].find("```").expect("the block closes");
    &rest[open..open + close]
}

#[test]
fn the_readme_documents_the_declaration_the_parser_takes() {
    let readme = include_str!("../README.md");
    let declaration =
        Declaration::parse(json_after(readme, "<!-- ikigai-cell declaration -->")).unwrap();
    assert_eq!(declaration.modules, ["fn"]);
    assert_eq!(declaration.capability, Vec::<String>::new());
    assert_eq!(
        declaration.resources,
        [
            DeclaredResource {
                iri: "urn:deck:greeting".into(),
                media_type: DEFAULT_TYPE.into(),
                text: "hello, slides".into(),
                writable: false,
            },
            DeclaredResource {
                iri: "urn:deck:note".into(),
                media_type: DEFAULT_TYPE.into(),
                text: String::new(),
                writable: true,
            },
        ]
    );
}

#[test]
fn the_example_page_ships_a_declaration_the_parser_takes() {
    let html = include_str!("../cell/example.html");
    let open = html
        .find(r#"<script type="application/ikigai-kernel+json">"#)
        .expect("the example declares its kernel");
    let body = &html[open..];
    let body = &body[body.find('>').unwrap() + 1..body.find("</script>").unwrap()];
    let declaration = Declaration::parse(body).expect("the example's declaration parses");
    assert!(declaration.holds("fn"));
    assert!(
        declaration.capability.is_empty(),
        "the example grants nothing"
    );
    assert!(declaration.resources.iter().any(|r| r.writable));
}

#[test]
fn an_empty_declaration_binds_nothing_and_grants_nothing() {
    for json in ["", "  ", "{}"] {
        let declaration = Declaration::parse(json).unwrap();
        assert_eq!(declaration, Declaration::default());
        let page = page(json);
        // No module was declared, so even the pure functions are not there.
        let refusal = err(&page, "source urn:iki:fn:toUpper hello");
        assert!(refusal.contains("no endpoint resolved"), "{refusal}");
        let refusal = err(&page, "source urn:file:notes.txt");
        assert!(refusal.contains("no endpoint resolved"), "{refusal}");
        let refusal = err(&page, "source urn:http:get url=https://example.com/");
        assert!(refusal.contains("no endpoint resolved"), "{refusal}");
    }
}

#[test]
fn a_declared_module_serves_the_cells() {
    let page = page(r#"{ "modules": ["fn"] }"#);
    assert_eq!(ok(&page, "source urn:iki:fn:toUpper hello"), "HELLO");
    // The whole grammar: a pipeline.
    assert_eq!(
        ok(
            &page,
            "source urn:iki:fn:toUpper hello | urn:iki:fn:toUpper"
        ),
        "HELLO"
    );
}

#[test]
fn cells_share_one_kernel_so_a_later_cell_reads_what_an_earlier_one_wrote() {
    let page = page(
        r#"{ "resources": [
              { "iri": "urn:deck:greeting", "text": "hello, slides" },
              { "iri": "urn:deck:note", "writable": true, "type": "text/plain; charset=utf-8" }
           ] }"#,
    );
    assert_eq!(ok(&page, "source urn:deck:greeting"), "hello, slides");
    assert_eq!(ok(&page, "source urn:deck:note"), "");
    // Cell one writes; cell two, a separate run on the same page, reads it back. The
    // empty read above was cached, so this also pins that the write cuts that entry.
    let first = run(&page, "# cell one\nsink urn:deck:note remember this");
    assert!(
        first[0].as_ref().unwrap().contains("wrote 13 bytes"),
        "{first:?}"
    );
    assert_eq!(ok(&page, "source urn:deck:note"), "remember this");
    // A resource the page did not declare writable stays as declared.
    let refusal = err(&page, "sink urn:deck:greeting overwritten");
    assert!(refusal.contains("read-only"), "{refusal}");
    assert_eq!(ok(&page, "source urn:deck:greeting"), "hello, slides");
}

#[test]
fn a_module_is_not_a_grant_the_capability_is() {
    // `fs` and `http` declared, nothing granted: both refuse, and `Unreachable` panics if
    // the HTTP module ever reached its transport.
    let narrow = page(r#"{ "modules": ["fs", "http"] }"#);
    let refusal = err(&narrow, "sink urn:file:notes.txt hello");
    assert!(refusal.starts_with("denied"), "{refusal}");
    let refusal = err(&narrow, "source urn:file:notes.txt");
    assert!(refusal.starts_with("denied"), "{refusal}");
    let refusal = err(&narrow, "source urn:http:get url=https://example.com/");
    assert!(refusal.starts_with("denied"), "{refusal}");

    // The scopes the page declares are the ones the cells hold, and no more.
    let granted =
        page(r#"{ "modules": ["fs"], "capability": ["urn:cap:fs:write:.", "urn:cap:fs:read:."] }"#);
    assert!(ok(&granted, "sink urn:file:notes.txt hello").contains("wrote 5 bytes"));
    assert_eq!(ok(&granted, "source urn:file:notes.txt"), "hello");
    let refusal = err(&granted, "delete urn:file:notes.txt");
    assert!(refusal.starts_with("denied"), "{refusal}");
}

#[test]
fn nothing_typed_into_a_cell_regains_more_than_the_page_declared() {
    let page = page(r#"{ "modules": ["fs"], "capability": ["urn:cap:fs:read:."] }"#);
    // The session is sealed at the declaration: `cap reset` returns to it, not to root,
    // and the typed `login` is refused outright, within a run and in the runs after it.
    for line in ["cap reset", "login urn:cap:fs:write:.", "logout"] {
        let session = page.cells.begin_run();
        let _ = session.eval(line);
        assert_eq!(session.capability(), session.floor(), "after `{line}`");
        let refusal = line_result(&session, "sink urn:file:x.txt pwned").unwrap_err();
        assert!(refusal.starts_with("denied"), "after `{line}`: {refusal}");
        let refusal = err(&page, "sink urn:file:x.txt pwned");
        assert!(
            refusal.starts_with("denied"),
            "a run after `{line}`: {refusal}"
        );
    }
    let session = page.cells.begin_run();
    assert!(session.is_sealed());
    assert_eq!(
        session.floor(),
        ikigai_core::Capability::scoped(["urn:cap:fs:read:."])
    );
    let reset = ok(&page, "cap reset");
    assert!(reset.contains("sealed"), "{reset}");
    // Narrowing still works, within the run.
    let refusal = run(
        &page,
        "cap urn:cap:fs:read:elsewhere\nsource urn:file:x.txt",
    )
    .pop()
    .unwrap()
    .unwrap_err();
    assert!(refusal.starts_with("denied"), "{refusal}");
}

#[test]
fn a_cells_narrowing_lasts_for_its_own_run_only() {
    let page =
        page(r#"{ "modules": ["fs"], "capability": ["urn:cap:fs:write:.", "urn:cap:fs:read:."] }"#);
    // Within the run, the narrowing holds from the line after it.
    let narrowed = run(
        &page,
        "cap urn:cap:fs:read:.\nsink urn:file:notes.txt hello",
    );
    assert!(
        narrowed[1].as_ref().unwrap_err().starts_with("denied"),
        "{narrowed:?}"
    );
    // The next cell starts again from the declared capability.
    assert!(ok(&page, "sink urn:file:notes.txt hello").contains("wrote 5 bytes"));
    // So does the one after a cell that sealed itself narrower.
    let _ = run(&page, "cap seal urn:cap:fs:read:elsewhere");
    assert_eq!(ok(&page, "source urn:file:notes.txt"), "hello");
    // And nothing typed widens past the declaration: no `delete` grant, in any run.
    for code in [
        "cap urn:cap:fs:delete:.\ndelete urn:file:notes.txt",
        "delete urn:file:notes.txt",
    ] {
        let refusal = run(&page, code).pop().unwrap().unwrap_err();
        assert!(refusal.starts_with("denied"), "{code}: {refusal}");
    }
}

#[test]
fn a_lisp_buffer_one_cell_leaves_open_does_not_swallow_the_next() {
    let page = page(r#"{ "modules": ["fn"] }"#);
    // `:lisp` opens the buffer and the form is buffered; the blank line that would submit
    // it is never a command line, so the run ends with the buffer still open.
    let opened = run(&page, ":lisp\n(+ 1 2)");
    assert!(
        opened[0].as_ref().unwrap().contains("lisp mode"),
        "{opened:?}"
    );
    assert_eq!(opened[1], Err("`(+ 1 2)` answered no output".to_string()));
    // The next cell answers, rather than being buffered as Lisp.
    assert_eq!(ok(&page, "source urn:iki:fn:toUpper hello"), "HELLO");
}

#[test]
fn another_tabs_write_to_a_cell_file_cuts_the_cached_read() {
    let page =
        page(r#"{ "modules": ["fs"], "capability": ["urn:cap:fs:write:.", "urn:cap:fs:read:."] }"#);
    assert!(ok(&page, "sink urn:file:notes.txt first").contains("wrote"));
    assert_eq!(ok(&page, "source urn:file:notes.txt"), "first");
    assert_eq!(ok(&page, "cache urn:file:notes.txt"), "cached");
    // The demo's `ws` files are not the cells': a change there cuts nothing here.
    assert_eq!(page.cells.storage_changed("ikigai:fs:ws/notes.txt"), None);
    assert_eq!(
        page.cells.storage_changed("ikigai:fs:cellar/notes.txt"),
        None
    );
    assert_eq!(ok(&page, "cache urn:file:notes.txt"), "cached");
    // Another tab writes the file behind the kernel's back. The storage event for the
    // cell segment cuts its thread, and the next read recomputes from the new value.
    std::fs::write(page.jail.path().join("notes.txt"), "second").unwrap();
    assert_eq!(
        page.cells
            .storage_changed("ikigai:fs:cell/notes.txt")
            .as_deref(),
        Some("urn:file:notes.txt")
    );
    assert_eq!(ok(&page, "cache urn:file:notes.txt"), "not cached");
    assert_eq!(ok(&page, "source urn:file:notes.txt"), "second");
}

#[test]
fn a_storage_key_names_a_file_only_under_its_own_jail() {
    assert_eq!(
        cell::file_thread("ikigai:fs:cell/a/b.txt", cell::CELL_FS_ROOT).as_deref(),
        Some("urn:file:a/b.txt")
    );
    assert_eq!(
        cell::file_thread("ikigai:fs:ws/a.txt", "ws").as_deref(),
        Some("urn:file:a.txt")
    );
    for key in [
        "ikigai:fs:ws/a.txt",
        "ikigai:fs:cellar/a.txt",
        "ikigai:fs:cell/",
        "other",
    ] {
        assert_eq!(cell::file_thread(key, cell::CELL_FS_ROOT), None, "{key}");
    }
    // A page that declared no file module has no cell files to cut.
    assert_eq!(
        page("{}").cells.storage_changed("ikigai:fs:cell/a.txt"),
        None
    );
}

#[test]
fn the_cell_docs_name_the_event_the_element_fires() {
    let source = include_str!("../src/cell.rs");
    let element = include_str!("../dist/ikigai-cell.js");
    let readme = include_str!("../README.md");
    assert!(element.contains("'ikigai-kernel-ready'"));
    assert!(source.contains("`ikigai-kernel-ready` event"));
    for text in [source, readme, element] {
        assert!(!text.contains("ikigai-cell-ready"));
    }
}

#[test]
fn a_malformed_declaration_is_refused_with_a_reason() {
    for (json, says) in [
        ("[]", "must be a JSON object"),
        ("{ not json", "not JSON"),
        (r#"{ "capabilities": [] }"#, "unknown key `capabilities`"),
        (r#"{ "modules": ["lisp"] }"#, "unknown module `lisp`"),
        (r#"{ "modules": ["fn", "fn"] }"#, "declared twice"),
        (r#"{ "modules": "fn" }"#, "array of strings"),
        (r#"{ "capability": ["root"] }"#, "not a `urn:cap:` scope"),
        (
            r#"{ "capability": ["urn:cap:"] }"#,
            "not a `urn:cap:` scope",
        ),
        (r#"{ "resources": [{ "text": "x" }] }"#, "needs an `iri`"),
        (
            r#"{ "resources": [{ "iri": "not an iri" }] }"#,
            "not an IRI",
        ),
        (
            r#"{ "resources": [{ "iri": "urn:a", "type": "plain" }] }"#,
            "not a media type",
        ),
        (
            r#"{ "resources": [{ "iri": "urn:a", "writable": "yes" }] }"#,
            "true or false",
        ),
        (
            r#"{ "resources": [{ "iri": "urn:a", "body": "x" }] }"#,
            "unknown key `body`",
        ),
        (
            r#"{ "resources": [{ "iri": "urn:a" }, { "iri": "urn:a" }] }"#,
            "declared twice",
        ),
    ] {
        let error = Declaration::parse(json).expect_err(json);
        assert!(error.contains(says), "{json}: {error}");
    }
}

#[test]
fn a_cells_code_is_its_command_lines() {
    assert_eq!(
        cell::commands("# a comment\n\n  source urn:deck:greeting  \n\tsink urn:deck:note hi\n"),
        ["source urn:deck:greeting", "sink urn:deck:note hi"]
    );
    assert!(cell::commands("  \n# only a comment\n").is_empty());
}

#[test]
fn the_declaration_reads_back_with_its_defaults_filled_in() {
    let declaration =
        Declaration::parse(r#"{ "resources": [{ "iri": "urn:deck:note", "writable": true }] }"#)
            .unwrap();
    assert_eq!(
        declaration.to_json(),
        serde_json::json!({
            "modules": [],
            "resources": [{ "iri": "urn:deck:note", "type": DEFAULT_TYPE, "writable": true }],
            "capability": [],
        })
    );
}
