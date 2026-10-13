# bitwire specification

**Status:** draft, format version 1. *bitwire* is a working name; the public
name is not decided yet.

bitwire is a general-purpose binary codec whose message types are written in
a subset of **JSON Schema**. Data is encoded **by type**: `type` plus the
OpenAPI `format` vocabulary say how each value is laid out. JSON Schema is
the notation, not a validator: validation keywords (`minimum`, `maxItems`,
`pattern`, …) are accepted and ignored.

1. A **compiler** (JavaScript, `js/compile.js`) checks the schema against the
   subset, canonicalises it, and emits a **descriptor**: a compact, versioned
   table that is data, not bytecode.
2. **Interpreters** (one per platform) load the descriptor, validate it once,
   then decode and encode messages by walking it.

The wire is compact without compression: varints for integers, raw IEEE
floats, and one shared **bit byte** stream for booleans, enum, union and
variant indices and optional-field presence. Both ends must hold the identical
descriptor; there is no built-in compatibility or extension mechanism. The
schema hash in the descriptor identifies the schema so that mismatches can be
detected out of band.

The key words MUST, MUST NOT, SHOULD and MAY are used as in RFC 2119.

---

## 1. Principles

- **Encode by type; not a validator.** The declared type alone decides the
  encoding. The codec checks only what it must to write a value at all:
  - a required field is present;
  - the JS type matches the declared type;
  - an integer is whole and fits its width;
  - an enum string is one of the values;
  - a union tag is known;
  - a record with variants holds exactly one of its variants.

  It does not check anything else:
  - validation keywords (`minimum`, `maxItems`, `pattern`, …) are ignored;
  - properties the schema does not declare are ignored: never read, never
    written;
  - a double given for a `float` is rounded to the nearest binary32;
  - a lone UTF-16 surrogate in a string is written as U+FFFD.
- **Strict wire.** Decoders do check the bytes: malformed or ambiguous wire
  data (bad varints, unknown indices, bad UTF-8, stray bits, trailing bytes)
  is refused, so each message has one meaning.
- **One encoding per value.** Decoding then re-encoding gives the same bytes,
  except for NaN payloads (§4.4).
- **Fast.** The descriptor is validated once; the walk needs no name lookups
  and, in typed languages, no allocation beyond the decoded value.

---

## 2. The schema subset

A schema is a JSON value. The compiler accepts the forms below and rejects
everything else with a JSON-pointer path into the schema and a message that
says what to write instead.

### 2.1 General rules

- Every schema is a JSON object. Boolean schemas (`true`/`false`) are
  rejected.
- **Ignored keywords** (allowed anywhere, no effect on the encoding):
  - annotations: `title`, `description`, `$comment`, `examples`, `default`,
    `deprecated`, `readOnly`, `writeOnly`;
  - validation: `minimum`, `maximum`, `exclusiveMinimum`,
    `exclusiveMaximum`, `multipleOf`, `minLength`, `maxLength`, `pattern`,
    `contentMediaType`, `minItems`, `maxItems`, `uniqueItems`, `contains`,
    `minContains`, `maxContains`, `minProperties`, `maxProperties`,
    `dependentRequired`, `propertyNames`, `additionalProperties`,
    `unevaluatedProperties`;
  - `format` on a plain string (`date-time`, `email`, …).
- **Root-only keywords** (allowed, ignored): `$schema`, `$id`, `$defs`,
  `definitions`.
- **`$ref`** must be a local JSON-pointer reference (`#/$defs/Name`) and may
  not sit beside other keywords except ignored ones. References are inlined;
  recursion is rejected. Errors inside a referenced schema are reported at
  the referenced location.
- Each schema has one **kind**, chosen in this order: `$ref` → `oneOf` →
  `enum`/`const` → `type`. A `oneOf` is a tagged union or a record with
  variants, chosen as §2.2 says.
