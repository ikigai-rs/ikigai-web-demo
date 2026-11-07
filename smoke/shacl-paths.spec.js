const { test, expect } = require('@playwright/test');

// Run the actual ?shacl=js loader and CDN validator without requiring the host wasm.
test('the JS SHACL loader preserves complex violation paths', async ({ page }) => {
  await page.route('**/shacl-test', route => route.fulfill({
    contentType: 'text/html', body: '<!doctype html><title>SHACL loader test</title>',
  }));
  await page.goto('/shacl-test');
  await page.addScriptTag({ url: '/shacl-loader.js' });
  const outcome = await page.evaluate(async () => {
    const prefixes = '@prefix sh: <http://www.w3.org/ns/shacl#> . @prefix ex: <http://example.org/> .';
    return JSON.parse(await window.shaclValidate(
      `${prefixes} ex:dave a ex:Person .`,
      `${prefixes}
        ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person ;
          sh:property [ sh:path ex:name ; sh:minCount 1 ] ;
          sh:property [ sh:path [ sh:alternativePath ( ex:email ex:phone ) ] ; sh:minCount 1 ] ;
          sh:property [ sh:path ( ex:employer ex:name ) ; sh:minCount 1 ] ;
          sh:property [ sh:path [ sh:inversePath ex:member ] ; sh:minCount 1 ] ;
          sh:property [ sh:path [ sh:oneOrMorePath ex:manager ] ; sh:minCount 1 ] .`,
      'application/json'));
  });
  expect(outcome).toEqual({
    conforms: false,
    violations: [
      '(http://example.org/email | http://example.org/phone)',
      '(http://example.org/employer / http://example.org/name)',
      '(http://example.org/manager)+',
      '^(http://example.org/member)',
      'http://example.org/name',
    ].map(path => ({
      focus_node: 'http://example.org/dave', path,
      component: 'http://www.w3.org/ns/shacl#MinCountConstraintComponent',
    })),
  });
});
