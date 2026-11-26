# ikigai-web-demo

The [ikigai](https://crates.io/crates/ikigai-core) resolution kernel running
**in the browser** via WebAssembly — no server, no fetch, no JS framework.

**▶ Live demo: <https://ikigai-rs.github.io/ikigai-web-demo/>**

The page **is** a resource. `index.html` is a near-empty shell that makes one call —
`compose('urn:data:page')` — and drops the result into the body. The kernel resolves
the page *shape* (HTML) and recursively expands every `$a{<iri>}` transclusion marker
in it, resolving each embedded resource through the kernel; a marker may carry
arguments (`$a{urn:iki:fn:toUpper?in="resource-oriented computing"}`) and a transcluded
shape may contain further markers, so composition recurses. Resolution, the endpoints,
the `compose` builtin, and the content-addressed cache all run client-side in WASM. The
client is ~5 lines of glue — the layout *and* its contents come from the kernel.

![The composed page, and the in-page ikigai CLI driving the same kernel](docs/web-cli.png)

The page even carries a live **terminal** — the *same* renderer-agnostic Engine the
desktop `ikigai` REPL uses, compiled to WASM and driving this page's kernel. It's
mounted by composition too: a `$a{urn:demo:web-cli}` marker in the page shape resolves
to an `<ikigai-cli>` element that wires itself up on insertion. Because it shares the
page's kernel and content-addressed cache, typing `source urn:iki:fn:compose
src=urn:data:page` into it returns the very page you're reading — reported `cached`,
since the page already composed it. The whole grammar works in the browser: pipelines
`|`, map `..`, fork `( a ; b )`, named `key=value` args, plus `compose`, `cache`, `cap`, and `list`.

**A namespace migration you can watch happen.** `ikigai-fn` 0.2.0 moved the four names it
owns from `urn:fn:*` into the shared `urn:iki:` namespace, and this host installs a
rewrite table (`Kernel::with_aliases`) in the same release that adopts the bump. So
`source urn:fn:toUpper hello` still answers here, `source urn:iki:fn:toUpper hello` comes
back `cached` off the *same* entry — one cache entry and one golden thread for both names —
and `list` advertises only the new one, so anything that discovers resources by reading the
catalog migrates itself. `source urn:kernel:aliases` reads the table back out with a live
hop count. The about box on the page is the proof in situ: its inner marker still names
`urn:fn:toUpper`, two levels down inside a composed page, and renders anyway.

**And the window is now closing, which is the more interesting half.** `ikigai-fn` 0.2.0
published on 2026-09-04, this host adopted it within the hour, and `ikigai-cli` followed
three days later in 0.1.18 with a table of its own — each at its own cadence, no flag day,
which is exactly what the rule was for. With the other hosts across, the table stops
protecting *hosts* and starts protecting names written down where a host cannot reach them:
`ikigai-runbook` 0.1.13 is a published crate this page links, and it hardcodes `source
urn:fn:toUpper hello`. Which poses the question the exhibit is really about — *when do you
delete an alias?* Discovery has already migrated itself, and the hop count cannot answer,
because the about box fires the rule once per composition on purpose and so it never reaches
zero. A rule like this comes out on a schedule, not on a signal.

A row of **ZeroTrust** buttons above the terminal walks the capability story, enforced
client-side in WASM: `cap read-only` narrows the session to a *read* scope, after which a
`sink urn:file:…` write is refused (`capability does not grant `write``) while reads still
resolve — and the file module's **jail** refuses to escape its root (`../../…`) even at
full authority. The *same* `cap` command and enforcement as the native CLI.

**Two SHACL engines, one resource.** `urn:shacl:validate` is served in the page by the real
[`ikigai-shacl`](https://crates.io/crates/ikigai-shacl) crate (rudof) by default, compiled to
wasm and loaded as a lazy module on the first validation, exactly like the XSLT and JSON-LD
modules (about 1.9 MB over the wire, once). Open the page with **`?shacl=js`** and the same
name is served instead by the JavaScript
[shacl-engine](https://github.com/zazuko/shacl-engine), kept for comparison. The two report
the same verdicts and violations; rudof also gives each violation a `message` and `value`.
The engine is chosen when the kernel is built, so a page never mixes two engines' reports
under one cache entry; the Demo tab's SHACL steps run unchanged on either, and the smoke
gate holds the two to the same verdicts.

It depends on the published [`ikigai-core`](https://crates.io/crates/ikigai-core),
[`ikigai-vocab`](https://crates.io/crates/ikigai-vocab), and
[`ikigai-engine`](https://crates.io/crates/ikigai-engine) crates, so a fresh
checkout builds on its own.

## Prerequisites

- A Rust toolchain (`rustup`).
- The WASM target: `rustup target add wasm32-unknown-unknown`
- `wasm-bindgen-cli`, matching the `wasm-bindgen` version in `Cargo.toml` (`=0.2.108`):
  `cargo install wasm-bindgen-cli --version 0.2.108`
- Something to serve static files over HTTP (the examples use Python 3's built-in
  server). Serving over HTTP matters: the browser needs the `application/wasm`
  MIME type, which opening the file as `file://` does not provide.

## Run it

From the repository root:

```bash
# 1. Compile the library to a raw .wasm (no JS bindings yet). `--lib` matters: the
#    crate also has a native-only server binary, which does not build for wasm.
cargo build --release --lib --target wasm32-unknown-unknown

# 2. Generate the JS glue + processed .wasm into dist/, next to index.html.
wasm-bindgen --target web --out-dir dist \
  target/wasm32-unknown-unknown/release/ikigai_web_demo.wasm

# 3. Serve dist/ over HTTP and open the page.
cd dist && python3 -m http.server 8087 --bind 127.0.0.1
```

Then open <http://127.0.0.1:8087>.

The page fills its slots from the kernel on load, and the **Interactive** section
lets you SOURCE `urn:iki:fn:toUpper` / `urn:iki:fn:reverseList` against your own input.

### After editing `src/lib.rs`

Re-run steps 1–2, then refresh the browser — the running server picks up the new
files. If nothing changed, you only need step 3 (the `dist/` artifacts are reused).

### Alternative: `trunk serve`

[`trunk`](https://trunkrs.dev) can do build + bindgen + serve + live-reload in one
command. It expects to drive its own `index.html` at the crate root, whereas this
demo ships a hand-written `dist/index.html` with an explicit ES-module import — so
the three-step recipe above matches what's in the repo.

## Embed a kernel cell: `<ikigai-cell>`

The same Engine, packaged for any page that wants runnable ikigai in it: a slide deck, a
course page, a book chapter. A cell shows its code (its own text content, editable), a
**Run** button and an output region below. Every cell on a page runs on **one** kernel,
so a later cell reads what an earlier one wrote. The kernel holds only what the page
declares, and every cell runs under only the capability the page grants. No server is
involved, and no file is fetched from any host but the page's own.

```html
<script type="application/ikigai-kernel+json">
{ "modules": ["fn"], "resources": [{ "iri": "urn:deck:note", "writable": true }] }
</script>
<script type="module" src="ikigai/ikigai-cell.<hash>.js"></script>

<ikigai-cell>source urn:iki:fn:toUpper hello</ikigai-cell>
<ikigai-cell>sink urn:deck:note remember this</ikigai-cell>
<ikigai-cell label="Read it back">source urn:deck:note</ikigai-cell>
```

### The files to copy

`scripts/package-cell.sh` writes the package into `dist/cell/`. Run steps 1 and 2 of
[Run it](#run-it) first. The smoke suite runs the script over the CI-built tree before its
tests (`smoke/package-cell.setup.js`), so `pages.yml` tests the package on every push. It
does not deploy it yet: the package is built only after the Pages artifact is uploaded.
Copy **every file** in `dist/cell/` except `index.html` into one directory of your site:

| File | What it is |
|---|---|
| `ikigai-cell.<hash>.js` | The element: the one file your page loads (`<script type="module">`). |
| `ikigai-cell.<hash>.css` | Its stylesheet. The element adds the `<link>` itself, next to its own URL. |
| `ikigai_web_demo.<hash>.js` | The wasm-bindgen glue the element imports. |
| `ikigai_web_demo_bg.<hash>.wasm` | The kernel (about 6.3 MB uncompressed). Serve it as `application/wasm`. |
| `manifest.json` | The hashed name of each file. `entry` names the one to load. |

Each `<hash>` is the first 16 hex digits of the file's SHA-256, so a host can cache them
forever. The files find each other by relative URL, so keep them together and unrenamed.
`index.html` is the example host page the smoke suite runs: a small deck whose arrow keys
change slides, the way reveal.js binds them. The kernel loads when a cell first runs, not
when the page loads.

### The declaration

A page declares its kernel once, as a JSON object in a
`<script type="application/ikigai-kernel+json">` block. A page with no block gets the
empty declaration: nothing bound, nothing granted. Two blocks are an error.

<!-- ikigai-cell declaration -->
```json
{
  "modules": ["fn"],
  "resources": [
    { "iri": "urn:deck:greeting", "text": "hello, slides" },
    { "iri": "urn:deck:note", "writable": true }
  ],
  "capability": []
}
```

- **`modules`** (default none) names the crates the kernel holds, chosen from `fn`
  (ikigai-fn: `toUpper`, `compose`, `each` and the rest), `rdf` (`urn:rdf:transrept`),
  `sniff`, `vocab` (`urn:ikigai:vocab`), `fs` (`urn:file:*`) and `http` (`urn:http:*`, on the
  browser's `fetch`). Declaring a module does not grant it: `fs` still needs
  `urn:cap:fs:*` scopes and `http` needs `urn:cap:net:*`. The file module is jailed to its own
  `localStorage` segment (`ikigai:fs:cell/<path>`), apart from the demo page's `ws`, and it
  persists across reloads of the origin. XSLT, JSON-LD and SHACL are not offered in a cell
  yet: they are lazy module wasms with loaders of their own.
- **`resources`** (default none) are resources the page serves from memory. Each one takes
  an `iri`, a `type` (default `text/plain;charset=utf-8`) and its `text` (default empty).
  A cell may `sink` into one only if it says `"writable": true`. Values live as long as the
  page.
- **`capability`** (default none) lists the `urn:cap:` scopes every cell runs under. The
  session is **sealed** there, so `cap reset`, `login` and `logout` typed into a cell can
  never rise above it, while `cap <scope…>` can still narrow it. There is no way to declare
  root.

An unknown key, an unknown module, a scope outside `urn:cap:` or a malformed resource is
refused with a reason, which every cell shows when it runs. A typo such as
`"capabilities"` is never silently ignored. `tests/cell_kernel.rs` pins this form against
the example above and against the example page.

### The element

A cell's code is its text content, one command per line, in the terminal's grammar
(pipelines `|`, map `..`, fork `( a ; b )`, `key=value` arguments). Blank lines and `#`
comments are skipped. Run executes the lines in order and stops at the first error. Runs are
serialized across the page in the order they are asked for.

| Attribute | Effect |
|---|---|
| `label` | The cell's accessible name (default `ikigai cell N`). |
| `readonly` | The code cannot be edited. |
| `autorun` | Run once when the cell is inserted. |
| `escape-to` | The id of the element Escape moves focus to. |

Keyboard: **Run** is a real `<button>`, reached with Tab and pressed with Enter or Space.
**Ctrl+Enter** (or **⌘+Enter**) in the code runs it. Every key event inside a cell stops
there, so a host that binds keys on the document (reveal.js turns arrow keys into slide
changes) never sees them. **Escape** returns focus to the host: to the `escape-to` element,
else the nearest ancestor with a `tabindex`, else the document. The output is an
`aria-live="polite"` status region, replaced on each run. Its fresh-output mark and the
running indicator are the cell's only motion, and both stop under
`prefers-reduced-motion: reduce`.

Colors and type come from the host's tokens, each falling back to the next:
`--code-bg` then `--bg` for the background, `--code-fg` then `--fg` for the text,
`--accent` for the frame, the button and the focus ring, and `--font-mono` for the face.
With none set, the defaults pass WCAG AA contrast on their own. Errors are marked by weight
and an `error ·` label, never by color alone.

### Events

All of them bubble and are `composed`.

| Event | Fired on | `detail` |
|---|---|---|
| `ikigai-kernel-ready` | `document`, once | `{ declaration }`: what the kernel holds, every default filled in |
| `ikigai-kernel-error` | `document`, once | `{ message }`: why the wasm or the declaration was refused |
| `ikigai-cell-run` | the cell | `{ code, commands }`: the code, and the command lines about to run |
| `ikigai-cell-result` | the cell | `{ ok, results }`: one `{ command, kind, text, cache }` per line run |
| `ikigai-cell-escape` | the cell | `{}`: Escape handed focus back to the host |

The element also exposes `run()`, which returns a promise of the same `{ ok, results }`.

The demo page's terminal (`<ikigai-cli>`) renders its answers with the same code
(`renderAction`, exported by `dist/ikigai-cell.js`). It runs on the demo's own kernel, not
on a cell kernel.

## Network demo: pull the page over WebTransport

The page above hosts the kernel *in the browser*. The companion demo
(`dist/net.html`) does **not** — it pulls the **same** `urn:data:page` from a kernel
running in a separate **server process**, over
[WebTransport](https://developer.mozilla.org/en-US/docs/Web/API/WebTransport)
(HTTP/3 over QUIC). The browser sends the `compose` request as `ikigai-wire` bytes —
the *same* protocol `ikigai-ipc`/`ikigai-quic` speak — the server composes the page
remotely, and streams the assembled HTML back. Only the wire codec runs in WASM; the
kernel is on the other end.

The composed page's terminal is **live too**: each command (`source <iri>`, `compose
<iri>`, `cache <iri>`, `list`) is its own `ikigai-wire` Call on a fresh WebTransport
stream, resolved by the remote kernel — so `source urn:iki:fn:toUpper hi` twice shows
`computed` then `cached`, the *server's* cache.

```bash
# build the WASM glue first (steps 1–2 above), then:
cargo run --bin ikigai-net-server     # serves on https://127.0.0.1:4433 (loopback only),
                                      # prints a cert hash and its file jail (default
                                      # ~/.ikigai/web-demo/ws; `-- 4433 --root /abs/dir`)
cd dist && python3 -m http.server 8087
# open http://127.0.0.1:8087/net.html — paste the printed cert hash (or use #cert=<hash>)
```

The cert is self-signed and **rotates each run**; the browser trusts it via
WebTransport's `serverCertificateHashes` (no CA), so paste the current hash.
`urn:file:*` on the server is jailed to an absolute directory, resolved once at startup
and printed: `--root DIR`, or `<data home>/web-demo/ws` (`~/.ikigai/web-demo/ws`) by
default, created if missing. It no longer depends on where the server was launched.

The server authenticates no client and resolves every call as **root**, so it binds
**loopback only** (`127.0.0.1`) by default. `--bind ADDR` widens it, and the server says
so at start: anyone who reaches that address with a QUIC client can read and write the
jail. A browser session must also come from an allowed `Origin`, by default the pages'
own (`http://127.0.0.1:8087`, `http://localhost:8087`, `https://ikigai-rs.github.io`);
`--allow-origin ORIGIN` (repeatable, `*` for any) replaces that list. A session from any
other origin is refused with 403 at the handshake. That stops a page on another site, not
a non-browser client, which can send any `Origin` or none.
The server can also **fetch**: `urn:http:get` and its siblings run on a native `ureq`
transport that returns a redirect rather than following it, so the HTTP endpoint re-checks
the `urn:cap:net:*` grant on every hop. A call with no net grant is refused as before; one
sent as root (the terminal's default) reaches whatever host it names.
Needs Chrome/Edge (or recent Firefox). It isn't on GitHub Pages — Pages is
static-only, and this needs a running server — so it's run-it-yourself.

### The wire version (2026-10-19: wire v9 and the failure classes)

The page's codec and the server are the **same build of this crate** (`ikigai-wire` 0.1.66,
wire v9): WebTransport gives a browser no say over the ALPN, so nothing negotiates a version
here, and `pages.yml` rebuilds `dist/` from the crate on every push. What that means in
practice:

- **The page sends only `Issue`, `IsCached` and `Entries`**, the wire's first three variants,
  so it still works against a v8 (or older) server. The server additionally answers a v9
  `IssueWith` from another kernel: the request runs under the run context it carried
  (retries left, time left, added to this server's clock), as the `IssueAs` it extends; a
  recording peer gets its spans back as with `IssueTraced`.
- **A failure crosses typed, at v9.** `decodeReply` renders the five classes wire v9 added
  (`retry after 30000 ms: …`, `over budget: …`, `refused: …`, `unsupported: …`, and
  `nesting budget exceeded: …`) exactly as the kernel displays them — the shared
  failure-class vectors. A `dist/` built from the crate **before** this pin shows such a
  failure as `decode failed: …`: rebuild it (steps 1-2 above) after pulling.
- With `ikigai-http` 0.3.0 the HTTP endpoints type an origin's answers by one status table:
  401/403 are `refused` (were `denied`), 429 is `retry after …` (and is not transient), 500
  is `endpoint error` (was `unavailable`). The terminal shows whichever the endpoint returns;
  nothing in this host matches on them.

## Conformance

The host passes [`ikigai-conformance`](https://crates.io/crates/ikigai-conformance) —
the module recipe as one test (`tests/conformance.rs`), run natively over the same kernel
the page composes: every endpoint this crate binds has typed inputs, declared faces and
scopes, and its cacheability pinned (constants cached forever, live session state never,
the clock until the next minute). The crates it composes are walked too; their findings at
the pinned versions are recorded, not waived, and drop as each publishes its conformant
release. `tests/k_adapter.rs` runs the runbook's own steps through the `/k/` adapter under
the session capability — the write a `read-only` session or another identity's segment
refuses is a typed `Denied`, before anything touches the disk.

## What's in here

- `src/lib.rs` — the in-browser kernel: binds the demo endpoints (incl. `compose`)
  and the page shapes, and exposes `evalLine` (the CLI Engine over the kernel) plus
  the network demo's wire codec (`encodeComposeCall` / `decodeReply`) to JS via
  `wasm-bindgen`.
- `src/bin/server.rs` — the WebTransport kernel server (native; `wtransport` +
  `ikigai-wire`), reusing the same `build_kernel()` so it composes the same page.
- `dist/index.html` — the in-browser demo (one `compose('urn:data:page')` → body).
- `dist/net.html` — the network demo (pull `urn:data:page` over WebTransport).
- `src/cell.rs`, `dist/ikigai-cell.js`, `dist/ikigai-cell.css`, `cell/example.html`,
  `scripts/package-cell.sh` — the embeddable `<ikigai-cell>`: its kernel declaration,
  the element, its stylesheet, the example host page and the packaging script.
  The committed files under `dist/`; the `.js`/`.wasm` are generated and gitignored.

## Accessibility

[ACCESSIBILITY.md](ACCESSIBILITY.md) says what the composed page, the in-page terminal, the runbook
and `net.html` aim at (WCAG 2.2 AA, evaluation in progress, no conformance claim), what has been
tested, the barriers known today, and how to report one: an issue with the `accessibility` label,
or private vulnerability reporting for anything sensitive.

## License

Demo code; same license as the ikigai crates (MIT OR Apache-2.0).
