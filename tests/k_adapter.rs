//! The `/k/` adapter, natively: a runbook step is TEXT in an `hx-get`, and the page's one
//! glue bridge runs it through the Engine under the SESSION capability. The runbook crate
//! has no Sink, no cap and nothing to gate (ikigai-cli PENDING §5), so the check that a
//! step naming a gated resource is refused — typed, before the resource is reached — lives
//! in the hosts. Here the steps are not copied: they are EXTRACTED from the HTML the
//! runbook resources render, exactly as htmx would read them, and run in order through the
//! same `Engine::eval` the page's `evalLineAsync` export drives, with the profiles the
//! page installs and the scopes the passkey bridge mints.
//!
//! ## The jail is `ws`, so the process runs in a scratch directory
//!
//! The page's scopes spell the jail root: `urn:cap:fs:read:ws` (the `read-only` profile),
//! `urn:cap:fs:write:ws/<id>` (a login). ikigai-fs matches a scope's directory against the
//! ROOT-JOINED path, so those scopes hold only over a jail whose root is literally `ws` —
//! the virtual `localStorage` root in the page, `./ws` relative to the process on native.
//! Each test therefore takes a process-wide lock, moves into its own tempdir and builds the
//! kernel over `ws` there; nothing here touches the working tree.
//!
//! ## Two gates, two witnesses each
//!
//! A capability can fail a step at either of two gates, and they leave different traces:
//!
//! - **The kernel's floor** (declared = enforced): a session holding NO grant under the
//!   family the action declares (`read-only` holds no `urn:cap:fs:write:*`) is refused
//!   BEFORE dispatch. Core reports that as a [`TraceEvent`] tagged [`DENIED_NOTE`] with
//!   `started == None` — the one event that names something which never ran.
//! - **The module's ACL** (the parameterized rule): a session holding a grant under the
//!   family but not for THIS path (`ws/abc` against `someone-else/…`) passes the floor,
//!   and the file endpoint's own longest-prefix rule refuses inside `invoke`. That refusal
//!   is a typed `Denied` too, but it leaves NO trace event — core records an invocation
//!   only after `invoke` returns `Ok` (reported for the hub) — so the witness is the disk:
//!   no file, no directory.
//!
//! Typed, twice: the kernel, issued the same request under the same session capability,
//! answers `Error::Denied`; and the text the adapter shows begins `denied:` — the engine's
//! `Entry.result` is `Result<String, String>`, so the taxonomy reaches the page as prose
//! only (the kernel is where the type is provable).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use ikigai_core::{
    ArgRef, Capability, Error, Iri, Kernel, Request, TraceEvent, Tracer, Verb, DENIED_NOTE,
};
use ikigai_engine::{Action, Engine};
use ikigai_web_demo::{build_kernel_in, define_cap_profiles};

/// Every event the kernel reported: computed invocations and pre-dispatch denials.
#[derive(Default)]
struct Recorder(Mutex<Vec<TraceEvent>>);

impl Tracer for Recorder {
    fn record(&self, event: TraceEvent) {
        self.0.lock().expect("recorder").push(event);
    }
}

impl Recorder {
    fn events(&self) -> Vec<TraceEvent> {
        self.0.lock().expect("recorder").clone()
    }
}

/// The process's working directory is one, so the tests that move it run one at a time.
static CWD: Mutex<()> = Mutex::new(());

/// A session over a scratch jail: the kernel (shared with the engine), the engine with the
/// page's profiles, and the recorder. Holds the cwd lock for its life.
struct Session {
    _cwd: MutexGuard<'static, ()>,
    _dir: tempfile::TempDir,
    jail: PathBuf,
    kernel: Arc<Kernel>,
    engine: Engine,
    trace: Arc<Recorder>,
}

