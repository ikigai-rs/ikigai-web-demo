// Before the suite: package <ikigai-cell> from the tree under test (../dist, the CI-built
// artifact in pages.yml) into ../dist/cell, with scripts/package-cell.sh. cell.spec.js runs
// those packaged, content-hashed files, the ones a host copies.
const { execFileSync } = require('node:child_process');
const path = require('node:path');

module.exports = () => {
  const root = path.resolve(__dirname, '..');
  execFileSync('bash', [path.join(root, 'scripts', 'package-cell.sh'), path.join(root, 'dist'), path.join(root, 'dist', 'cell')], {
    cwd: root,
    stdio: 'inherit',
  });
};
