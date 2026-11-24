//! The native server's file jail is an ABSOLUTE path (ledger #183), and what that means
//! for the scopes a client carries.
//!
//! `ikigai-net-server` resolves its jail once at startup (`--root DIR`, else
//! `<data home>/web-demo/ws`) and builds the kernel with [`build_kernel_in`] over that
//! canonical path; it used to call `build_kernel`, whose root is the page's virtual `ws`,
//! and so served whatever `ws` sat beside the directory it was launched from.
//!
//! `ikigai-fs` places an ABSOLUTE path-ACL scope against the jail root's own spelling, so
//! over an absolute root a client spells `urn:cap:fs:read:<root>/…`. A RELATIVE scope names
//! a path inside the jail whatever the root (ikigai-fs 0.5.0 and later, ledger #230), so the
//! page's `<id>` holds here too, while the page's old spelling `ws/<id>` now names the
//! jail's own `ws/<id>/` and is refused for `<id>/…` (default-deny, never widened; ledger
//! #4098).
//!
//! No test here changes the working directory, which is the point: an absolute root makes
//! the process's cwd irrelevant.

use ikigai_core::{ArgRef, Capability, Iri, Kernel, Request, Verb};
use ikigai_web_demo::build_kernel_in;

fn source(kernel: &Kernel, target: &str, capability: &Capability) -> Result<String, String> {
    let request = Request::new(Verb::Source, Iri::parse(target).expect("iri"));
    futures::executor::block_on(kernel.issue(request, capability))
        .map(|repr| String::from_utf8(repr.bytes).expect("UTF-8"))
        .map_err(|e| e.to_string())
}

fn sink(kernel: &Kernel, target: &str, content: &str, capability: &Capability) {
    let request = Request::new(Verb::Sink, Iri::parse(target).expect("iri"))
        .with_arg("content", ArgRef::Inline(content.as_bytes().to_vec()));
    futures::executor::block_on(kernel.issue(request, capability))
        .unwrap_or_else(|e| panic!("sink {target}: {e}"));
}

#[test]
fn an_absolute_jail_serves_its_own_files_and_takes_scopes_spelled_against_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = std::fs::canonicalize(dir.path())
        .expect("canonical")
        .join("ws");
    std::fs::create_dir(&root).expect("jail");
    let kernel = build_kernel_in("Remote (test)", root.clone());

    // A write through the kernel lands inside the absolute jail, not beside the cwd.
    sink(
        &kernel,
        "urn:file:abc/note.txt",
        "hello",
        &Capability::root(),
    );
    assert_eq!(
        std::fs::read_to_string(root.join("abc/note.txt")).expect("on disk"),
        "hello"
    );

    // A scope spelled against the absolute root holds.
    let spelled = format!("urn:cap:fs:read:{}/abc", root.display());
    assert_eq!(
        source(
            &kernel,
            "urn:file:abc/note.txt",
            &Capability::scoped([spelled])
        )
        .as_deref(),
        Ok("hello")
    );

    // A relative scope is jail-relative: the page's spelling holds over this jail too.
    assert_eq!(
        source(
            &kernel,
            "urn:file:abc/note.txt",
            &Capability::scoped(["urn:cap:fs:read:abc"])
        )
        .as_deref(),
        Ok("hello")
    );

    // The page's OLD spelling names the jail's own `ws/abc/`, not `abc/`: refused, not widened.
    let old = Capability::scoped(["urn:cap:fs:read:ws/abc"]);
    assert!(
        source(&kernel, "urn:file:abc/note.txt", &old).is_err(),
        "a `ws/…` scope names `ws/…` inside the jail, never `abc/…`"
    );
}