fn session() -> Session {
    let cwd = CWD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let jail = dir.path().join("ws");
    std::fs::create_dir(&jail).expect("jail");
    std::env::set_current_dir(dir.path()).expect("chdir into the scratch directory");
    let kernel = Arc::new(build_kernel_in("Embedded (adapter)", "ws"));
    let trace = Arc::new(Recorder::default());
    kernel.set_tracer(trace.clone() as Arc<dyn Tracer>);
    let engine = Engine::new(Arc::clone(&kernel));
    define_cap_profiles(&engine);
    Session {
        _cwd: cwd,
        _dir: dir,
        jail,
        kernel,
        engine,
        trace,
    }
}

/// What the adapter does with a line: `evalLineAsync(cmd)` → `{kind, text}`.
fn run(engine: &Engine, cmd: &str) -> Result<String, String> {
    match engine.eval(cmd) {
        Action::Output(entry) => entry.result,
        Action::Clear => Ok(String::new()),
        _ => panic!("`{cmd}` is not an output"),
    }
}

/// Reverse the runbook's attribute escaping (`esc` in ikigai-runbook).
fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The commands a runbook page offers, in order — every `hx-get="/k/<command>"` the HTML
/// carries except the tab strip's own navigation and the `clear` button: the same slice
/// the page's `htmx:beforeRequest` handler takes (`path.slice(path.indexOf('/k/') + 3)`).
fn steps(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("hx-get=\"/k/") {
        rest = &rest[at + "hx-get=\"/k/".len()..];
        let end = rest.find('"').expect("a closed attribute");
        let cmd = unescape(&rest[..end]);
        rest = &rest[end..];
        if cmd == "clear" || cmd.starts_with("source urn:runbook:") {
            continue;
        }
        out.push(cmd);
    }
    out
}

