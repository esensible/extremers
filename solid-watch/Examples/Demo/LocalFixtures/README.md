Put extra fixture bundles here to run them in the demo (integrity check and
live view), one directory each, laid out like `fixtures/mutations/<name>/`:
`bundle.js`, `bundle.js.sha256`, `script.json`, `log.jsonl` (and
`fragment.json` for an app schema fragment). The live view shows the
`timers` bundle unless a `live.txt` here names another one (its fixture
states are then cycled). The contents are git-ignored.
