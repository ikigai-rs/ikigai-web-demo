// The gate that RUNS the wasm kernel.
//
// Why this exists, concretely: ikigai-time 0.1.12 stamped a completed job with
// `std::time::Instant::now()`. That COMPILES on wasm32-unknown-unknown and panics at
// runtime — and it panicked while holding the job-registry mutex, so every later
// `urn:time:*` read died `cannot recursively acquire mutex` once a second. Because
// `urn:time:jobs` is one of three sub-requests `urn:data:control-cards` composes, the
// Control panel of the public demo rendered a header and nothing else from 2026-08-12
// to 2026-09-01. Every gate in ci.yml passed the whole time: fmt, clippy on the wasm
// lib, clippy on all native targets. All three are compile-only, and no compile-only
// gate can see a panic. `cargo test` passed too — the repo had no tests.
//
// So this asserts the two things a compiler cannot: the page RENDERS, and a timed job
// COMPLETES. The second is the sharp one — the panic was in the completion stamp, so a
// run count that climbs is direct evidence that the exact broken path now works.
const { test, expect } = require('@playwright/test');

// Load-time console noise we tolerate, deliberately and narrowly.
//
// The nav clock is `hx-trigger="load, every 1s"` inside the composed page, and
// index.html calls `htmx.process(app)` — which fires that `load` trigger synchronously —
// a few statements BEFORE it installs the `htmx:beforeRequest` listener that resolves
// `/k/...` through the kernel instead of over the network. Exactly one tick escapes
// through that gap and 404s. There are no awaits between the two, so it is one tick on a
// fast machine and one tick on a slow one; the deployed site shows the same.
//
// That ONE escaped request is logged TWICE, by two different reporters:
//   1. htmx's own error   — "Response Status Error Code 404 from /k/source urn:time:now
//                            html=true", attributed to htmx.org on unpkg;
//   2. the browser's      — "Failed to load resource: ... 404 (Not Found)", attributed to
//                            the /k/... URL itself.
// Hence a ceiling of two lines. This is a pre-existing wart in the page's bootstrap
// ordering and not this gate's business to fix — but the COUNT is its business: if it
// grows, the fetch interception has stopped intercepting, which is a real regression.
const KNOWN_404 = [
  /Response Status Error Code 404 from \/k\/source urn:time:now html=true/,
  /Failed to load resource.*404/,
];
const KNOWN_404_URL = /\/k\/source(%20| )urn:time:now(%20| )html=true/;
const KNOWN_404_MAX = 2;

/** Pull `runs <n>` off the `urn:time:jobs` line whose target matches. */
function runsFor(readout, target) {
  const line = (readout || '').split('\n').find((l) => l.includes(target));
  if (!line) return null;
  const m = line.match(/runs (\d+)/);
  return m ? Number(m[1]) : null;
}

