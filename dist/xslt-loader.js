// Lazy loader for the dynamically-loadable XSLT module (Phase 2). The host wasm imports
// `xslt_transform` from here; the ~2.4 MB module wasm (xrust) is fetched + instantiated
// only on the FIRST call, then cached — so it costs nothing unless the Catalog page is
// opened. This little file is the "transport" between the two wasm instances (a JS byte
// channel); a future by-reference module would add a callback the other way.
let _mod = null;
let _loading = null;

async function load() {
  if (!_mod) {
    // Coalesce concurrent first-hits onto one instantiation.
    _loading = _loading || (async () => {
      const m = await import('./ikigai_xslt_module.js');
      await m.default(); // instantiate the module's wasm
      _mod = m;
    })();
    await _loading;
  }
  return _mod;
}

// By value: the host already resolved the refs and passes the bytes.
export async function xslt_transform(src, stylesheet, text) {
  return (await load()).transform(src, stylesheet, text);
}

// By reference: the host passes the IRIs; the module resolves them itself by calling
// back to the host's `hostResolve` (it's an import of the module wasm).
export async function xslt_transform_refs(srcUri, styleUri, text) {
  return (await load()).transform_refs(srcUri, styleUri, text);
}
