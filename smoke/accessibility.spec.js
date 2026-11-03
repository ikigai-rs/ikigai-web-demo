const { test, expect } = require('@playwright/test');
const AxeBuilder = require('@axe-core/playwright').default;

async function audit(page) {
  const result = await new AxeBuilder({ page }).analyze();
  expect(result.violations.map(({ id, nodes }) => ({
    id, nodes: nodes.map(({ target, failureSummary }) => ({ target, failureSummary })),
  }))).toEqual([]);
}

test('composed views and every runbook tab pass axe and reflow at 320px', async ({ page }) => {
  await page.goto('/index.html');
  await expect(page.locator('.ik-toolbar')).toBeVisible();
  for (const width of [1280, 320]) {
    await page.setViewportSize({ width, height: 900 });
    for (const view of ['Home', 'Catalog', 'Control', 'Demo']) {
      await page.locator('.ik-toolbar').getByRole('button', { name: view, exact: true }).click();
      if (view === 'Catalog') await expect(page.locator('.card').first()).toBeVisible();
      if (view === 'Control') await expect(page.locator('.ctl-readout')).toHaveCount(3);
      if (view === 'Demo') await expect(page.locator('.rb-tab').first()).toBeVisible();
      await audit(page);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      if (view === 'Demo') {
        const names = await page.locator('.rb-tab').allTextContents();
        for (const name of names) {
          await page.getByRole('tab', { name, exact: true }).click();
          await expect(page.getByRole('tab', { name, exact: true })).toHaveAttribute('aria-selected', 'true');
          const step = page.locator('.rb-step').first();
          if (name.trim() !== 'Identity' && await step.count()) {
            await step.click();
            await expect(page.locator('#rb-out')).not.toBeEmpty();
          }
          await audit(page);
          expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
        }
      }
    }
  }
});

test('network connection form has named controls and landmarks', async ({ page }) => {
  await page.goto('/net.html');
  await audit(page);
  await expect(page.getByLabel('Server URL')).toHaveValue('https://127.0.0.1:4433');
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
});

test('keyboard navigation keeps focus, titles, history, and bookmarked tabs', async ({ page }) => {
  await page.goto('/index.html');
  const catalog = page.getByRole('button', { name: 'Catalog', exact: true });
  await catalog.focus();
  await page.keyboard.press('Enter');
  await expect(catalog).toBeFocused();
  await expect(page).toHaveTitle('Catalog — ikigai');
  await expect(page).toHaveURL(/#catalog$/);
  await page.getByRole('button', { name: 'Demo', exact: true }).click();
  const timer = page.getByRole('tab', { name: 'Timer', exact: true });
  await timer.focus();
  await page.keyboard.press('Enter');
  await expect(timer).toBeFocused();
  await expect(page).toHaveTitle('Demo · Timer — ikigai');
  await expect(page).toHaveURL(/#demo\/timer$/);
  await page.goBack();
  await expect(page.getByRole('tab', { name: 'Basics', exact: true })).toHaveAttribute('aria-selected', 'true');
  await page.goBack();
  await expect(catalog).toBeFocused();
  await expect(page).toHaveTitle('Catalog — ikigai');
  await page.goBack();
  await expect(page).toHaveTitle('Home — ikigai');
  await page.goto('/index.html#demo/timer');
  await expect(timer).toHaveAttribute('aria-selected', 'true');
  await expect(timer).toBeFocused();
  await expect(page).toHaveTitle('Demo · Timer — ikigai');
  await page.reload();
  await expect(timer).toHaveAttribute('aria-selected', 'true');
  await expect(page).toHaveURL(/#demo\/timer$/);
});

test('terminal exposes a response log and keeps visible keyboard focus', async ({ page }) => {
  await page.goto('/index.html');
  const input = page.getByRole('textbox', { name: 'ikigai command' });
  await input.focus();
  await page.keyboard.press('Tab');
  await page.keyboard.press('Shift+Tab');
  await expect(input).toBeFocused();
  expect(await input.evaluate(el => getComputedStyle(el).outlineStyle)).toBe('solid');
  const log = page.getByRole('log', { name: 'Command transcript' });
  await input.fill('source urn:iki:fn:toUpper accessibility');
  await input.press('Enter');
  await expect(log).toContainText('ACCESSIBILITY');
  await expect(input).toBeFocused();
  await audit(page);
});

test('live displays pause across views and Cache scrolls with the keyboard', async ({ page }) => {
  await page.clock.install({ time: new Date('2026-11-02T12:00:00Z') });
  await page.goto('/index.html');
  await page.getByRole('button', { name: 'Control', exact: true }).click();
  const jobs = page.getByRole('region', { name: 'Time jobs readout' });
  await expect(jobs).toContainText('urn:time:now');
  const cache = page.getByRole('region', { name: 'Cache readout' });
  await expect(page.locator('#ik-clock-val')).toHaveText(/^\d{2}:\d{2}$/);
  await cache.focus();
  const initialJobs = await jobs.textContent();
  await expect.poll(() => jobs.textContent()).not.toBe(initialJobs);
  await expect(cache).toBeFocused();
  await page.getByRole('button', { name: 'Pause live updates' }).click();
  await expect(page.getByRole('button', { name: 'Resume live updates' })).toHaveAttribute('aria-pressed', 'true');
  const stopped = await jobs.textContent();
  const clock = await page.locator('#ik-clock-val').textContent();
  await cache.focus();
  await page.keyboard.press('End');
  await expect.poll(() => cache.evaluate(el => el.scrollTop)).toBeGreaterThan(0);
  // Observe more than two poll intervals; a single unchanged sample cannot prove pause.
  await page.waitForTimeout(2200);
  // Cross a clock minute as well: HH:MM would stay constant in a short sample.
  await page.clock.fastForward(61_000);
  await expect(jobs).toHaveText(stopped);
  await expect(page.locator('#ik-clock-val')).toHaveText(clock);
  await expect(cache).toBeFocused();
  await page.getByRole('button', { name: 'Home', exact: true }).click();
  await page.getByRole('button', { name: 'Control', exact: true }).click();
  await expect(page.locator('.ctl-live')).toHaveText('paused');
  await page.getByRole('button', { name: 'Resume live updates' }).click();
  const resumed = await jobs.textContent();
  await expect.poll(() => jobs.textContent()).not.toBe(resumed);
  await expect(page.locator('#ik-clock-val')).not.toHaveText(clock);
  await expect(page.locator('.ik-clock-colon')).toHaveCSS('animation-name', 'none');
});

test('disabled JavaScript and blocked WebAssembly explain the requirements', async ({ browser, page }) => {
  const context = await browser.newContext({ javaScriptEnabled: false });
  const noScript = await context.newPage();
  await noScript.goto('/index.html');
  // Playwright's text selector deliberately excludes noscript descendants.
  await expect(noScript.locator('noscript p')).toBeVisible();
  await expect(noScript.locator('noscript p')).toContainText('JavaScript is disabled.');
  await context.close();
  await page.route('**/*.wasm*', route => route.abort());
  await page.goto('/index.html');
  await expect(page.locator('#app')).toContainText('The WebAssembly kernel could not load.');
});