test('the wasm kernel renders the Control plane and its timed jobs complete', async ({ page }) => {
  /** @type {string[]} */ const known404s = [];
  /** @type {string[]} */ const unexpected = [];

  page.on('console', (msg) => {
    if (msg.type() !== 'error') return;
    const url = msg.location()?.url || '';
    const line = `${msg.text()} :: ${url}`;
    // Allowlisted only when the message matches AND it is about the nav-clock request —
    // so a 404 on anything else (a missing lazy module wasm, say) still fails the gate.
    const known =
      KNOWN_404.some((re) => re.test(msg.text())) &&
      (KNOWN_404_URL.test(url) || KNOWN_404_URL.test(msg.text()));
    (known ? known404s : unexpected).push(line);
  });
  // A Rust panic reaches the page as an uncaught exception (console_error_panic_hook
  // logs it, then the wasm traps). Both channels are collected; neither may fire.
  page.on('pageerror', (err) => unexpected.push(`pageerror: ${err.message}`));

  await page.goto('/index.html');

  // --- The page is one composed resource -----------------------------------
  // The toolbar does not exist in index.html; it is authored by `urn:data:page` and
  // injected after the wasm boots. Seeing it at all means the kernel resolved.
  const toolbar = page.locator('nav.ik-toolbar');
  await expect(toolbar).toBeVisible();

  // --- Control: three live cards, each with content ------------------------
  await toolbar.getByRole('button', { name: 'Control', exact: true }).click();

  const cards = page.locator('#ctl-cards .ctl-card');
  await expect(cards).toHaveCount(3);
  await expect(page.locator('#ctl-cards .ctl-title')).toHaveText([
    'Scheduler',
    'Cache',
    'Time jobs',
  ]);

  // On the public site the outage looked like this from here: `#ctl-cards` stayed empty,
  // because one failing sub-request took the whole compose down.
  //
  // NECESSARY BUT NOT SUFFICIENT, and it is worth knowing which. Replaying the bug locally
  // (`ikigai-time = "0.1.12"`, rebuilt, run against this gate) reproduces the exact panic
  // chain — `time not implemented on this platform` at `<std::time::Instant>::now`, then
  // `unreachable`, then `cannot recursively acquire mutex` — and yet all three cards still
  // RENDER, with the job frozen at `runs 0`. Whether the compose dies outright or merely
  // stops advancing is a timing detail. So this check alone would have missed it; the run
  // count below, and the console hygiene at the end, are what actually catch it. Both were
  // confirmed to fail against that rebuilt-broken artifact before this gate was committed.
  const readouts = page.locator('#ctl-cards pre.ctl-readout');
  for (let i = 0; i < 3; i++) {
    await expect(readouts.nth(i)).not.toBeEmpty();
  }

  // --- A timed job COMPLETES ------------------------------------------------
  // The nav clock is registered once, at engine init, as a persistent 1s job sourcing
  // the cacheable `urn:time:now`. It needs no interaction: within a second of load the
  // kernel is already running the code path that used to panic.
  const timeJobs = readouts.nth(2);
  await expect(timeJobs).toContainText('urn:time:now');
  await expect(timeJobs).toContainText('(persistent)');

  const clockRuns = await timeJobs.textContent().then((t) => runsFor(t, 'urn:time:now'));
  expect(clockRuns, 'the nav-clock job should be listed with a run count').not.toBeNull();
  await expect
    .poll(async () => runsFor(await timeJobs.textContent(), 'urn:time:now'), {
      message: 'the persistent nav-clock job should keep completing fires',
      timeout: 20_000,
    })
    .toBeGreaterThan(clockRuns);

  // --- Demo -> Timer: schedule a job through the runbook --------------------
  // The load-time job proves completion; this proves SCHEDULING still works end to end,
  // through the htmx adapter and `urn:time:schedule`, the way a visitor drives it.
  await page.locator('nav.ik-toolbar').getByRole('button', { name: 'Demo', exact: true }).click();
  // The runbook tab strip is a real `tablist` — the tabs are `role=tab`, not buttons.
  await page.getByRole('tab', { name: 'Timer', exact: true }).click();
  await page.getByRole('button', { name: 'start a 1-second greeter timer' }).click();
  await expect(page.locator('#rb-out')).toContainText('scheduled job');

  await page.locator('nav.ik-toolbar').getByRole('button', { name: 'Control', exact: true }).click();
  const timeJobs2 = page.locator('#ctl-cards pre.ctl-readout').nth(2);
  await expect(timeJobs2).toContainText('urn:demo:greeter');

  const greeterRuns = await timeJobs2.textContent().then((t) => runsFor(t, 'urn:demo:greeter'));
  expect(greeterRuns, 'the greeter job should be listed with a run count').not.toBeNull();
  await expect
    .poll(async () => runsFor(await timeJobs2.textContent(), 'urn:demo:greeter'), {
      message: 'the scheduled greeter job should tick',
      timeout: 20_000,
    })
    .toBeGreaterThan(greeterRuns);

  // The greeter's output is echoed back into the card, so a job that "runs" but resolves
  // to nothing does not pass.
  await expect(timeJobs2).toContainText('last: Hello from the ikigai kernel');

  // --- Console hygiene ------------------------------------------------------
  expect(unexpected, `unexpected console errors:\n${unexpected.join('\n')}`).toEqual([]);
  expect(
    known404s.length,
    `the pre-existing nav-clock 404s should stay capped at ${KNOWN_404_MAX} ` +
      `(more means the /k/ fetch interception stopped intercepting):\n${known404s.join('\n')}`,
  ).toBeLessThanOrEqual(KNOWN_404_MAX);
});

