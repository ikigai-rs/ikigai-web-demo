// <ikigai-cell>, as a host embeds it: the PACKAGED directory (dist/cell/, written by
// scripts/package-cell.sh in pages.yml) and its example host page, which plays a slide deck
// whose document-level key handler moves between slides on the arrow keys, the way reveal.js
// binds them. Every check runs the hashed files a host would copy, not the sources.
const { test, expect } = require('@playwright/test');
const AxeBuilder = require('@axe-core/playwright').default;
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');

const PACKAGE = path.resolve(__dirname, '..', 'dist', 'cell');
const PAGE = '/cell/index.html';

async function audit(page) {
  const result = await new AxeBuilder({ page }).analyze();
  expect(result.violations.map(({ id, nodes }) => ({
    id, nodes: nodes.map(({ target, failureSummary }) => ({ target, failureSummary })),
  }))).toEqual([]);
}

const cell = (page, n) => page.locator('ikigai-cell').nth(n);
async function run(page, n) {
  await cell(page, n).getByRole('button', { name: 'Run' }).click();
  await expect(cell(page, n).getByRole('status')).not.toBeEmpty();
}

test('the package is content-hashed local files, and the page fetches nothing else', async ({ page, baseURL }) => {
  const manifest = JSON.parse(fs.readFileSync(path.join(PACKAGE, 'manifest.json'), 'utf8'));
  for (const [logical, file] of Object.entries(manifest.files)) {
    const ext = path.extname(logical);
    const base = path.basename(logical, ext);
    const digest = crypto.createHash('sha256').update(fs.readFileSync(path.join(PACKAGE, file))).digest('hex');
    expect(file).toBe(`${base}.${digest.slice(0, 16)}${ext}`);
  }
  expect(manifest.entry).toBe(manifest.files['ikigai-cell.js']);
  expect(fs.readdirSync(PACKAGE).sort()).toEqual(
    [...Object.values(manifest.files), 'index.html', 'manifest.json'].sort());

  const requests = [];
  page.on('request', (request) => requests.push(request.url()));
  await page.goto(PAGE);
  for (let n = 0; n < 4; n++) await run(page, n);
  const origin = new URL(baseURL).origin;
  expect(requests.filter((url) => new URL(url).origin !== origin)).toEqual([]);
  const fetched = requests.map((url) => path.basename(new URL(url).pathname));
  for (const file of Object.values(manifest.files)) expect(fetched).toContain(file);
});

test('cells share one kernel and run only what the page declared', async ({ page }) => {
  const ready = [];
  await page.exposeFunction('kernelReady', (declaration) => ready.push(declaration));
  await page.addInitScript(() => {
    document.addEventListener('ikigai-kernel-ready', (e) => window.kernelReady(e.detail.declaration));
    window.cellResults = [];
    document.addEventListener('ikigai-cell-result', (e) => window.cellResults.push(e.detail));
  });
  await page.goto(PAGE);
  await expect(cell(page, 0).getByRole('textbox')).toHaveValue('source urn:deck:greeting | urn:iki:fn:toUpper');
  await run(page, 0);
  await expect(cell(page, 0).getByRole('status')).toContainText('HELLO, SLIDES');
  // A later cell reads what an earlier one wrote.
  await run(page, 1);
  await expect(cell(page, 1).getByRole('status')).toContainText('wrote 20 bytes to urn:deck:note');
  await run(page, 2);
  await expect(cell(page, 2).getByRole('status')).toContainText('written on slide two');
  // The page declared no file module and no authority.
  await run(page, 3);
  await expect(cell(page, 3).getByRole('status')).toContainText('no endpoint resolved for urn:file:notes.txt');
  // An edited cell runs its new code under the same declaration, and the declared
  // capability (none) is all a typed `cap reset` regains.
  const code = cell(page, 3).getByRole('textbox');
  await code.fill('cap reset\nsink urn:deck:greeting overwritten');
  await run(page, 3);
  await expect(cell(page, 3).getByRole('status')).toContainText('read-only');

  expect(ready).toEqual([{
    modules: ['fn'],
    resources: [
      { iri: 'urn:deck:greeting', type: 'text/plain', writable: false },
      { iri: 'urn:deck:note', type: 'text/plain;charset=utf-8', writable: true },
    ],
    capability: [],
  }]);
  const results = await page.evaluate(() => window.cellResults);
  expect(results.map((r) => r.ok)).toEqual([true, true, true, false, false]);
  expect(results[0].results[0]).toMatchObject({ command: 'source urn:deck:greeting | urn:iki:fn:toUpper', kind: 'output', text: 'HELLO, SLIDES' });
});

test('a Lisp buffer one cell leaves open ends with its run, and the next cell answers', async ({ page }) => {
  await page.goto(PAGE);
  const code = cell(page, 3).getByRole('textbox');
  // A cell that opens a Lisp buffer and leaves it open.
  await code.fill(':lisp\n(+ 1 2)');
  await run(page, 3);
  await expect(cell(page, 3).getByRole('status')).toContainText('lisp mode');
  await run(page, 0);
  await expect(cell(page, 0).getByRole('status')).toContainText('HELLO, SLIDES');
});

