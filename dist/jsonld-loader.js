// Lazy loader for the dynamically-loadable JSON-LD module. The module wasm (the json-ld
// crate) is fetched + instantiated only on the FIRST call, then cached — so it costs nothing
// unless a urn:jsonld:* op is invoked. Like xslt-loader.js, this is the "transport" between
// the two wasm instances: a JS byte channel carrying the module-session protocol. The
// module's `hostCall` import resolves from the global scope (the host sets
// `globalThis.hostCall`), so the channel runs both ways.
let _mod = null;
let _loading = null;

async function load() {
  if (!_mod) {
    // Coalesce concurrent first-hits onto one instantiation.
    _loading = _loading || (async () => {
      const m = await import('./ikigai_jsonld.js');
      await m.default(); // instantiate the module's wasm
      _mod = m;
    })();
    await _loading;
  }
  return _mod;
}

// Run one module session: hand the module the encoded `ModuleCall::Invoke` bytes and get
// back the encoded `ModuleReply` bytes.
export async function jsonldInvokeSession(invokeBytes) {
  return (await load()).invoke_session(invokeBytes);
}
