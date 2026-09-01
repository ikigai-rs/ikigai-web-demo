# smoke — the gate that runs the wasm

`ci.yml` checks that this repo *compiles*: `cargo fmt`, `cargo clippy --lib --target
wasm32-unknown-unknown`, `cargo clippy --all-targets`. Nothing there executes a single
instruction of the artifact that ships, and `cargo test` passes vacuously — the crate has
no tests.

That gap has a cost on the record. `ikigai-time 0.1.12` stamped completed jobs with
`std::time::Instant::now()`. It compiles clean for `wasm32-unknown-unknown` and panics at
runtime — and it panicked while holding the job-registry mutex, so every later
`urn:time:*` read died `cannot recursively acquire mutex` once a second. Since
`urn:time:jobs` is one of three sub-requests `urn:data:control-cards` composes, the
Control panel of <https://ikigai-rs.github.io/ikigai-web-demo/> showed a header and
nothing else from **2026-08-12 to 2026-09-01**, with every check green throughout.

This directory holds one Playwright test that loads the built `dist/`, drives the demo the
way a visitor does, and asserts two things no compiler can:

1. **The Control plane renders** — three cards (Scheduler · Cache · Time jobs), each with
   content.
2. **A timed job completes** — the persistent 1s nav-clock job's `runs` count climbs, and a
   greeter timer scheduled through *Demo → Timer* ticks and reports its output.

The second is the sharp one. The panic was in the completion stamp, so a run count that
advances is direct evidence the broken path works.

Console errors are asserted too: none, apart from a capped, pre-existing pair (see the
comment in `demo.spec.js`).

## It was verified against the actual bug

Before this landed, the gate was replayed against a deliberately broken build — `Cargo.toml`
pinned back to `ikigai-time = "0.1.12"`, `JobRegistry::new` reverted to its one-argument
form, rebuilt for wasm32. That build **compiles without a warning** and the gate fails it,
reproducing the original panic chain verbatim: `time not implemented on this platform` at
`<std::time::Instant>::now`, then `unreachable`, then `cannot recursively acquire mutex`.

Worth knowing which assertion caught it: in that replay all three cards still *rendered*,
frozen at `runs 0`. The card-count check alone would have missed it. The run count and the
console hygiene are what failed.

## Running it locally

You need a built `dist/` — the wasm and its JS bindings are gitignored because CI generates
them:

```bash
cargo build --release --lib --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir dist target/wasm32-unknown-unknown/release/ikigai_web_demo.wasm
```

Then, from this directory:

```bash
npm ci && npx playwright install chromium && npx playwright test
```

It takes about five seconds. `npx playwright test --headed` watches it happen, and
`npx playwright show-trace test-results/*/trace.zip` opens the trace after a failure.

## Where it runs

In `pages.yml`, as a `smoke` job between `build` and `deploy`. `build` hands over the exact
stamped `dist/` it is about to publish, and `deploy` is `needs: [build, smoke]` — so a tree
that compiles but does not run never reaches the public URL.

Running it there, rather than as an in-crate `wasm-bindgen-test`, buys coverage a Rust test
cannot have: `dist/index.html` carries ~400 lines of hand-written glue (the `/k/` fetch
interception, the htmx adapter, the passkey bridge) that no Rust test can reach, and
`pages.yml` builds two module wasms from `git clone --depth 1` of `ikigai-xslt` and
`ikigai-jsonld` — unpinned upstream `HEAD` that nothing in this repo compiles or lints.

**Known gap:** this gate runs on push to `main`, so it blocks the *deploy*, not the *merge*.
Closing that means factoring `build` + `smoke` into a reusable workflow that `ci.yml` also
calls on `pull_request`. That is a deliberate follow-up, not an oversight.
