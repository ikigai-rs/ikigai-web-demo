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
