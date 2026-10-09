# bitwire

*Working name; the public name is not decided yet.*

A compact, fast, general-purpose binary codec whose message types are
written in **JSON Schema**. Values are encoded by type (`type` plus the
OpenAPI `format` names: `int32`, `uint64`, `float`, ...). It is a codec, not
a validator: validation keywords are accepted and ignored, and so are
properties the schema does not declare. A compiler turns the schema into a
small **descriptor**; interpreters on each platform walk it to decode and
encode. There is no code generation and no compression.

The wire is byte-aligned like postcard (varints, raw little-endian floats,
length-prefixed strings), except that booleans, enum and union indices and
optional-field presence bits share **bit bytes**, so tags and flags cost
bits rather than bytes.

```json
{
  "type": "object",
  "properties": {
    "speed": { "type": "number", "format": "float" },
    "phase": {
      "oneOf": [
        { "type": "object", "properties": { "tag": { "const": "Idle" } }, "required": ["tag"] },
        { "type": "object", "properties": { "tag": { "const": "Running" },
                                            "since": { "type": "integer", "format": "uint64" } },
          "required": ["tag", "since"] }
      ]
    },
    "note": { "type": "string" }
  },
  "required": ["speed", "phase"]
}
```

`{"speed": 6.5, "phase": {"tag": "Idle"}}` encodes to 5 bytes: the float,
then one bit byte holding the union index and the presence bit of `note`.

## What is here

| Path | What |
|---|---|
| [`SPEC.md`](SPEC.md) | The schema subset, layout and value encodings, canonical form and hash, descriptor format, error codes, test-vector format |
| [`js/`](js/) | ES5 JavaScript, no dependencies: `compile.js` (schema → descriptor) and `codec.js` (load / decode / encode). Runs in Node, QuickJS and old browsers |
| [`vectors/`](vectors/) | Golden vectors shared by every implementation: round trips, decode and encode refusals, malformed descriptors |

Interpreters for Rust (`no_std`, no allocation), Swift and Kotlin/JVM will
follow; each must pass `vectors/`.

## JavaScript

```js
var bitwire = require('bitwire');           // or <script src="codec.js">: global `bitwire`

var desc = bitwire.compile(schema);         // Uint8Array; throws CompileError { errors: [{ path, message }] }
var d = bitwire.load(desc);                 // validate once, reuse
var bytes = bitwire.encode(d, { speed: 6.5, phase: { tag: 'Idle' } });
var value = bitwire.decode(d, bytes);
```

Refusals throw `WireError` with a `code`: `descriptor`, `truncated`,
`trailing`, `invalid`, `precision` (decode) and `shape`, `range` (encode).
See SPEC §7. Byte strings are `Uint8Array`s. A number written to a `float`
field is rounded to the nearest 32-bit float, as `Math.fround` does.

Compile errors name the offending location as a JSON pointer and say what to
write instead, so that a person, or a model writing the schema, can fix it in
one pass.

### Tests and benchmark

```sh
cd js
node test/node.js                 # the shared vectors, plus compiler refusals
qjs test/qjs.js                   # the same under quickjs-ng (classic-script loading)
node scripts/gen-vectors.js       # regenerate vectors/ (--check to verify they are current)
node bench/node.js                # a ~1 KB message vs JSON (qjs bench/qjs.js likewise)
```

## License

MIT, see [LICENSE](LICENSE).
