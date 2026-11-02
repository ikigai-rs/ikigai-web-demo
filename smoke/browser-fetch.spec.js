const { test, expect } = require('@playwright/test');

const MAX_BODY = 16 * 1024 * 1024;

// Exercise the real wasm transport through the engine, replacing only the peer.
// No Content-Length: the transport must count bytes as they arrive.
async function fetchStream(page, sizes, { fail = false, empty = false } = {}) {
  await page.goto('/net.html');
  return page.evaluate(async ({ sizes, fail, empty }) => {
    const { default: init, evalLineAsync } = await import('/ikigai_web_demo.js');
    await init();
    let pulls = 0;
    let canceled = false;
    let stream;
    const originalFetch = window.fetch;
    window.fetch = async () => {
      stream = empty ? null : new ReadableStream({
        pull(controller) {
          if (pulls === sizes.length) {
            if (fail) controller.error(new Error('peer read failed'));
            else controller.close();
            return;
          }
          controller.enqueue(new Uint8Array(sizes[pulls++]).fill(120));
        },
        cancel() { canceled = true; },
      }, { highWaterMark: 0 });
      return new Response(stream, {
        status: empty ? 204 : 200,
        headers: { 'content-type': 'text/plain' },
      });
    };
    try {
      const result = JSON.parse(await evalLineAsync('source urn:http:get url=https://peer.invalid/body'));
      return { kind: result.kind, textLength: result.text.length,
        text: result.kind === 'error' || result.text.length < 1024 ? result.text : '', pulls, canceled,
        locked: stream?.locked ?? false };
    } finally {
      window.fetch = originalFetch;
    }
  }, { sizes, fail, empty });
}

test('browser fetch refuses a stream beyond 16 MiB and cancels before reading the rest', async ({ page }) => {
  const result = await fetchStream(page, [MAX_BODY, 1, 1024]);
  expect(result.pulls).toBe(2);
  expect(result.kind).toBe('error');
  expect(result.text).toContain(`exceeds ${MAX_BODY} bytes`);
  expect(result.canceled).toBe(true);
  expect(result.locked).toBe(false);
});

test('browser fetch refuses a single oversized chunk', async ({ page }) => {
  const result = await fetchStream(page, [MAX_BODY + 1]);
  expect(result.kind).toBe('error');
  expect(result.text).toContain(`exceeds ${MAX_BODY} bytes`);
  expect(result.canceled).toBe(true);
  expect(result.locked).toBe(false);
});

test('browser fetch accepts exactly 16 MiB before the endpoint applies its own limit', async ({ page }) => {
  const result = await fetchStream(page, [MAX_BODY - 1, 1]);
  // The endpoint has its own 8 MiB limit, applied AFTER transport collection.
  // Reaching that refusal proves the transport accepted the complete 16 MiB.
  expect(result.kind).toBe('error');
  expect(result.text).toContain('HTTP response body exceeds 8388608 bytes');
  expect(result.canceled).toBe(false);
  expect(result.locked).toBe(false);
});

test('browser fetch accepts a null response body', async ({ page }) => {
  const result = await fetchStream(page, [], { empty: true });
  expect(result.kind).toBe('output');
  expect(result.textLength).toBe(0);
});

test('browser fetch propagates a stream failure instead of returning a partial body', async ({ page }) => {
  const result = await fetchStream(page, [3], { fail: true });
  expect(result.kind).toBe('error');
  expect(result.text).toContain('http transport:');
  expect(result.locked).toBe(false);
});

test('browser fetch preserves a successful body across chunks', async ({ page }) => {
  const result = await fetchStream(page, [2, 0, 3]);
  expect(result.kind).toBe('output');
  expect(result.text).toBe('xxxxx');
  expect(result.canceled).toBe(false);
  expect(result.locked).toBe(false);
});
