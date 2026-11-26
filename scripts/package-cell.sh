#!/usr/bin/env bash
# Package <ikigai-cell> for a host page: every file a host copies, under a content-hashed
# name, into one directory, plus a manifest and the example host page.
#
#   scripts/package-cell.sh [DIST] [OUT]      # defaults: dist dist/cell
#
# DIST must hold the wasm-bindgen output (ikigai_web_demo.js, ikigai_web_demo_bg.wasm) and
# the element's sources (ikigai-cell.js, ikigai-cell.css), with ikigai-cell.js unstamped:
# it rewrites each `?v=__BUILD__` specifier to the hashed file it names, and refuses to
# finish if one is left. Nothing written here refers to another host. The smoke suite runs
# it over the CI-built tree before its tests (smoke/package-cell.setup.js).
set -euo pipefail

DIST=${1:-dist}
OUT=${2:-$DIST/cell}
HERE=$(cd "$(dirname "$0")/.." && pwd)

for f in ikigai_web_demo.js ikigai_web_demo_bg.wasm ikigai-cell.js ikigai-cell.css; do
  [[ -f "$DIST/$f" ]] || { echo "package-cell: $DIST/$f is missing (build the wasm first)" >&2; exit 1; }
done

rm -rf "$OUT"
mkdir -p "$OUT"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# hashed NAME EXT FILE: copy FILE into OUT as NAME.<16 hex of its SHA-256>.EXT; print that name.
hashed() {
  local h
  h=$(sha256sum "$3" | cut -c1-16)
  cp "$3" "$OUT/$1.$h.$2"
  echo "$1.$h.$2"
}

wasm=$(hashed ikigai_web_demo_bg wasm "$DIST/ikigai_web_demo_bg.wasm")

# The glue: its fallback wasm URL names the hashed wasm too (the element passes the URL
# explicitly, so this only matters to a host that imports the glue itself). A glue that
# imports a wasm-bindgen snippet would need that file shipped as well; refuse it here.
if grep -q "snippets/" "$DIST/ikigai_web_demo.js"; then
  echo "package-cell: the glue imports a wasm-bindgen snippet, which this script does not ship" >&2
  exit 1
fi
sed "s/'ikigai_web_demo_bg\.wasm'/'$wasm'/g" "$DIST/ikigai_web_demo.js" > "$work/glue.js"
glue=$(hashed ikigai_web_demo js "$work/glue.js")

css=$(hashed ikigai-cell css "$DIST/ikigai-cell.css")

sed -e "s#\./ikigai_web_demo\.js?v=__BUILD__#./$glue#g" \
    -e "s#\./ikigai_web_demo_bg\.wasm?v=__BUILD__#./$wasm#g" \
    -e "s#\./ikigai-cell\.css?v=__BUILD__#./$css#g" \
    "$DIST/ikigai-cell.js" > "$work/cell.js"
if grep -n "__BUILD__" "$work/cell.js"; then
  echo "package-cell: a ?v=__BUILD__ specifier above names no packaged file" >&2
  exit 1
fi
cell=$(hashed ikigai-cell js "$work/cell.js")

cat > "$OUT/manifest.json" <<JSON
{
  "entry": "$cell",
  "files": {
    "ikigai-cell.js": "$cell",
    "ikigai-cell.css": "$css",
    "ikigai_web_demo.js": "$glue",
    "ikigai_web_demo_bg.wasm": "$wasm"
  }
}
JSON

sed "s#__IKIGAI_CELL_JS__#$cell#g" "$HERE/cell/example.html" > "$OUT/index.html"

echo "package-cell: wrote $OUT"
ls -l "$OUT"
