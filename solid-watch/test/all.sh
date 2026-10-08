#!/bin/sh
# Everything CI runs for solid-watch, from the package root:
#   QJS=/path/to/qjs test/all.sh
# codegen up to date, test bundles rebuilt, fixtures regenerated (qjs and
# node must agree), fixtures unchanged vs git when CHECK_CLEAN=1, unit tests,
# import boundary.
set -eu
cd "$(dirname "$0")/.."
node codegen/codegen.mjs --check
npx rollup -c test/rollup.config.js --silent
node test/make-fixtures.mjs --qjs "${QJS:-qjs}"
node test/make-layout-fixtures.mjs
if [ "${CHECK_CLEAN:-0}" = 1 ]; then
  git diff --exit-code -- fixtures
fi
node --test --test-reporter=spec test/*.test.mjs
node codegen/codegen.mjs --fragment test/demo-fragment.json --ts test/types/demo-jsx.d.ts
npx tsc -p test/types
npx tsc -p test/types/tsconfig.fragment.json
echo "types ok"
node test/boundary.mjs
