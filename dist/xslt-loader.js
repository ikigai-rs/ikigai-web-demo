// Lazy loader for the dynamically-loadable XSLT module. The ~2.4 MB module wasm (xrust) is
// fetched + instantiated only on the FIRST call, then cached — so it costs nothing unless
// the Catalog page is opened. This file is the "transport" between the two wasm instances:
// a JS byte channel carrying the module-session protocol. The module's `hostCall` import is
// resolved from the global scope (the host sets `globalThis.hostCall`), so the byte channel
// runs both ways — `Invoke` in, `HostCall`/`HostResult` exchanges back out.
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

// Run one module session: hand the module the encoded `ModuleCall::Invoke` bytes and get
// back the encoded `ModuleReply` bytes. The module pumps its sub-resource callbacks to the
// host's `hostCall` global while this awaits.
export async function xsltInvokeSession(invokeBytes) {
  return (await load()).invoke_session(invokeBytes);
}

// By value: the host already resolved the refs and passes the bytes (the simple path).
export async function xslt_transform(src, stylesheet, text) {
  return (await load()).transform(src, stylesheet, text);
}
