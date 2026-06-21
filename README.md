# ikigai-web-demo

The [ikigai](https://crates.io/crates/ikigai-core) resolution kernel running
**in the browser** via WebAssembly — no server, no fetch, no JS framework.

**▶ Live demo: <https://ikigai-rs.github.io/ikigai-web-demo/>**

The page **is** a resource. `index.html` is a near-empty shell that makes one call —
`compose('urn:data:page')` — and drops the result into the body. The kernel resolves
the page *shape* (HTML) and recursively expands every `$a{<iri>}` transclusion marker
in it, resolving each embedded resource through the kernel; a marker may carry
arguments (`$a{urn:fn:toUpper?in="resource-oriented computing"}`) and a transcluded
shape may contain further markers, so composition recurses. Resolution, the endpoints,
the `compose` builtin, and the content-addressed cache all run client-side in WASM. The
client is ~5 lines of glue — the layout *and* its contents come from the kernel.

![The composed page, and the in-page ikigai CLI driving the same kernel](docs/web-cli.png)

The page even carries a live **terminal** — the *same* renderer-agnostic Engine the
desktop `ikigai` REPL uses, compiled to WASM and driving this page's kernel. It's
mounted by composition too: a `$a{urn:demo:web-cli}` marker in the page shape resolves
to an `<ikigai-cli>` element that wires itself up on insertion. Because it shares the
page's kernel and content-addressed cache, typing `source urn:fn:compose
src=urn:data:page` into it returns the very page you're reading — reported `cached`,
since the page already composed it. The whole grammar works in the browser: pipelines
`|`, map `..`, fork `( a ; b )`, named `key=value` args, plus `compose`, `cache`, `cap`, and `list`.

A row of **ZeroTrust** buttons above the terminal walks the capability story, enforced
client-side in WASM: `cap read-only` narrows the session to a *read* scope, after which a
`sink urn:file:…` write is refused (`capability does not grant `write``) while reads still
resolve — and the file module's **jail** refuses to escape its root (`../../…`) even at
full authority. The *same* `cap` command and enforcement as the native CLI.

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
# 1. Compile the crate to a raw .wasm (no JS bindings yet).
cargo build --release --target wasm32-unknown-unknown

# 2. Generate the JS glue + processed .wasm into dist/, next to index.html.
wasm-bindgen --target web --out-dir dist \
  target/wasm32-unknown-unknown/release/ikigai_web_demo.wasm

# 3. Serve dist/ over HTTP and open the page.
cd dist && python3 -m http.server 8087 --bind 127.0.0.1
```

Then open <http://127.0.0.1:8087>.

The page fills its slots from the kernel on load, and the **Interactive** section
lets you SOURCE `urn:fn:toUpper` / `urn:fn:reverseList` against your own input.

### After editing `src/lib.rs`

Re-run steps 1–2, then refresh the browser — the running server picks up the new
files. If nothing changed, you only need step 3 (the `dist/` artifacts are reused).

### Alternative: `trunk serve`

[`trunk`](https://trunkrs.dev) can do build + bindgen + serve + live-reload in one
command. It expects to drive its own `index.html` at the crate root, whereas this
demo ships a hand-written `dist/index.html` with an explicit ES-module import — so
the three-step recipe above matches what's in the repo.

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
stream, resolved by the remote kernel — so `source urn:fn:toUpper hi` twice shows
`computed` then `cached`, the *server's* cache.

```bash
# build the WASM glue first (steps 1–2 above), then:
cargo run --bin ikigai-net-server     # serves on https://127.0.0.1:4433, prints a cert hash
cd dist && python3 -m http.server 8087
# open http://127.0.0.1:8087/net.html — paste the printed cert hash (or use #cert=<hash>)
```

The cert is self-signed and **rotates each run**; the browser trusts it via
WebTransport's `serverCertificateHashes` (no CA), so paste the current hash.
Needs Chrome/Edge (or recent Firefox). It isn't on GitHub Pages — Pages is
static-only, and this needs a running server — so it's run-it-yourself.

## What's in here

- `src/lib.rs` — the in-browser kernel: binds the demo endpoints (incl. `compose`)
  and the page shapes, and exposes `evalLine` (the CLI Engine over the kernel) plus
  the network demo's wire codec (`encodeComposeCall` / `decodeReply`) to JS via
  `wasm-bindgen`.
- `src/bin/server.rs` — the WebTransport kernel server (native; `wtransport` +
  `ikigai-wire`), reusing the same `build_kernel()` so it composes the same page.
- `dist/index.html` — the in-browser demo (one `compose('urn:data:page')` → body).
- `dist/net.html` — the network demo (pull `urn:data:page` over WebTransport).
  The committed files under `dist/`; the `.js`/`.wasm` are generated and gitignored.

## License

Demo code; same license as the ikigai crates (MIT OR Apache-2.0).
