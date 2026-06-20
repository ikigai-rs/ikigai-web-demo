# ikigai demo runbook

**One line:** a content-addressed, capability-secured resolution kernel —
*ZeroTrust · Flexible · Dynamic · Cacheable* — that runs identically in a
terminal, across processes, over the network, and in a browser tab.

Two repos: this one (`ikigai-web-demo`) for the browser demos, and a sibling
`ikigai-cli` checkout for the terminal.

---

## 0. The live web demo — zero setup

**<https://ikigai-rs.github.io/ikigai-web-demo/>**

- The whole page is **one resource**: the browser made a single
  `compose(urn:data:page)` call; the in-browser WASM kernel recursively expanded
  every `$a{…}` transclusion marker. No fetch, no server.
- Scroll to **the embedded terminal** — it's the *real* CLI engine compiled to
  WASM, driving the page's own kernel. Try:
  - `source urn:fn:compose src=urn:data:page` → returns **the very page you're
    looking at**, tagged `cached` (it shares the page's cache).
  - `source urn:demo:split "a,b,c" .. urn:fn:toUpper` → `A` `B` `C` (the `..` map operator)
  - `source urn:demo:split "x,y,z" | ( urn:fn:toUpper ; urn:fn:reverseList )` → fork/join
  - `source urn:host:info` → **Embedded (Browser)** · `browser · wasm32` (the host names itself)
  - `list`, `help`

**Files in the tab — `localStorage`-backed, shared with JavaScript.** The page mounts
the `ikigai-fs` module at `urn:file:{path}`, jailed to a virtual `ws` root, on its
wasm `localStorage` backend — the same `file:` contract as the native CLI, only the
storage differs:

- `sink urn:file:note.txt remember the milk` → `wrote 18 bytes`
- `source urn:file:note.txt` → reads it back (`as=application/octet-stream` for raw bytes)
- **Reload the tab**, then `source urn:file:note.txt` again → still there. It lives in
  `localStorage` under `ikigai:fs:ws/note.txt`, so the page's own JavaScript shares
  it: in the devtools console `localStorage.getItem('ikigai:fs:ws/note.txt')` returns
  what `sink` wrote — and a JS `setItem` is read straight back by `source`.

**The kernel as resources — `urn:kernel:*`.** The kernel exposes its *own* operations
as capability-gated resources, resolved intrinsically (before any space):

- `source urn:kernel:cache` → the cache entry count
- `source urn:kernel:threads` → the golden threads that have been cut, and how often
- `sink urn:kernel:cut urn:file:note.txt` → cut a thread **by resolving a resource**
  (no special API — so it works over the wire, gated by capability). A `sink` to a
  file already auto-cuts its thread, so `urn:file:note.txt` shows up in
  `urn:kernel:threads` after you write it.

## 1. The terminal CLI — from the `ikigai-cli` checkout

```bash
cargo run --bin ikigai          # full-screen TUI REPL
```

- **Resolve:** `source urn:fn:toUpper hello` → `HELLO`
- **Pipelines / map / fork:** `source urn:fn:toUpper hi | urn:demo:wrap`,
  `source urn:demo:split "a,b,c" .. urn:fn:toUpper`, `( a ; b )`
- **Named args (contract-driven):** `source urn:demo:greet greeting=Hi name=World`
- **compose:** `source urn:fn:compose src=urn:data:page`
- **Cache visibility:** every result tags `computed` / `cached` / `uncacheable`;
  `cache <iri>` probes without resolving.
- **Host info:** `source urn:host:info` → **Embedded (Native)** + runtime — the host
  names itself (and it's `uncacheable`, a live fact).
- **Editable input line:** Emacs / vi / native keybindings, kill-ring, system
  clipboard, OSC-52 over SSH. Switch with `config keybindings=vi`.
- `list`, `describe urn:fn:toUpper`, `help`, `quit`.

**Files — capability-gated, jailed, cacheable.** The CLI mounts `ikigai-fs` at
`urn:file:{path}`, jailed to `~/.ikigai/workspace` (override with `$IKIGAI_FILES`):

- `sink urn:file:notes.txt remember the milk` → `wrote 18 bytes`; `source urn:file:notes.txt` → reads it back.
- **Capabilities:** `cap read-only` drops the session to reads — `sink …` then errors
  *capability does not grant `write`*. (`read` / `write` / `delete` / `agent` are the
  other profiles.) The **jail** is the hard floor under the capability: even at root,
  `source urn:file:../../etc/hosts` → *parent-directory segments are not allowed*.

**The golden thread — write a thing, watch the cache invalidate.** File reads cache
under a golden thread named after the resource; a write cuts it:

- `source urn:file:notes.txt` → `[computed]`, again → `[cached]` (file reads cache now);
  then `sink urn:file:notes.txt v2` ; `source urn:file:notes.txt` → `v2 [computed]` — the write invalidated it.
- **Through composition:** `sink urn:file:page.txt 'latest: $a{urn:file:note.txt}'` ;
  `sink urn:file:note.txt v1` ; `source urn:fn:compose src=urn:file:page.txt` (→ `[cached]`
  on a repeat) ; then `sink urn:file:note.txt v2` ; recompose → **recomputed, with v2** —
  writing the *leaf* file invalidated the *composed page* that transcluded it, no special handling.
- **External edits too:** a filesystem watcher runs behind the workspace — edit a file
  in your editor (no ikigai command) and the next `source` recomputes. The cache tracks
  the world, not just kernel-mediated writes.

**The kernel as resources — `urn:kernel:*`.** `sink urn:kernel:cut <thread>` cuts a
thread by resolving a resource; `source urn:kernel:threads` / `urn:kernel:cache`
introspect live kernel state — the reflection surface, capability-gated.

**trace shows the path *and* the authority.** `trace urn:data:page` draws the
recursive resolution tree (client · transport · each node's endpoint / cache / bytes).
Under a narrowed session it marks each node: `cap freebusy` then
`trace urn:personal:contacts` → `cap ✗ denied`; an authorized node shows `cap ✓`.

**Batch caching (one-shot `-c`).** Several `-c` commands run in order over one
kernel, so overlap is served from cache — and a summary prints at the end:

```bash
ikigai -c 'source urn:fn:toUpper hi' \
       -c 'source urn:fn:toUpper hi' \
       -c 'source urn:host:info'
# stdout:  HI / HI / (host info)
# stderr:  [computed] / [cached] / [uncacheable]
#          — batch: 3 commands · 1 cached · 1 computed · 1 uncacheable
```

## 2. Across processes & the network — same REPL, pluggable transports

```bash
# IPC (Unix socket, peer-credential checked) — two terminals:
cargo run --bin ikigai -- serve            # A: kernel server
cargo run --bin ikigai -- --connect        # B: attaches — shares A's cache

# QUIC (network, mutually-pinned TLS — needs the feature):
cargo run --features quic --bin ikigai -- cert generate
cargo run --features quic --bin ikigai -- serve quic://127.0.0.1:4433       # A
cargo run --features quic --bin ikigai -- --connect quic://127.0.0.1:4433   # B
```

Point out: two clients on one server **see each other's `cached` results** — the
cache lives on the server.

`source urn:host:info` while connected reports the transport — **Remote (IPC)** or
**Remote (QUIC)** — so the *same* command names how you reached the kernel. And batch
caching shines remotely: a one-shot `-c` batch against a warm server is served from
cache, not recomputed:

```bash
ikigai serve &                                       # server — cache starts empty
ikigai --connect -c 'source urn:fn:toUpper hi'       # warm the SERVER's cache (--connect!)
ikigai --connect -c 'source urn:fn:toUpper hi' \
       -c 'source urn:fn:toUpper hi' \
       -c 'source urn:host:info'
#  → — batch: 3 commands · 2 cached · 1 uncacheable   (Remote (IPC))
```

(Note the warming step **also** needs `--connect` — without it, `ikigai -c …` runs a
throwaway *embedded* kernel and warms its own cache, not the server's.)

## 3. The network web demo — pull over WebTransport (this repo)

```bash
# build the WASM glue once:
cargo build --release --lib --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir dist \
  target/wasm32-unknown-unknown/release/ikigai_web_demo.wasm

# run the kernel server (prints a cert hash) + serve the page:
cargo run --bin ikigai-net-server          # → https://127.0.0.1:4433 + cert sha-256
cd dist && python3 -m http.server 8087
```

Open `http://127.0.0.1:8087/net.html`, paste the printed cert hash, **Connect**.
The browser sends a `compose` request as `ikigai-wire` bytes; the **server**
composes the page and streams the HTML back — first pull `computed`, second
`cached` (the server's cache persists). Same page as demo 0, but the kernel is
*remote*. (Chrome/Edge; the cert hash rotates each server run.)

**The terminal in that page is live too** — each command is its own `ikigai-wire`
Call on a fresh WebTransport stream, resolved by the remote kernel:
- `list` — the resources bound *on the server*
- `source urn:fn:toUpper hello` twice → `computed` then `cached` (the server's cache)
- `cache urn:fn:toUpper hello` — is it cached, server-side? (no resolve)
- `compose urn:data:page` — recompose the page over the wire
- `source urn:host:info` → **Remote (WebTransport)** · `native · …` (the *server's* runtime)

So it's literally **demo 0's terminal, but every command goes over the network.**

---

## The "money moments"

- **Same kernel, four runtimes** — terminal → Unix socket → QUIC → browser tab →
  WebTransport, identical behavior.
- **The page composes itself**, in-browser *or* over the wire, recursively, from
  addressable resources.
- **Caching is structural** — `cached` shows up across processes, across the
  network, and across a page reload.
- **The golden thread** — caches stay *correct*: write a file (or edit it in your
  editor) and the cached read — **and any composed page that transcluded it** —
  recompute, because each cache entry tracks the resources it depends on. Edit one
  thing, the whole composed thing updates.
- **Files are just resources** — `urn:file:*` is `std::fs` in the terminal and
  `localStorage` in the browser, same `file:` contract, capability-gated and jailed;
  a static asset and a generated result are interchangeable behind one address.
- **The kernel is resources too** — `urn:kernel:cut` / `urn:kernel:threads` /
  `urn:kernel:cache`: the kernel's own operations are capability-gated resources you
  `source`/`sink` like any other — it introspects and controls itself through the
  same uniform interface.
- **One wire protocol** (`ikigai-wire`) is reused byte-for-byte by IPC, QUIC, and
  WebTransport — the resolution seam is universal.
- **`source urn:host:info` names the situation** — the *same* command reports
  `Embedded (Native)` / `Embedded (Browser)` / `Remote (IPC)` / `Remote (QUIC)` /
  `Remote (WebTransport)` and the runtime, so each demo says what it is.
