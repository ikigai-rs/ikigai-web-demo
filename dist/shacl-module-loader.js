// Lazy loader for the REAL ikigai-shacl crate (rudof) as a dynamically-loadable wasm module.
// It serves `urn:shacl:validate` by default; a page opened with `?shacl=js` uses shacl-engine
// (shacl-loader.js) instead, and then this file never fetches anything. Like
// xslt-loader.js and jsonld-loader.js, it is the "transport" between two wasm instances: a JS
// byte channel carrying the module-session protocol. The module's `hostCall` import resolves
// from the global scope (the host sets `globalThis.hostCall`), so a by-reference `shapes`
// comes back to the host kernel and the report inherits its golden threads.
//
// The module wasm is large (~7.8 MB raw, ~2 MB gzip: rudof plus an in-memory oxigraph), so
// it is fetched + instantiated only on the FIRST validation, then cached.
let _mod = null;
let _loading = null;

async function load() {
  if (!_mod) {
    // Coalesce concurrent first-hits onto one instantiation. A failed load is not cached,
    // so the next validation retries instead of re-awaiting a rejected promise.
    _loading = _loading || (async () => {
      const m = await import('./ikigai_shacl.js');
      await m.default(); // instantiate the module's wasm
      _mod = m;
    })().catch((e) => { _loading = null; throw e; });
    await _loading;
  }
  return _mod;
}

// Run one module session: hand the module the encoded `ModuleCall::Invoke` bytes and get
// back the encoded `ModuleReply` bytes.
export async function shaclInvokeSession(invokeBytes) {
  return (await load()).invoke_session(invokeBytes);
}