/// The same Sink the step issues, at the kernel, under `capability` — the typed answer.
fn typed_sink(kernel: &Kernel, target: &str, capability: &Capability) -> Error {
    let request = Request::new(Verb::Sink, Iri::parse(target).expect("iri"))
        .with_arg("content", ArgRef::Inline(b"nope".to_vec()));
    futures::executor::block_on(kernel.issue(request, capability))
        .err()
        .unwrap_or_else(|| panic!("{target} resolved under {capability:?}"))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The Identity tab, signed in as the page signs in (`sink urn:host:login` with the three
/// `ws/<id>` scopes the passkey bridge mints): three steps, extracted and run. The write
/// inside the segment lands, the read returns it, and the write OUTSIDE the segment is
/// refused by the file module's ACL — typed at the kernel, `denied:` through the adapter —
/// with nothing on disk and (since the refusal is the module's, inside `invoke`) no trace
/// event at all: the floor passed, because the session does hold a `write` grant.
#[test]
fn an_identity_step_outside_the_segment_is_denied_and_never_reaches_the_disk() {
    let s = session();
    let login = run(
        &s.engine,
        "sink urn:host:login urn:cap:fs:read:ws/abc urn:cap:fs:write:ws/abc urn:cap:fs:delete:ws/abc",
    )
    .expect("login is a session operation");
    assert!(login.starts_with("logged in"), "{login}");

    let tab = run(&s.engine, "source urn:runbook:identity as=text/html").expect("the tab renders");
    let steps = steps(&tab);
    assert_eq!(
        steps,
        vec![
            "sink urn:file:abc/secret.txt mine only",
            "source urn:file:abc/secret.txt",
            "sink urn:file:someone-else/secret.txt nope",
        ],
        "the walkthrough bakes the session id into its steps"
    );

    run(&s.engine, &steps[0]).expect("a write inside the segment lands");
    assert_eq!(read(&s.jail.join("abc/secret.txt")), "mine only");
    assert_eq!(run(&s.engine, &steps[1]).expect("readable"), "mine only");

    let before = s.trace.events().len();
    let refused = run(&s.engine, &steps[2]).expect_err("outside the segment is refused");
    assert!(refused.starts_with("denied:"), "{refused}");
    assert!(
        !s.jail.join("someone-else").exists(),
        "nothing was written outside the segment"
    );
    let after = s.trace.events();
    assert_eq!(
        after.len(),
        before,
        "the module's ACL refused inside invoke, which core does not trace: {after:?}"
    );
    assert!(
        matches!(
            typed_sink(
                &s.kernel,
                "urn:file:someone-else/secret.txt",
                &s.engine.capability()
            ),
            Error::Denied(_)
        ),
        "the same request under the session capability is a typed Denied at the kernel"
    );
}

/// The ZeroTrust tab, run end to end through the adapter with the page's `read-only`
/// profile: the write before `cap read-only` lands; the write after it is refused at the
/// KERNEL's floor (no `urn:cap:fs:write:*` grant at all) — the trace holds exactly one
/// `denied` event with nothing started, and the file is untouched; the read still resolves;
/// the jail escape is refused even at full authority; the narrowed net grant refuses the
/// host outside it. The two network steps reach the transport, which on native is a stub —
/// an error, not a denial, and the assertion says which.
#[test]
fn the_zerotrust_tab_runs_through_the_adapter_and_the_gated_write_is_refused_before_dispatch() {
    let s = session();
    let tab = run(&s.engine, "source urn:runbook:zerotrust as=text/html").expect("renders");
    let steps = steps(&tab);
    assert_eq!(steps.len(), 10, "{steps:?}");
    assert_eq!(steps[1], "cap read-only");
    assert_eq!(steps[2], "sink urn:file:note.txt nope");

    run(&s.engine, &steps[0]).expect("1 · write a file");
    let note = s.jail.join("note.txt");
    assert_eq!(read(&note), "remember the milk");

    run(&s.engine, &steps[1]).expect("2 · cap read-only");
    assert_eq!(
        s.engine.capability(),
        Capability::root().attenuate(["urn:cap:fs:read:ws"]),
        "the page's profile"
    );
    let before = s.trace.events().len();
    let refused = run(&s.engine, &steps[2]).expect_err("3 · write → denied");
    assert!(refused.starts_with("denied:"), "{refused}");
    assert_eq!(read(&note), "remember the milk", "untouched");
    let events = s.trace.events();
    assert_eq!(
        events.len(),
        before + 1,
        "one event, the denial: {events:?}"
    );
    let denial = &events[before];
    assert_eq!(denial.target, "urn:file:note.txt");
    assert_eq!(
        denial.notes,
        vec![(DENIED_NOTE.to_string(), "urn:cap:fs:write:*".to_string())],
        "refused at the floor, for the scope the action declares"
    );
    assert!(
        denial.started.is_none() && denial.ended.is_none(),
        "nothing ran: {denial:?}"
    );
    assert!(matches!(
        typed_sink(&s.kernel, "urn:file:note.txt", &s.engine.capability()),
        Error::Denied(_)
    ));

    assert_eq!(
        run(&s.engine, &steps[3]).expect("4 · read → ok"),
        "remember the milk"
    );
    let escape = run(&s.engine, &steps[4]).expect_err("5 · escape jail → denied");
    assert!(
        escape.contains("not allowed"),
        "the jail's own refusal: {escape}"
    );
    run(&s.engine, &steps[5]).expect("6 · cap reset");
    assert_eq!(s.engine.capability(), Capability::root());
    run(&s.engine, &steps[4]).expect_err("the jail holds at root too");

    run(&s.engine, &steps[6]).expect("7 · grant one host");
    let allowed = run(&s.engine, &steps[7]).expect_err("8 reaches the transport");
    assert!(
        allowed.contains("wasm-only"),
        "not a denial — the native fetch stub: {allowed}"
    );
    let elsewhere = run(&s.engine, &steps[8]).expect_err("9 · fetch elsewhere → denied");
    assert!(elsewhere.starts_with("denied:"), "{elsewhere}");
    run(&s.engine, &steps[9]).expect("10 · cap reset");
}