// ---------------------------------------------------------------------------
// The runbook tab strip: an offer this kernel can actually honour.
//
// `ikigai-runbook` ships a hardcoded list of built-in demos, so every host got a **Lisp**
// tab whether or not it could serve one. This kernel cannot: it does not link
// `ikigai-lisp` (Steel does not compile to wasm) and never has, so every step in that tab
// answered `no endpoint resolved for urn:lisp:eval` on the public demo. `hide_tab("lisp")`
// (runbook 0.1.13) withdraws it.
//
// Absence alone is a weak assertion — hiding *every* tab would also pass it, and
// `hide_tab` accepts an unknown id silently, so a typo hides nothing and says nothing
// either. Hence both halves: Lisp gone, and named tabs that must survive still there.
test('the Demo tab strip withdraws Lisp and keeps the tabs this kernel serves', async ({ page }) => {
  /** @type {string[]} */ const unexpected = [];
  page.on('pageerror', (err) => unexpected.push(`pageerror: ${err.message}`));

  await page.goto('/index.html');
  const toolbar = page.locator('nav.ik-toolbar');
  await expect(toolbar).toBeVisible();
  await toolbar.getByRole('button', { name: 'Demo', exact: true }).click();

  // The strip is a real tablist rendered by the kernel, so waiting on a known tab waits
  // on the whole strip having been swapped in.
  const strip = page.locator('#runbook nav.rb-tabs');
  await expect(strip.getByRole('tab', { name: 'Basics', exact: true })).toBeVisible();

  // Gone.
  await expect(strip.getByRole('tab', { name: 'Lisp', exact: true })).toHaveCount(0);

  // Still here: two built-ins, plus both tabs this host adds itself — `hide_tab` and
  // `add_tab` write to the same strip, so a mistake in one is visible in the other.
  for (const name of ['Basics', 'Piping', 'SHACL', 'Identity', 'Timer']) {
    await expect(strip.getByRole('tab', { name, exact: true })).toBeVisible();
  }

  // A blunt floor under "hid everything": the strip is still the full walkthrough set.
  expect(await strip.getByRole('tab').count()).toBeGreaterThanOrEqual(12);

  expect(unexpected, `unexpected page errors:\n${unexpected.join('\n')}`).toEqual([]);
});

