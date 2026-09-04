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

This directory holds Playwright tests that load the built `dist/`, drive the demo the way a
visitor does, and assert things no compiler can.

**The kernel runs** (`the wasm kernel renders the Control plane…`):

1. **The Control plane renders** — three cards (Scheduler · Cache · Time jobs), each with
   content.
2. **A timed job completes** — the persistent 1s nav-clock job's `runs` count climbs, and a
   greeter timer scheduled through *Demo → Timer* ticks and reports its output.

The second is the sharp one. The panic was in the completion stamp, so a run count that
advances is direct evidence the broken path works.

Console errors are asserted too: none, apart from a capped, pre-existing pair (see the
comment in `demo.spec.js`).

**The tab strip offers only what this kernel can serve** (`the Demo tab strip withdraws
Lisp…`): `ikigai-runbook` ships a hardcoded built-in list, so every host got a **Lisp** tab
whether or not it could serve one. This kernel cannot — it does not link `ikigai-lisp`
(Steel does not compile to wasm) — so every step in that tab answered `no endpoint resolved
for urn:lisp:eval` on the public site. `hide_tab("lisp")` withdraws it. The test asserts
**both** halves: Lisp absent, *and* named tabs still present. Absence alone is weak —
hiding everything would pass it — and `hide_tab` accepts an unknown id silently, so a typo
hides nothing and says nothing.

**The passkey ceremony asks the authenticator** (`the passkey ceremony asks the
authenticator…`): the bridge used to cache the credential id in `localStorage` and branch
on it, treating that cache as authoritative about what the keychain held. Once written,
every later sign-in called `get()` with `allowCredentials` naming that one credential, and
nothing ever removed the key — so a browser without that passkey was pinned forever on a
branch that could not succeed, with registration unreachable (observed in Safari). A
WebAuthn ceremony cannot complete headlessly, so the test stubs `navigator.credentials` and
asserts on the **branch taken and the options asked for**: a fresh profile probes, finds
nothing, and *offers* registration rather than minting a credential unasked; the offered
affordance registers a **discoverable** credential (`residentKey: required`, platform
attachment) and ends signed in; an authenticator that already holds one signs in with it
and mints nothing new; `get()` never carries `allowCredentials`; and no credential id is
cached. Registration is a second, separate click on purpose — WebKit gates a WebAuthn
ceremony on user activation and `get()` consumes it, so chaining `create()` onto the failed
probe is what would be refused in Safari, the browser the bug was reported from. The test
also fails on any browser dialog, since the recovery is in-page by design. It checks the
mechanism only — what the Identity tab *means* (serverless identity selection/derivation,
not authenticated login) is unchanged.

**The transition window is open** (`the old urn:fn: names still resolve…`): `ikigai-fn`
0.2.0 renamed the four names it owns — `urn:fn:*` → `urn:iki:fn:*` — and installs no alias,
because binding authority is a host concern and this repo is the host. `build_kernel`
installs `AliasTable::new().prefix("urn:fn:", "urn:iki:fn:")` in the same release that
adopts the bump; without that pairing the demo would ship with every `urn:fn:` name dead.
The claim being gated is not "it compiles" but "every old name a bookmark, a transcript or
a visitor's muscle memory still holds keeps answering", so the test drives the in-page
terminal and reads the Control panel:

- the **old** name resolves (`source urn:fn:toUpper …`), and so does the new one;
- both share **one cache entry** — the second spelling comes back `cached`, not
  recomputed, and `cache urn:fn:toUpper …` (the read-only probe) agrees. The Cache card
  lists the backing name only;
- the **catalog offers the new name only** (`list`), which is what makes a catalog-driven
  consumer migrate itself while an old-name holder keeps working;
- the rewrite survives **nested** resolution: `urn:data:about` keeps one old-name marker
  on purpose, two levels down inside the composed page, and its text must render.

⚠ That last one couples the page to the rule: `compose` propagates a failed marker, so the
exhibit and the alias rule are deleted in the same edit or the whole page dies.

## It was verified against the actual bug

Before this landed, the gate was replayed against a deliberately broken build — `Cargo.toml`
pinned back to `ikigai-time = "0.1.12"`, `JobRegistry::new` reverted to its one-argument
form, rebuilt for wasm32. That build **compiles without a warning** and the gate fails it,
reproducing the original panic chain verbatim: `time not implemented on this platform` at
`<std::time::Instant>::now`, then `unreachable`, then `cannot recursively acquire mutex`.

Worth knowing which assertion caught it: in that replay all three cards still *rendered*,
frozen at `runs 0`. The card-count check alone would have missed it. The run count and the
console hygiene are what failed.

The alias test was verified the same way, by **ablation**: remove `.with_aliases(...)` from
`build_kernel` and rebuild. The page then fails to compose at all — `compose error: no
endpoint resolved for urn:fn:toUpper`, and all four tests fail on a missing toolbar. Point
the about-box marker at the new name as well, so the page renders again, and the failures
narrow to exactly the claims: `urn:kernel:aliases` reads `(no rewrite table installed)`,
`source urn:fn:toUpper …` answers `no endpoint resolved for urn:fn:toUpper`, and the probe
answers `not cached`. Restore both and it passes. Re-run that when the rule changes: a
transition window nobody has watched close is not a demonstrated window.

The two later tests were replayed the same way, against the pre-fix `src/lib.rs` and
`dist/index.html` rebuilt for wasm32: the strip test fails on `Lisp` (`toHaveCount` 0
expected, 1 received), and the passkey test fails at the recovery affordance — the old
bridge never asked the authenticator anything, it read `localStorage` and registered
straight away, so `#ik-register` does not exist. (The assertions that pin the dead end
itself — no cached credential id, and a returning visitor's `get()` carrying no
`allowCredentials` — fail on that build too, further down.) Neither passes against the bug
it exists for.

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

That workflow runs on **pull requests too**, with `deploy` guarded by
`if: github.event_name != 'pull_request'`. So a PR builds and smoke-tests without
publishing, and the gate blocks the *merge*, not just the deploy. It costs a wasm build and
the two module clones per PR; a gate that first executes after merge would report a broken
demo rather than prevent one, which is precisely the twenty days above.

Running it there, rather than as an in-crate `wasm-bindgen-test`, buys coverage a Rust test
cannot have: `dist/index.html` carries ~400 lines of hand-written glue (the `/k/` fetch
interception, the htmx adapter, the passkey bridge) that no Rust test can reach, and
`pages.yml` builds two module wasms from `ikigai-xslt` and `ikigai-jsonld`, which nothing
in this repo compiles or lints. Those were unpinned upstream `HEAD` until 2026-09-02 — any
upstream commit reached the public site with no commit here — and are now pinned to exact
SHAs in the workflow's `env:` block. Pinning makes a module change deliberate; this gate is
what vets the bump.

**Note on cost:** `build` and `smoke` now run on every PR, duplicating work `ci.yml` does
not do. If that becomes a drag, the tidy-up is to factor `build` + `smoke` into a reusable
workflow both files call — same coverage, one definition. It was left as the simpler shape
until there is a reason to add the indirection.
