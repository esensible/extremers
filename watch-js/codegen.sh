#!/bin/sh
# Regenerate extremers' solid-watch fragment outputs from extremers.schema.json:
# JSX types for the engine bundles, the Swift props/table for the watch app,
# and the JS table for the browser harness. Run from the repo root after
# `npm ci` in solid-watch/.
set -eu
cd "$(dirname "$0")/.."
node solid-watch/codegen/codegen.mjs --fragment watch-js/extremers.schema.json \
  --src watch-js/extremers.schema.json \
  --ts watch-js/generated/extremers-jsx.d.ts \
  --swift watch-js/generated/ExtremersNodeProps.swift \
  --js watch-js/generated/extremers-props.js "$@"