// ---------------------------------------------------------------------------
// The passkey ceremony: the authenticator is the source of truth, not localStorage.
//
// The old bridge cached the credential id in `localStorage` under `ikigai:passkey:credId`
// and branched on it. Once written, every later sign-in called `get()` with
// `allowCredentials` naming that one credential — and nothing ever removed the key. If
// the passkey was not in THIS authenticator (never synced, deleted, made on another
// device or profile) Safari found no local match and offered only QR-code-or-hardware-key.
// The browser was pinned forever on a branch that could not succeed, with the
// registration path unreachable. Brian hit exactly this in Safari.
//
// A WebAuthn ceremony cannot complete headlessly, so this asserts on which branch the page
// takes and what it asks for: stub `navigator.credentials` and read the options back.
//
// SCOPE, deliberately: this checks the MECHANISM. What the Identity tab means — serverless
// identity *selection/derivation*, not authenticated login — is unchanged and out of scope.
test('the passkey ceremony asks the authenticator, and can still register', async ({ page }) => {
  /** @type {string[]} */ const unexpected = [];
  page.on('pageerror', (err) => unexpected.push(`pageerror: ${err.message}`));
  // Nothing here should ever open a browser dialog: the recovery is an in-page affordance,
  // precisely so registration starts from its own click. Playwright dismisses dialogs by
  // default, so a regression to `confirm()` would fail below rather than hang — but say so.
  page.on('dialog', (d) => {
    unexpected.push(`unexpected dialog: ${d.message()}`);
    return d.dismiss();
  });

  // Installed before any page script runs, so the bridge only ever sees the stub.
  await page.addInitScript(() => {
    /** @type {{op: string, opts: any}[]} */ const calls = [];
    // `empty` = a fresh profile whose authenticator holds nothing for this origin;
    // `holds` = a returning visitor's, with a discoverable credential in it.
    window.__pkCalls = calls;
    window.__pkMode = 'empty';
    const FIXED_RAW_ID = new Uint8Array(32).fill(7).buffer;
    // Buffers don't survive the page->test boundary; their presence is all we assert.
    const plain = (pk) =>
      JSON.parse(
        JSON.stringify(pk, (_k, v) =>
          v instanceof ArrayBuffer || ArrayBuffer.isView(v) ? '<bytes>' : v,
        ),
      );
    navigator.credentials.get = async (o) => {
      calls.push({ op: 'get', opts: plain(o.publicKey) });
      if (window.__pkMode === 'empty') {
        const err = new Error('the authenticator holds no credential for this origin');
        err.name = 'NotAllowedError';
        throw err;
      }
      return { rawId: FIXED_RAW_ID };
    };
    navigator.credentials.create = async (o) => {
      calls.push({ op: 'create', opts: plain(o.publicKey) });
      return { rawId: FIXED_RAW_ID };
    };
  });

  await page.goto('/index.html');
  const toolbar = page.locator('nav.ik-toolbar');
  await expect(toolbar).toBeVisible();
  await toolbar.getByRole('button', { name: 'Demo', exact: true }).click();
  await page.getByRole('tab', { name: 'Identity', exact: true }).click();

  // --- Fresh profile: probe, find nothing, OFFER registration ---------------
  await page.locator('#ik-login').click();

  // No passkey exists, so the page must not be signed in and must not have registered
  // one behind the visitor's back — it offers.
  await expect(page.locator('#ik-register')).toBeVisible();
  await expect(page.locator('#ik-logout')).toHaveCount(0);
  expect(
    await page.evaluate(() => window.__pkCalls.map((c) => c.op)),
    'the probe alone must not mint a credential',
  ).toEqual(['get']);

  // WebKit gates a WebAuthn ceremony on user activation and `get()` consumes it, so
  // registration has to start from its OWN click — chaining it onto the failed probe is
  // what would refuse in Safari, the browser this bug was reported from.
  await page.locator('#ik-register').click();

  // Signed in: the affordance flips and the segment walkthrough appears, so the ceremony
  // ran all the way through `sink urn:host:login` and not merely up to the stub.
  await expect(page.locator('#ik-logout')).toBeVisible();
  await expect(page.locator('.rb-steps .rb-step')).toHaveCount(3);

  const fresh = await page.evaluate(() => window.__pkCalls);
  expect(
    fresh.map((c) => c.op),
    'a fresh profile probes the authenticator, finds nothing, then registers on request',
  ).toEqual(['get', 'create']);

  // THE REGRESSION. `allowCredentials` is what named a credential the authenticator might
  // not hold; without it the credential answers for itself and the keychain decides.
  expect(fresh[0].opts.allowCredentials, 'get() must not name a credential').toBeUndefined();

  // Registration must mint a DISCOVERABLE credential — otherwise the allowCredentials-free
  // get() above has nothing to find, and the next visit dead-ends again.
  expect(fresh[1].opts.authenticatorSelection).toMatchObject({
    residentKey: 'required',
    requireResidentKey: true,
    authenticatorAttachment: 'platform',
  });

  // No credential identity is cached anywhere: that cache WAS the bug.
  expect(
    await page.evaluate(() => localStorage.getItem('ikigai:passkey:credId')),
    'the page must keep no record of which credential the authenticator holds',
  ).toBeNull();

  // --- Returning visitor: assert, and do NOT mint a second passkey ----------
  await page.locator('#ik-logout').click();
  await expect(page.locator('#ik-login')).toBeVisible();
  await page.evaluate(() => {
    window.__pkCalls.length = 0;
    window.__pkMode = 'holds';
  });

  await page.locator('#ik-login').click();
  await expect(page.locator('#ik-logout')).toBeVisible();

  const returning = await page.evaluate(() => window.__pkCalls);
  expect(
    returning.map((c) => c.op),
    'an authenticator that already holds a passkey signs in with it, and registers nothing',
  ).toEqual(['get']);
  expect(returning[0].opts.allowCredentials).toBeUndefined();

  expect(unexpected, `unexpected page errors:\n${unexpected.join('\n')}`).toEqual([]);
});

