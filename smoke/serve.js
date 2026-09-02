// A static file server over ../dist — the EXACT bytes pages.yml is about to upload,
// including the `?v=<sha>` cache-bust stamp (a query is ignored here, as on Pages).
//
// Hand-rolled rather than `npx serve` for two reasons: no extra dependency to resolve
// at CI time, and an explicit MIME map. `application/wasm` is not optional — a browser
// refuses `WebAssembly.instantiateStreaming` on anything else, so a server that guesses
// wrong fails the smoke test for a reason that has nothing to do with the demo.
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');

const ROOT = path.resolve(__dirname, '..', 'dist');
const PORT = Number(process.env.SMOKE_PORT || 4173);

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.wasm': 'application/wasm',
  '.json': 'application/json; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.ico': 'image/x-icon',
  '.map': 'application/json; charset=utf-8',
  '.ts': 'text/plain; charset=utf-8',
};

const server = http.createServer((req, res) => {
  // Strip the query (`?v=<sha>`) and the fragment before resolving to a path.
  let rel = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
  if (rel.endsWith('/')) rel += 'index.html';
  const file = path.join(ROOT, path.normalize(rel));
  // Refuse to escape the document root — `..` in a request path is not a file read.
  if (!file.startsWith(ROOT + path.sep) && file !== ROOT) {
    res.writeHead(403).end('forbidden');
    return;
  }
  fs.readFile(file, (err, body) => {
    if (err) {
      res.writeHead(404, { 'content-type': 'text/plain' }).end('not found');
      return;
    }
    res.writeHead(200, {
      'content-type': TYPES[path.extname(file).toLowerCase()] || 'application/octet-stream',
      'cache-control': 'no-store',
    });
    res.end(body);
  });
});

server.listen(PORT, '127.0.0.1', () => {
  console.log(`smoke: serving ${ROOT} on http://127.0.0.1:${PORT}`);
});
