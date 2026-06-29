// Browser SHACL validator behind `urn:shacl:validate`. rudof's validator is native-only
// (wasm-gated), so in the browser the SAME resource is served by the pure-JS shacl-engine.
// The output is held to the SAME contract as the native (rudof) crate — a ValidationOutcome
// {conforms, violations:[{focus_node, path, component}]} — proven equal in ikigai-shacl's
// js-parity suite. shacl-engine + rdf-ext + n3 are loaded lazily from a CDN (esm.sh) on the
// first validation, so page load doesn't depend on the CDN and the heavy libs aren't fetched
// until SHACL is actually used (the lazy-module ethos, in pure JS).

let libsPromise

function loadLibs () {
  if (!libsPromise) {
    // @zazuko/env is the RDF/JS environment that pairs with shacl-engine (both Zazuko); it
    // loads cleanly over the CDN where rdf-ext@2's multi-package default does not.
    libsPromise = Promise.all([
      import('https://esm.sh/@zazuko/env@2'),
      import('https://esm.sh/n3@2.1.0'),
      import('https://esm.sh/shacl-engine@1.1.1/Validator.js')
    ]).then(([envMod, n3, validatorMod]) => ({
      rdf: envMod.default,
      Parser: n3.Parser,
      Writer: n3.Writer,
      Validator: validatorMod.default
    }))
  }
  return libsPromise
}

// Bytewise compare, mirroring Rust's derive(Ord) on ValidationOutcome so the browser's
// violation order matches the native CLI's.
const cmp = (a, b) => (a < b ? -1 : a > b ? 1 : 0)

function outcomeJson (report) {
  const violations = report.results.map(r => ({
    focus_node: r.focusNode?.term?.value ?? null,
    path: r.path?.[0]?.predicates?.[0]?.value ?? null,
    component: r.constraintComponent?.value ?? null
  }))
  // (focus, path [None→'' sorts first, matching Option<String>], component)
  violations.sort((x, y) =>
    cmp(x.focus_node ?? '', y.focus_node ?? '') ||
    cmp(x.path ?? '', y.path ?? '') ||
    cmp(x.component ?? '', y.component ?? ''))
  return JSON.stringify({ conforms: report.conforms, violations }, null, 2)
}

function reportTurtle (Writer, report) {
  return new Promise((resolve, reject) => {
    const writer = new Writer({ prefixes: { sh: 'http://www.w3.org/ns/shacl#' } })
    for (const quad of report.dataset) writer.addQuad(quad)
    writer.end((err, result) => (err ? reject(err) : resolve(result)))
  })
}

// data + shapes Turtle → the report, rendered per `asType` (application/json → the portable
// ValidationOutcome; else the report graph as Turtle). Returns a string.
async function shaclValidate (dataTtl, shapesTtl, asType) {
  const { rdf, Parser, Writer, Validator } = await loadLibs()
  const parse = ttl => rdf.dataset(new Parser({ factory: rdf }).parse(ttl))
  const report = await new Validator(parse(shapesTtl), { factory: rdf })
    .validate({ dataset: parse(dataTtl) })
  if ((asType || '').split(';')[0].trim() === 'application/json') {
    return outcomeJson(report)
  }
  return reportTurtle(Writer, report)
}

window.shaclValidate = shaclValidate