// ---------------------------------------------------------------------------
// The transition window: `urn:fn:*` → `urn:iki:fn:*`.
//
// `ikigai-fn` 0.2.0 renamed the four names it owns and installs no alias — binding
// authority is a host concern, and this repo is the host. So this host bumped the library
// and installed `AliasTable::new().prefix("urn:fn:", "urn:iki:fn:")` in the same release
// (`build_kernel`), and the claim it is making is not "the code compiles" but "every old
// name a visitor, a bookmark or a pasted transcript still holds keeps answering."
//
// That claim is only worth the running of it. Each assertion below is one of ikigai-core's
// alias decisions, checked against the built wasm in a browser rather than trusted:
//
//   decision 3 — the two names share ONE cache entry. Proved on the SERVING path (the
//                second spelling comes back `cached`, not recomputed), on the read-only
//                probe, and on the readout (every entry the page and this test created
//                under BOTH spellings is listed under the backing name, and none under the
//                logical one). The other half of decision 3 — one golden thread, so a
//                `Sink` through either name cuts the other — is NOT exercised here: it
//                follows from the same identity point (the id is derived after the rewrite
//                is adopted), but every `urn:iki:fn:*` endpoint is Source-only, so this
//                host has no sinkable resource in that namespace to cut.
//   decision 4 — the catalog advertises the NEW name only, which is what makes a
//                catalog-driven consumer migrate itself while an old-name holder keeps
//                working. `list` must show one and not the other.
//   nested     — the home page is a compose shape, and `urn:data:about` (a shape inside
//                that shape) keeps one old-name marker on purpose. It is two levels down,
//                and `compose` propagates a failed marker, so if the rewrite ever stopped
//                happening the page would not render at all.
//
// ABLATION (run before this was committed, and the way to re-verify it): delete the
// `.with_aliases(...)` line in `build_kernel`, rebuild, and every old-name assertion here
// fails — the terminal answers `no endpoint resolved for urn:fn:toUpper`, the about-box
// text never appears, and the page itself renders `compose error:` instead of a toolbar.
// Restore the line and it passes again. A window nobody has watched close is not a
// demonstrated window.

/** Run one line in the in-page terminal and return its output, isolated.
 *
 * `clear` empties the transcript (keeping history), so what is read back afterwards is
 * this line's output and nothing else — the alternative, matching against the whole
 * scrollback, would let an earlier command's text satisfy a later assertion. */
async function runLine(page, line) {
  const cli = page.locator('ikigai-cli');
  const input = cli.locator('.cli-input');
  await input.fill('clear');
  await input.press('Enter');
  await expect(cli.locator('.cli-line')).toHaveCount(0);
  await input.fill(line);
  await input.press('Enter');
  // The echo lands synchronously; the result follows when the kernel answers.
  await expect.poll(async () => cli.locator('.cli-line').count()).toBeGreaterThan(1);
  return (await cli.locator('.cli-line:not(.cli-echo)').allTextContents()).join('\n');
}

/** How many cache entries the Cache card lists for a given IRI (one line each). */
function cacheEntriesFor(readout, iri) {
  return (readout || '').split('\n').filter((l) => l.includes(iri)).length;
}

