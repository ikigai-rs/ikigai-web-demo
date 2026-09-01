// The smoke gate's configuration. One browser, one worker, no parallelism: the demo is
// a single page with a live 1s timer and a shared kernel, so overlapping sessions would
// only add noise.
const { defineConfig, devices } = require('@playwright/test');

const PORT = Number(process.env.SMOKE_PORT || 4173);
const BASE = `http://127.0.0.1:${PORT}`;

module.exports = defineConfig({
  testDir: __dirname,
  testMatch: '*.spec.js',
  // The test waits on a 1s job ticking twice and on the wasm fetch+instantiate; 90s is
  // slack for a cold CI runner, not an expected duration (it runs in ~15s warm).
  timeout: 90_000,
  expect: { timeout: 20_000 },
  fullyParallel: false,
  workers: 1,
  // One retry on CI absorbs genuine external flake — the page loads htmx from unpkg, and
  // a cold runner can miss a browser launch. It cannot mask the failure this gate exists
  // for: the wasm panic reproduced on EVERY load, so a retry reproduces it too.
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [['github'], ['list']] : [['list']],
  use: {
    baseURL: BASE,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: {
    command: 'node serve.js',
    cwd: __dirname,
    url: `${BASE}/index.html`,
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
    stdout: 'pipe',
  },
});