- Every other keyword is rejected, structural ones (`anyOf`, `allOf`, `not`,
  `if`/`then`/`else`, `prefixItems`, `patternProperties`, `nullable`, type
  lists such as `["integer", "null"]`, `type: "null"`) with a hint.

### 2.2 Kinds and their encodings

| Schema | Op | Wire |
|---|---|---|
| `{"type":"boolean"}` | `BOOL` | 1 bit in a bit byte |
| `{"type":"integer","format":"uint8"}` | `U8` | 1 raw byte |
| `{"type":"integer","format":"int8"}` | `I8` | 1 raw byte, two's complement |
| `{"type":"integer","format":"uint16"}` / `uint32` / `uint64` | `U16` / `U32` / `U64` | unsigned LEB128 varint |
| `{"type":"integer","format":"int16"}` / `int32` / `int64` | `I16` / `I32` / `I64` | zigzag LEB128 varint |
| `{"type":"integer"}` | `I64` | (no `format` means `int64`) |
| `{"type":"number","format":"float"}` | `F32` | 4 bytes, IEEE 754 binary32, little-endian |
| `{"type":"number","format":"double"}` or `{"type":"number"}` | `F64` | 8 bytes, IEEE 754 binary64, little-endian |
| `{"type":"string"}` | `STRING` | length in bytes (varint) + UTF-8 |
| `{"type":"string","contentEncoding":"base64"}` | `BYTES` | length (varint) + raw bytes |
| `{"enum":[...]}` / `{"const":"..."}` | `ENUM` | index in `bitLength(n−1)` bits of a bit byte |
| `{"type":"object",...}` | `OBJECT` | fields in declared order |
| `{"type":"array","items":S}` | `ARRAY` | length (varint) + items |
| `{"oneOf":[...]}`, tagged | `UNION` | branch index in `bitLength(n−1)` bits + the branch |
| `{"type":"object","properties":{...},"oneOf":[...]}`, variants | `VARIANTS` | fields in declared order + variant index in `bitLength(n−1)` bits + the variant |

**Integer formats** are the OpenAPI Format Registry names (`int8`, `int16`,
`int32`, `int64`, `uint8`, `uint16`, `uint32`, `uint64`); **number formats**
are `float` and `double`. Any other `format` on an integer or number is
rejected, because it would leave the width unknown.

- *Why `int64` by default:* JSON Schema's `integer` is signed and unbounded,
  so the widest signed type is the only default that refuses nothing a
  schema author would expect to work. Zigzag costs at most one bit of
  range.
- *Why `double` by default:* it is JSON's own number type, so it carries
  every JSON number exactly.
- *Why `int8`/`uint8` are raw bytes and wider types are varints:* a varint
  of an 8-bit value takes 1 or 2 bytes, while a raw byte always takes 1 and
  needs no loop. From 16 bits up, small values are the common case, and a
  varint saves bytes (postcard and protobuf make the same choice).

**Byte strings** use JSON Schema's `contentEncoding: "base64"` (OpenAPI's
`format: "byte"`/`"binary"` are deprecated in favour of it). The base64 is
only the JSON notation: on the wire, and in decoded values, they are raw
bytes.

**Enums** are strings only, distinct, 1 to 256 values. A `const` is a
one-value enum (0 bits). `type`, if given, must be `"string"`.

**Objects.** `properties` is ordered: the order in which the properties are
written is the wire order. Fields not listed in `required` are optional.
Only the declared properties are encoded; any other properties of a value
are ignored, and a decoded object has only declared properties. At most 256
properties. Property names MUST NOT look like array indices (`0`, `42`, …),
because JavaScript reorders such keys, and MUST NOT be `__proto__`.

**Arrays.** `items` is required. An array whose items always encode to
nothing (a `const`, an object of only `const` fields, …) is rejected: it
would carry only its length, and a decoder could be made to loop without
reading any data.

