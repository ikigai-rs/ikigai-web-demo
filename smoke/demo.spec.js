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