test('the old urn:fn: names still resolve, through the host alias table', async ({ page }) => {
  /** @type {string[]} */ const unexpected = [];
  page.on('pageerror', (err) => unexpected.push(`pageerror: ${err.message}`));

  await page.goto('/index.html');
  const toolbar = page.locator('nav.ik-toolbar');
  await expect(toolbar).toBeVisible();

  // --- Nested: an old name two levels down, resolved during the page compose ---
  // `urn:data:page` transcludes `urn:data:about`, which transcludes `$a{urn:fn:toUpper}`.
  // Seeing the uppercased text means the rewrite happened inside a nested resolution, on
  // the ordinary render path, with nothing in this test driving it.
  await expect(page.locator('aside.about')).toContainText('THE OLD NAME STILL RESOLVES');
  // The new name, in the same box — both spellings reach the same endpoint.
  await expect(page.locator('aside.about')).toContainText('EVEN THIS NESTED SHAPE WAS COMPOSED');

  // --- The table is installed, and says so ---------------------------------
  const aliases = await runLine(page, 'source urn:kernel:aliases');
  expect(aliases).toContain('prefix');
  expect(aliases).toMatch(/urn:fn:\s+->\s+urn:iki:fn:/);
  // The page's own compose already put hops on the counter — the exhibit above.
  expect(aliases).toMatch(/[1-9]\d* hops/);

  // --- Both names resolve, and share ONE entry -----------------------------
  // A value nothing else in the demo uses, so the entry it creates is unambiguous.
  const probe = 'alias-window-proof';
  const old = await runLine(page, `source urn:fn:toUpper ${probe}`);
  expect(old).toContain(probe.toUpperCase());
  expect(old).toContain('computed');

  // The SAME argument under the NEW name. `cached` here is the whole of decision 3: the
  // kernel adopted the backing name before it computed the request id, so this is not a
  // second endpoint run that happens to agree — it is the first one's entry.
  const renamed = await runLine(page, `source urn:iki:fn:toUpper ${probe}`);
  expect(renamed).toContain(probe.toUpperCase());
  expect(renamed).toContain('cached');
  expect(renamed).not.toContain('computed');

  // And the read-only probe canonicalizes the same way: `cache <old name>` must not
  // answer "not cached" about an entry the new name is being served from.
  expect(await runLine(page, `cache urn:fn:toUpper ${probe}`)).toContain('cached');

  // --- The catalog advertises the NEW name only ----------------------------
  const list = await runLine(page, 'list');
  expect(list).toContain('urn:iki:fn:toUpper');
  expect(list).toContain('urn:iki:fn:compose');
  // `urn:iki:fn:` does not contain `urn:fn:`, so this is a real exclusion, not a
  // substring that the new name happens to satisfy.
  expect(list, 'the catalog must not offer the pre-migration names').not.toContain('urn:fn:');

  // --- A LINKED MODULE's baked-in old names still run -----------------------
  // `ikigai-runbook` 0.1.13 hardcodes `source urn:fn:toUpper hello` as a Basics step, and
  // this host cannot edit it — it is a published crate. That step working is the case the
  // window actually exists for: content the host does not control, still holding the old
  // name, resolving anyway because the alias lives in the HOST rather than the library.
  await toolbar.getByRole('button', { name: 'Demo', exact: true }).click();
  await page.getByRole('tab', { name: 'Basics', exact: true }).click();
  await page.getByRole('button', { name: 'uppercase' }).click();
  await expect(page.locator('#rb-out')).toContainText('HELLO');

  // --- The Cache card: one entry for two names -----------------------------
  await toolbar.getByRole('button', { name: 'Control', exact: true }).click();
  const cacheCard = page.locator('#ctl-cards pre.ctl-readout').nth(1);
  await expect(cacheCard).not.toBeEmpty();
  const readout = await cacheCard.textContent();
  // The page composed toUpper twice (once under each spelling) and this test added one
  // more argument under both spellings — every one of them keyed to the backing name.
  expect(cacheEntriesFor(readout, 'urn:iki:fn:toUpper')).toBeGreaterThanOrEqual(3);
  expect(
    readout,
    'a logical name with its own cache entry is the stale-representation bug decision 3 exists to prevent',
  ).not.toContain('urn:fn:toUpper');

  expect(unexpected, `unexpected page errors:\n${unexpected.join('\n')}`).toEqual([]);
});