**Tagged unions.** A tagged union is a `oneOf` of 1 to 256 object schemas sharing
a **tag property**: a property that every branch lists in `required` and
defines as a one-value string enum (`{"const": "Name"}`), different in each
branch (modelled on JTD's `discriminator`). The tag is named by
`"discriminator": {"propertyName": "..."}` (only `propertyName` is
supported) or, without it, inferred as the only property that qualifies in
every branch. Branch order is wire order. The value carries the tag as one
of its properties (serde's "internally tagged" representation).

**Records with variants.** An object schema may carry `oneOf` beside its
`properties`. Each alternative names one **variant**: it is an object
schema (`type` `"object"` or absent) with exactly one property, the
variant's name, which it lists in `required`; that property's schema is the
variant's, and it is an object, or itself a record with variants, to any
depth:

```json
{"type": "object",
 "properties": {"line": {"type": "object", "properties": {"a": {"type": "integer"}}}},
 "required": ["line"],
 "oneOf": [
   {"properties": {"idle": {"type": "object"}}, "required": ["idle"]},
   {"properties": {"countdown": {"type": "object", "properties": {"gun": {"type": "integer"}}, "required": ["gun"]}}, "required": ["countdown"]}
 ]}
```

- The `properties` and `required` beside `oneOf` are ordinary **plain
  fields**, as in an object. A bare `{"oneOf": [...]}` of this shape is a
  record with variants and no plain fields. `type`, if given, is
  `"object"`.
- A value is an object holding the plain fields and **exactly one** variant
  key: `{"line": {"a": 1}, "countdown": {"gun": 1790000000000}}`. No tag
  property exists or is added: the variant's key is the tag (serde's
  "externally tagged" representation).
- 1 to 256 variants; variant order is wire order. The plain field names and
  the variant names are all distinct and follow the property name rules of
  **Objects** above.
- A variant's schema is an object or a record with variants, not a tagged
  union; to hold a union, put it in a property of the variant's object.

On the wire, the plain fields are written as an `OBJECT`'s, then the
variant's index in `bitLength(n−1)` bits, then the variant's object. A
record with one variant writes no index bits.

**Choosing the form of a `oneOf`.** A schema with `oneOf` is

1. a tagged union, when it has `discriminator`;
2. otherwise a record with variants, when every alternative is a variant
   alternative as above (after following `$ref`s): one property, listed in
   its `required`, whose schema is an object (`type: "object"`, or a bare
   `oneOf`, which must then be a record with variants too);
3. otherwise a tagged union, its tag property inferred.

In case 3, when every alternative is an object schema and none has a
property that could be a tag (a `const`), no tag can be inferred; the
compiler then explains why the alternatives are not variants, since that is
the form such a schema is closest to.

### 2.3 Limits

| Limit | Value |
|---|---|
| enum values, union branches, variants | 256 |
| object properties, plain fields of a record with variants | 256 |
| nesting depth (root = 1; a union branch or a variant counts as a level) | 32 |
| string, byte string and array length | 2^32 − 1 |

### 2.4 Values

| Kind | Value |
|---|---|
| boolean | `true` / `false` |
| integers | an integer of the declared width; hosts with native 64-bit integers decode unsigned formats as unsigned 64-bit and signed formats as signed 64-bit (`UInt64`/`Int64`, `u64`/`i64`, `ULong`/`Long`); JS decodes to a number (§4.3) |
| float / double | any IEEE value of that width, including ±0, ±infinity and NaN |
| string | a string (valid Unicode) |
| byte string | a byte array (`Uint8Array` in JS) |
| enum | one of its strings |
| object | an object; absent optional fields are omitted; present fields in declared order |
| array | an array |
| tagged union | the branch's object with the tag property first: `{"tag": "Both", "cross": 47, "time": 1790000025780}` |
| record with variants | an object: the plain fields as for an object, then the one variant's key: `{"line": {"a": 1}, "countdown": {"gun": 1790000000000}}` |

**Absent and null.** On encode, a field is **absent** when the host value
has no such key or property: a missing map key (Swift, Kotlin), or a
missing property (JS) — in JS a property whose value is `undefined` also
counts as missing, since JS cannot tell them apart in practice. An explicit
`null` (`nil`, `JSONNull`, …) is **present**: `null` is never a value of
any type, so it is refused as `shape`, for optional fields too. Properties
the schema does not declare are ignored. For a union, the branch is chosen
by the tag, and only that branch's declared properties are read. For a
record with variants, the variant is the one variant key present: none, or
more than one, is refused as `shape`, and so is a variant whose value is
not an object (`null` included, since a `null` key is present). A key that
is neither a plain field nor a variant is ignored, so a value holding only
undeclared keys has no variant.

**Name comparison.** Property names, variant names, enum values and union tags compare by
exact Unicode code points (equivalently, by their UTF-8 bytes), with no
normalisation: `"é"` (U+00E9) and `"é"` (U+0065 U+0301) are different
names. Hosts whose default string equality normalises (Swift's `String ==`)
MUST compare code points or UTF-8 bytes instead. This applies to the
descriptor's distinct-name rules (§6.3) as well as to encode lookups.

---

## 3. Layout: byte-aligned values and bit bytes

A message is a sequence of **bytes**, written in the order of a depth-first
walk of the value in schema order. Two kinds of item are written:

- **Byte values** (`U8`/`I8`, varints, floats, string and byte-string
  lengths and contents, array lengths) are written at the current byte
  position, which then advances past them.
- **Bit fields** (a `BOOL`, an `ENUM` index, a `UNION` index, a `VARIANTS`
  index, an optional field's presence bit) of width `w` (1 ≤ w ≤ 8) go into the **current bit
  byte**. If there is none yet, or it has fewer than `w` unused bits, the
  encoder **reserves the byte at the current position** as a new bit byte
  (initially zero) and the position advances by one. The field then takes
  the lowest unused bits: bit `used` to `used + w − 1`, where bit 0 is the
  least significant, and `used += w`. A 0-width field (a one-value enum, a
  one-branch union, a one-variant record) writes nothing and never reserves
  a bit byte.

So a bit byte sits where the first bit field that needed it occurred, and
later byte values follow it; bit fields that come after those byte values
still fill the same bit byte while it has room. Byte values never shift.

**One stream-wide bit byte.** There is a single current bit byte for the
whole message, not one per object, array, union or record: nested presence
bits, union and variant indices and booleans from any depth share it. A
record's variant index, written after its plain fields, shares a bit byte
with their presence bits whenever it fits. This packs the most
bits per byte, and a decoder reproduces it exactly because it walks the
descriptor in the same order.

**No straddling.** A field never spans two bit bytes: if it does not fit in
the bits left, those bits stay unused and a new bit byte is reserved. This
wastes at most `w − 1` bits per new bit byte (at most 7), and lets every
implementation read a field with one shift and one mask.

**Unused bits** of every bit byte (those above `used` when it is abandoned
for a new bit byte, or at the end of the message) MUST be zero. A decoder
refuses non-zero unused bits as `invalid`.

**Optional fields.** An optional field is written as its presence bit
(1 = present) immediately followed, if present, by its value. Required
fields have no presence bit.

### 3.1 Worked example

The schema `extreme-race/schema/view.json` (a `float` speed, then a union
`phase` whose `InSequence` branch holds a `uint64` start time and a union
`line` whose `Both` branch holds a `uint8` cross and a `uint64` time)
encodes

```json
{"speed": 23.200000762939453,
 "phase": {"tag": "InSequence", "start_time": 1790000300123,
           "line": {"tag": "Both", "cross": 47, "time": 1790000025780}}}
```

as 18 bytes:

| Offset | Bytes | Item |
|---|---|---|
| 0 | `9a 99 b9 41` | `speed`: binary32 0x41b9999a, little-endian |
| 4 | `0d` | bit byte, reserved by the `phase` index: bits 0–1 = `01` (InSequence), then bits 2–3 = `11` (line = Both); 0b0000_1101 |
| 5 | `db 80 d4 a2 8c 34` | `start_time` varint, written after the bit byte |
| 11 | `2f` | `cross` = 47, raw byte |
| 12 | `b4 a1 c3 a2 8c 34` | `time` varint |

The `line` index is written after `start_time` in walk order, yet it lands
in the bit byte at offset 4, which still had 6 unused bits.

---

## 4. Value encodings

### 4.1 Varints

A varint is LEB128: the value in 7-bit groups, least significant first, each
in one byte whose bit 7 is set when another byte follows.

- **Unsigned** types write the value; **signed** types write its zigzag,
  `(v << 1) ^ (v >> (W−1))` for width `W`, i.e. `2v` for `v ≥ 0` and
  `−2v − 1` for `v < 0`.
- A varint for a type of width `W` has at most `ceil(W/7)` bytes (3, 5 or
  10); the value (or zigzag) MUST be below `2^W` (for 64 bits: the 10th byte
  is 0 or 1); and it MUST be canonical: a varint of more than one byte MUST
  NOT end with a `0x00` byte. Decoders refuse violations as `invalid`.
- Lengths (string, byte string, array) are `uint32` varints.

These are the same bytes postcard and protobuf write for the same values.

### 4.2 Integers on encode

An integer is refused as `range` if it is not a whole number or is outside
the declared width (300 as `uint8`, −1 as `uint32`), because it cannot be
written. Integers are never rounded or clamped. Validation keywords such as
`minimum` are not consulted.

### 4.3 Integers beyond 2^53 in JavaScript

A JS number holds every integer up to 2^53 exactly, and only some above it.
The JS codec:

- **encodes** any integer-valued number within the type's range exactly
  (every such number is an exact integer), computing 64-bit varints in two
  32-bit halves;
- **decodes** a 64-bit value to the number that equals it, and refuses with
  `precision` when no JS number does (for example 2^53 + 1 or 2^64 − 1).

`precision` is not a wire error: it says the host's number type cannot hold
a valid value. Hosts with native 64-bit integers (Rust, Swift, Kotlin)
decode the full range and never raise it: unsigned formats decode to an
unsigned 64-bit integer, signed formats to a signed 64-bit integer (§2.4).

### 4.4 Floats

`float` and `double` are the raw IEEE 754 bit patterns, **little-endian**
(least significant byte first). Every value is representable and is encoded
as is, including ±0, subnormals, ±infinity and NaN.

- **Byte order.** Values are byte-aligned, so a byte order must be chosen.
  Little-endian is the native order of every target in view (ARM, RISC-V,
  x86, WebAssembly), the order postcard and protobuf use, and lets a typed
  host copy the bytes directly.
- **NaN.** Encoders MUST write NaN as the canonical quiet NaN
  (`0x7fc00000` / `0x7ff8000000000000`), because JavaScript cannot observe or
  control NaN payloads. Decoders accept any NaN bit pattern and produce NaN.
  This is the one place where re-encoding a decoded value can change bytes.
- **binary32 from a double.** A double given for a `float` field (in JS,
  every number) is rounded to the nearest binary32, ties to even: IEEE
  conversion, as `Math.fround`, `DataView.setFloat32`, Rust `as f32`, Swift
  `Float(x)` and Kotlin `toFloat()` all do. Values beyond the binary32 range
  become ±infinity. Decoding gives the rounded value (6.4 decodes as
  6.400000095367432).

### 4.5 Strings and byte strings

A length varint (bytes, < 2^32) then the bytes. Encoders refuse a string,
byte string or array whose length (UTF-8 bytes, bytes or items) is 2^32 or
more as `range`. Strings are UTF-8; decoders
refuse anything that is not strict UTF-8 (overlong forms, surrogates, values
above U+10FFFF, bad continuation bytes) as `invalid`. Encoders write a lone
UTF-16 surrogate (possible in JS, Swift and Kotlin strings) as U+FFFD, as
`TextEncoder` does.

### 4.6 The end of a message

After the root value, the message MUST end: any further byte is refused as
`trailing`. Reading past the end at any point is refused as `truncated`.
The unused-bits check (§3) of the last bit byte happens before the trailing
check.

---

## 5. Canonical form and schema hash

The descriptor carries an 8-byte **schema hash**: the first 8 bytes of the
SHA-256 of the UTF-8 bytes of the schema's **canonical text**. Schemas that
differ only in ignored keywords, formatting, keyword order or the use of
`$ref`/`$defs` have the same canonical text, hence the same descriptor.

The canonical text is compact JSON (no whitespace) with every `$ref`
inlined, in exactly these shapes and key orders:

| Kind | Canonical text |
|---|---|
| boolean | `{"type":"boolean"}` |
| integer | `{"type":"integer","format":"int64"}` (the format always written) |
| number | `{"type":"number","format":"double"}` (always written) |
| string | `{"type":"string"}` |
| byte string | `{"type":"string","contentEncoding":"base64"}` |
| enum / const | `{"enum":[values]}` |
| object | `{"type":"object","properties":{name:schema,...},"required":[names]}`, properties in declared order, `required` in declared property order, both always written |
| array | `{"type":"array","items":S}` |
| tagged union | `{"oneOf":[branches],"discriminator":{"propertyName":"tag"}}`, each branch an object with its tag property in place |
| record with variants | `{"type":"object","properties":{name:schema,...},"required":[names],"oneOf":[alternatives]}`: the plain fields exactly as for an object (`properties` and `required` always written, empty when there are none), then every alternative in declared order as `{"type":"object","properties":{"variant":schema},"required":["variant"]}` |

Strings are written as by ECMAScript `JSON.stringify`. The hash is an
identity, not an integrity check: interpreters do not recompute it.

---

## 6. Descriptor format

```
descriptor = version:u8  hash:8 bytes  nodeCount:varint  root:node  names
names      = { length:varint  utf8:bytes[length] }     (as many as the nodes use)
```

- `version` is 1. `nodeCount` is the total number of nodes.
- Descriptor varints are unsigned LEB128, at most 5 bytes, canonical, at
  most 2^32 − 1.

### 6.1 Nodes

A node starts with an **op byte**: the op in bits 0–6 and the **optional**
flag in bit 7. The optional flag is valid only on a direct field of an
`OBJECT` or a plain field of a `VARIANTS` (it gives the field a presence
bit), never on a union branch or a variant.

| Op | Name | After the op byte |
|---|---|---|
| 1 | `BOOL` | — |
| 2 | `U8` | — |
| 3 | `I8` | — |
| 4 | `U16` | — |
| 5 | `I16` | — |
| 6 | `U32` | — |
| 7 | `I32` | — |
| 8 | `U64` | — |
| 9 | `I64` | — |
| 10 | `F32` | — |
| 11 | `F64` | — |
| 12 | `STRING` | — |
| 13 | `BYTES` | — |
| 14 | `ENUM` | `n − 1`: u8 (1 to 256 values) |
| 15 | `OBJECT` | `fields`: varint, `size`: varint, then `fields` child nodes |
| 16 | `ARRAY` | `size`: varint, then 1 child node (the item) |
| 17 | `UNION` | `n − 1`: u8 (1 to 256 branches), `size`: varint, then `n` child nodes, each an `OBJECT` |
| 18 | `VARIANTS` | `fields`: varint, `n − 1`: u8 (1 to 256 variants), `size`: varint, then `fields` child nodes (the plain fields), then `n` child nodes (the variants), each an `OBJECT` or a `VARIANTS` |

`size` is the byte length of the node's children (for `VARIANTS`, its
fields and its variants together). It lets a walker skip a subtree (e.g.
the union branches or variants before the chosen one) without parsing it;
leaf nodes are self-delimiting. Bit widths (`bitLength(n − 1)`) are derived
from `n` at load, never stored. Children are in walk order (pre-order), so a
decoder reads the descriptor forward as it reads the message.

### 6.2 Names

Names are consumed in node order (pre-order), at the node that owns them,
before its children:

- `ENUM`: `n` names, the values in index order;
- `OBJECT`: `fields` names, the field names in order;
- `UNION`: `1 + n` names: the tag property name, then each branch's tag
  value. The branch objects then consume their own field names (which
  exclude the tag).
- `VARIANTS`: `fields + n` names: the plain field names in order, then the
  variant names in index order. The fields then consume their names, then
  the variants theirs.

Nodes store no name indices. Interpreters that build named values (JS,
Swift, Kotlin) resolve names once at load; one that does not need them
(e.g. Rust feeding a typed engine) still validates the names section.

### 6.3 Load-time validation

Every interpreter MUST check the following before using a descriptor, and
refuse it (`descriptor`) on any failure:

1. No read goes past the end of the descriptor.
2. `version` is 1.
3. Every descriptor varint follows §6.
4. Every op is 1–18; the optional flag appears only on direct `OBJECT`
   fields and plain `VARIANTS` fields.
5. `OBJECT` and `VARIANTS` have at most 256 fields; every `UNION` branch's
   op byte is exactly 15; every `VARIANTS` variant's op byte is exactly 15
   or 18.
6. Every `size` equals the byte length of the children actually parsed.
7. Nesting depth is at most 32 (root = 1, each child one deeper: a field,
   an item, a union branch, a variant).
8. No `ARRAY` item is **empty**. An empty node is an `ENUM` with `n = 1`,
   an `OBJECT` whose fields are all required and empty (including none),
   a `UNION` with `n = 1` whose branch is empty, or a `VARIANTS` whose
   fields are all required and empty (including none), with `n = 1` and an
   empty variant.
9. The number of nodes parsed equals `nodeCount`.
10. The names section holds exactly as many names as the nodes consume; each
    `length` fits in the remaining bytes; each is strict UTF-8; nothing
    follows the last name.
11. Names are distinct where they must be: an object's field names; a
    union's tag property name together with each branch's field names; a
    record's field names and variant names, as one set; an enum's values;
    a union's tag values. Distinct means different code points (§2.4), with
    no normalisation. No field, variant or tag property is named
    `__proto__`.

---

## 7. Errors

| Code | Raised by | Meaning |
|---|---|---|
| `descriptor` | load | the descriptor fails §6.3 |
| `truncated` | decode | the message ends before the value does |
| `trailing` | decode | bytes after the end of the value |
| `invalid` | decode | a non-canonical or too-wide varint, a length above 2^32 − 1, an enum, union or variant index ≥ n, non-zero unused bits in a bit byte, invalid UTF-8 |
| `precision` | decode (hosts without 64-bit integers) | a valid integer the host's number type cannot hold exactly (§4.3) |
| `shape` | encode | the value has the wrong type for its node (not a boolean / number / string / byte array / object / array as required; a variant's value is an object), it is an explicit `null` (§2.4), a required property is absent, a union's tag property is absent or not a string, or a record with variants holds no variant key or more than one |
| `range` | encode | the value has the right type but cannot be written: a non-whole number or an integer outside the width, a string not among the enum's values or the union's tags, or a string, byte string or array of length 2^32 or more |

These are the only encode refusals. Encoders never refuse a value for
anything else.

A decoder reports the first problem it meets while reading in order. When
an encoded value has several faults, which one is reported is not
specified; the test vectors contain only single-fault values.

---

## 8. Test vectors

`vectors/` holds golden vectors generated by `js/scripts/gen-vectors.js`
from the JavaScript implementation; every implementation MUST pass them.

**`vectors/codec.json`**: `{ "format": 1, "schemas": [...] }`. Each entry:

- `schema`, `canonical`, `hash`: for compilers (interpreters may ignore
  them, but the descriptor's header hash, as 16 lowercase hex digits, equals
  `hash`);
- `descriptor`: hex; MUST load;
- `cases`: `{ name, value, wire, decoded?, value_f64?, decoded_f64?,
  decode_only?, js? }`:
  - normally, `encode(value)` MUST give `wire` and `decode(wire)` MUST give
    `decoded` if present (when encoding drops or rounds something: extra
    properties, a double rounded to binary32), else `value`;
  - `decode_only: true`: only `decode(wire)` is checked (e.g. a
    non-canonical NaN, which encoders never write);
  - `js: "precision"`: an integer beyond what a JS number holds exactly.
    Hosts with native 64-bit integers treat it as a normal case; a JS-style host
    MUST refuse `decode(wire)` with `precision` and skip the encode;
- `decode_refusals`: `{ name, wire, error }`;
- `encode_refusals`: `{ name, value, error }`.

**`vectors/descriptors.json`**: `{ "format": 1, "refusals": [{ name,
descriptor, error }] }`; each MUST be refused at load.

**Representation.** Byte strings in the vector files, and values at `BYTES`
nodes, are lowercase hex (`""` is empty).

Integers are exact decimal JSON numbers, including beyond 2^53. Parse them
as **exact integers**, using arbitrary precision where needed: valid values
fit in 64 bits, but some encode-refusal values do not (2^64 for `uint64`,
2^63 for `int64`), and a parser that falls back to a double reads 2^64 + 1
as 2^64.

A float has its exact bits in `value_f64` (or `decoded_f64` for `decoded`)
unless it is a whole number of magnitude below 2^53. That covers fractions,
values of magnitude ≥ 2^53 (e.g. 3.4028234663852886e38, which some default
parsers, serde_json's among them, misread by one ulp), −0, and non-finite
values. The map goes from JSON pointer (`""` for the root) to 16 hex digits
of the big-endian binary64 pattern (a `float` value widened to binary64).
The JSON at that pointer holds the number, `0` for −0, or `null` when not
finite. Consumers MUST build mapped numbers from the map rather than the
JSON text, and compare floats by bits, with any NaN equal to any NaN. Decoded objects list keys in wire
order (a union's tag first; a record's plain fields, then its variant's
key); hosts that keep key order SHOULD check it.

The vectors cover each kind, records with variants among them: with and
without plain fields, nested, with one variant, sharing a bit byte with
presence bits, and a whole race state (a phase with variants beside
optional fields); and among
the refusals, a record value with no variant, two variants or a variant
that is not an object, and descriptors with a variant that is not an
`OBJECT` or `VARIANTS`, a field and a variant of the same name, a wrong
`size` and a truncated header. `js/test/compile-refusals.json` holds
schemas the compiler must refuse, with the path and the words its message
must contain.

---

## 9. Design notes

- **Postcard alignment.** Byte values use exactly postcard's encodings
  (LEB128, zigzag, raw `u8`/`i8`, little-endian floats, length-prefixed
  strings and sequences). Only booleans, enum, union and variant indices
  and option presence differ: they move into shared bit bytes. A bridge
  between the two formats is a straight walk of the descriptor.
- **Two forms of `oneOf`.** A tagged union puts its tag inside the value
  (serde's internally tagged enums, JTD's `discriminator`); a record with
  variants names the variant by its key (serde's externally tagged enums),
  so the value is exactly what the schema declares, with no added tag
  field. Both encode the same way: an index, then an object.
- **Why bit bytes:** a byte-aligned wire is fast to read and write, but tags
  and flags are mostly 1–3 bits. Packing them into reserved bytes keeps
  every byte value aligned while saving about a byte per union or option.
- **Prior art:** the pipeline (canonicalise, then named rules) follows JSON
  BinPack; the varint and float encodings follow postcard and protobuf;
  descriptor walking without allocation follows bitproto's C runtime;
  keeping names apart from the runtime table follows protobuf's upb
  MiniTables; tagged unions follow JSON Type Definition's `discriminator`;
  integer and number widths use the OpenAPI Format Registry.
