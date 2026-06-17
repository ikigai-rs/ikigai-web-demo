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
ikigai serve &                                   # server with a warm cache
ikigai -c 'source urn:fn:toUpper hi'             # … warm it
ikigai --connect -c 'source urn:fn:toUpper hi' \
       -c 'source urn:fn:toUpper hi' \
       -c 'source urn:host:info'
#  → — batch: 3 commands · 2 cached · 0 computed · 1 uncacheable   (Remote (IPC))
```

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
- **One wire protocol** (`ikigai-wire`) is reused byte-for-byte by IPC, QUIC, and
  WebTransport — the resolution seam is universal.
- **`source urn:host:info` names the situation** — the *same* command reports
  `Embedded (Native)` / `Embedded (Browser)` / `Remote (IPC)` / `Remote (QUIC)` /
  `Remote (WebTransport)` and the runtime, so each demo says what it is.