test('a declaration the kernel refuses says why, in every cell', async ({ page }) => {
  await page.route('**/cell/index.html', async (route) => {
    const response = await route.fetch();
    const body = (await response.text()).replace('"capability": []', '"capabilities": []');
    await route.fulfill({ response, body });
  });
  await page.addInitScript(() => {
    document.addEventListener('ikigai-kernel-error', (e) => { window.kernelError = e.detail.message; });
  });
  await page.goto(PAGE);
  await run(page, 0);
  await expect(cell(page, 0).getByRole('status')).toContainText('unknown key `capabilities`');
  expect(await page.evaluate(() => window.kernelError)).toContain('unknown key `capabilities`');
});

test('the Run control is a keyboard button, a focused cell keeps its keys, Escape returns to the host', async ({ page }) => {
  await page.goto(PAGE);
  const position = page.locator('#position');
  const first = cell(page, 0);
  // Tab reaches the code, then the Run button.
  await page.locator('#deck').focus();
  await page.keyboard.press('Tab');
  await expect(first.getByRole('textbox')).toBeFocused();
  await page.keyboard.press('Tab');
  const button = first.getByRole('button', { name: 'Run' });
  await expect(button).toBeFocused();
  expect(await button.evaluate((el) => el.tagName)).toBe('BUTTON');
  expect(await button.evaluate((el) => getComputedStyle(el).outlineStyle)).toBe('solid');
  await page.keyboard.press('Enter');
  await expect(first.getByRole('status')).toContainText('HELLO, SLIDES');
  await expect(button).toBeFocused();
  // Space activates it too.
  await first.getByRole('status').evaluate((el) => el.replaceChildren());
  await page.keyboard.press('Space');
  await expect(first.getByRole('status')).toContainText('HELLO, SLIDES');

  // The host's arrow keys do not fire from inside a cell: not from its button, not from
  // its code, where they move the caret.
  for (const target of [button, first.getByRole('textbox')]) {
    await target.focus();
    for (const key of ['ArrowRight', 'ArrowLeft', 'ArrowRight']) await page.keyboard.press(key);
    await expect(position).toHaveText('1');
  }
  // Ctrl+Enter in the code runs it.
  const second = cell(page, 1);
  await second.getByRole('textbox').focus();
  await page.keyboard.press('Control+Enter');
  await expect(second.getByRole('status')).toContainText('wrote 20 bytes');
  // Escape hands focus back to the host (the deck), whose keys then work again.
  await page.evaluate(() => {
    window.escaped = false;
    document.addEventListener('ikigai-cell-escape', () => { window.escaped = true; }, { once: true });
  });
  await page.keyboard.press('Escape');
  expect(await page.evaluate(() => window.escaped)).toBe(true);
  await expect(page.locator('#deck')).toBeFocused();
  await page.keyboard.press('ArrowRight');
  await expect(position).toHaveText('2');
});

test('output is a polite live region, and the cells pass axe on the host palette and their own', async ({ page }) => {
  await page.goto(PAGE);
  for (let n = 0; n < 4; n++) {
    const output = cell(page, n).getByRole('status');
    await expect(output).toHaveAttribute('aria-live', 'polite');
    await run(page, n);
  }
  await audit(page);
  // With no host tokens at all, the cell's own defaults.
  await page.addStyleTag({ content: ':root { --bg: initial; --fg: initial; --accent: initial; --code-bg: initial; --code-fg: initial; --font-mono: initial; } body { background: #fff; color: #1a1a1a; }' });
  await audit(page);
  await page.setViewportSize({ width: 320, height: 800 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('the cell takes the host color tokens', async ({ page }) => {
  await page.goto(PAGE);
  const frame = cell(page, 0).locator('.ikc-frame');
  const button = cell(page, 0).getByRole('button', { name: 'Run' });
  const style = (locator, prop) => locator.evaluate((el, p) => getComputedStyle(el)[p], prop);
  expect(await style(frame, 'backgroundColor')).toBe('rgb(241, 243, 245)'); // --code-bg
  expect(await style(frame, 'color')).toBe('rgb(29, 29, 27)'); // --code-fg
  expect(await style(button, 'borderTopColor')).toBe('rgb(11, 95, 138)'); // --accent
  expect(await style(frame, 'fontFamily')).toContain('ui-monospace'); // --font-mono
  await page.addStyleTag({ content: ':root { --code-bg: #102030; --code-fg: #fafafa; --accent: #ffcc00; --font-mono: "Courier New", monospace; }' });
  expect(await style(frame, 'backgroundColor')).toBe('rgb(16, 32, 48)');
  expect(await style(frame, 'color')).toBe('rgb(250, 250, 250)');
  expect(await style(button, 'borderTopColor')).toBe('rgb(255, 204, 0)');
  expect(await style(frame, 'fontFamily')).toContain('Courier New');
});

test('fresh output is marked with motion only when the reader allows it', async ({ page }) => {
  for (const [reducedMotion, animation] of [['no-preference', 'ikc-fresh'], ['reduce', 'none']]) {
    await page.emulateMedia({ reducedMotion });
    await page.goto(PAGE);
    await run(page, 0);
    const output = cell(page, 0).getByRole('status');
    expect(await output.evaluate((el) => getComputedStyle(el).animationName)).toBe(animation);
  }
});
