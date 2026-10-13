/*
 * bitwire compiler (working name): JSON Schema subset → descriptor bytes.
 * See ../SPEC.md for the subset, the canonical form and the descriptor.
 *
 * Data is encoded by type: `type` plus OpenAPI `format` choose the encoding.
 * Validation keywords (minimum, maxItems, pattern, ...) are accepted and
 * ignored. Plain ES5 plus typed arrays, no dependencies. Loadable as a
 * CommonJS module or as a classic script (which adds compile/canonicalize/
 * CompileError to a global `bitwire` object).
 */
(function (root, factory) {
  var api = factory();
  if (typeof module === 'object' && module && module.exports) {
    module.exports = api;
  } else {
    var ns = root.bitwire || (root.bitwire = {});
    for (var k in api) {
      if (Object.prototype.hasOwnProperty.call(api, k)) ns[k] = api[k];
    }
  }
}(this, function () {
  'use strict';

  var FORMAT_VERSION = 1;
  var MAX_DEPTH = 32;
  var MAX_FIELDS = 256;
  var MAX_CHOICES = 256;

  var OP = {
    BOOL: 1, U8: 2, I8: 3, U16: 4, I16: 5, U32: 6, I32: 7, U64: 8, I64: 9,
    F32: 10, F64: 11, STRING: 12, BYTES: 13, ENUM: 14, OBJECT: 15, ARRAY: 16, UNION: 17,
    VARIANTS: 18
  };
  var OPTIONAL = 0x80;

  var INTEGER_FORMATS = {
    int8: OP.I8, int16: OP.I16, int32: OP.I32, int64: OP.I64,
    uint8: OP.U8, uint16: OP.U16, uint32: OP.U32, uint64: OP.U64
  };
  var NUMBER_FORMATS = { 'float': OP.F32, 'double': OP.F64 };
  var DEFAULT_INTEGER = 'int64';
  var DEFAULT_NUMBER = 'double';

  var hasOwn = Object.prototype.hasOwnProperty;

  /** Ignored wherever they appear: annotations and validation-only keywords. */
  var IGNORED = {
    title: 1, description: 1, $comment: 1, examples: 1, 'default': 1,
    deprecated: 1, readOnly: 1, writeOnly: 1,
    minimum: 1, maximum: 1, exclusiveMinimum: 1, exclusiveMaximum: 1, multipleOf: 1,
    minLength: 1, maxLength: 1, pattern: 1, contentMediaType: 1,
    minItems: 1, maxItems: 1, uniqueItems: 1, contains: 1, minContains: 1, maxContains: 1,
    minProperties: 1, maxProperties: 1, dependentRequired: 1, propertyNames: 1,
    // Only declared properties are encoded; anything else is ignored.
    additionalProperties: 1, unevaluatedProperties: 1
  };
  /** Allowed, and ignored, at the root only. */
  var ROOT_ONLY = { $schema: 1, $id: 1, $defs: 1, definitions: 1 };

  /** Extra advice for keywords outside the subset. */
  var HINTS = {
    patternProperties: "only properties listed under 'properties' are encoded; list each one",
    dependentSchemas: 'not supported',
    allOf: 'not supported; write the combined schema out',
    anyOf: "use 'oneOf' with a tag property (a 'const' string in every branch)",
    not: 'not supported',
    'if': 'not supported',
    then: 'not supported',
    'else': 'not supported',
    prefixItems: 'tuples are not supported; use an object',
    unevaluatedItems: 'not supported',
    additionalItems: 'not supported',
    nullable: "leave the property out of 'required' to make it optional",
    'x-float': "use \"format\": \"float\" or \"double\"",
    $anchor: "use '$ref' with a JSON pointer such as '#/$defs/Name'",
    $dynamicRef: 'not supported',
    $dynamicAnchor: 'not supported',
    $recursiveRef: 'not supported'
  };

  // ---- errors -------------------------------------------------------------

  /** Thrown by compile/canonicalize; `errors` is [{ path, message }]. */
  function CompileError(errors) {
    this.name = 'CompileError';
    this.errors = errors;
    var lines = [];
    for (var i = 0; i < errors.length; i++) {
      lines.push((errors[i].path || '(root)') + ': ' + errors[i].message);
    }
    this.message = 'schema rejected:\n' + lines.join('\n');
    var s = new Error(this.message).stack;
    if (s) this.stack = s;
  }
  CompileError.prototype = Object.create(Error.prototype);
  CompileError.prototype.constructor = CompileError;

  function Ctx(root) {
    this.root = root;
    this.errors = [];
    this.seen = {};
    this.refStack = [];
  }
  Ctx.prototype.err = function (path, message) {
    var key = path + '\n' + message;
    if (this.seen[key]) return;
    this.seen[key] = true;
    this.errors.push({ path: path, message: message });
  };

  // ---- helpers ------------------------------------------------------------

  function isArray(v) {
    return Object.prototype.toString.call(v) === '[object Array]';
  }
  function isPlainObject(v) {
    return typeof v === 'object' && v !== null && !isArray(v);
  }
  function ptr(path, key) {
    return path + '/' + String(key).replace(/~/g, '~0').replace(/\//g, '~1');
  }
  function q(s) {
    return JSON.stringify(s);
  }
  function hasLoneSurrogate(s) {
    for (var i = 0; i < s.length; i++) {
      var c = s.charCodeAt(i);
      if (c >= 0xd800 && c <= 0xdbff) {
        var d = s.charCodeAt(i + 1);
        if (!(d >= 0xdc00 && d <= 0xdfff)) return true;
        i++;
      } else if (c >= 0xdc00 && c <= 0xdfff) {
        return true;
      }
    }
    return false;
  }
  function keysOf(o) {
    var out = [];
    for (var k in o) if (hasOwn.call(o, k)) out.push(k);
    return out;
  }

  /** Reports keywords of `k` that are not in `allowed`. */
  function allowOnly(ctx, k, allowed, path, what) {
    var keys = keysOf(k);
    for (var i = 0; i < keys.length; i++) {
      if (hasOwn.call(allowed, keys[i])) continue;
      var hint = hasOwn.call(HINTS, keys[i]) ? ' (' + HINTS[keys[i]] + ')' : '';
      ctx.err(ptr(path, keys[i]), "keyword '" + keys[i] + "' is not supported on " + what + hint);
    }
  }

  // ---- canonicalisation (one named rule per kind) -------------------------

  /**
   * The keywords of a schema that matter: ignored ones dropped, root-only
   * ones reported when not at the root. Null (reported) when the schema is
   * not a JSON object.
   */
  function keywords(ctx, node, path, isRoot) {
    if (node === true || node === false) {
      ctx.err(path, 'boolean schemas (true/false) are not supported; write an object schema');
      return null;
    }
    if (!isPlainObject(node)) {
      ctx.err(path, 'a schema must be a JSON object');
      return null;
    }
    var k = {};
    var keys = keysOf(node);
    for (var i = 0; i < keys.length; i++) {
      var key = keys[i];
      if (hasOwn.call(IGNORED, key)) continue;
      if (hasOwn.call(ROOT_ONLY, key)) {
        if (!isRoot) ctx.err(ptr(path, key), "'" + key + "' is only allowed at the root of the schema");
        continue;
      }
      k[key] = node[key];
    }
    return k;
  }

  function canon(ctx, node, path, isRoot) {
    var k = keywords(ctx, node, path, isRoot);
    if (!k) return null;
    if (hasOwn.call(k, '$ref')) return refRule(ctx, k, path);
    if (hasOwn.call(k, 'oneOf')) return oneOfRule(ctx, k, path);
    if (hasOwn.call(k, 'const') || hasOwn.call(k, 'enum')) return enumRule(ctx, k, path);
    if (!hasOwn.call(k, 'type')) {
      allowOnly(ctx, k, {}, path, 'this schema');
      ctx.err(path, "missing 'type': use boolean, integer, number, string, object or array, " +
        "or 'enum'/'const' for a set of strings, 'oneOf' for a tagged union or for variants, or '$ref'");
      return null;
    }
    var t = k.type;
    if (isArray(t)) {
      ctx.err(ptr(path, 'type'), "'type' must be a single string; to make a value optional, " +
        "leave its property out of 'required' instead of allowing null");
      return null;
    }
    switch (t) {
      case 'boolean': return booleanRule(ctx, k, path);
      case 'integer': return integerRule(ctx, k, path);
      case 'number': return numberRule(ctx, k, path);
      case 'string': return stringRule(ctx, k, path);
      case 'object': return objectRule(ctx, k, path);
      case 'array': return arrayRule(ctx, k, path);
      case 'null':
        ctx.err(ptr(path, 'type'), "type 'null' is not supported; to make a value optional, " +
          "leave its property out of 'required'");
        return null;
      default:
        ctx.err(ptr(path, 'type'), 'unknown type ' + q(t));
        return null;
    }
  }

  function refRule(ctx, k, path) {
    var r = resolveRef(ctx, k, path);
    if (!r) return null;
    ctx.refStack.push(r.frag);
    var out = canon(ctx, r.target, r.frag, r.frag === '');
    ctx.refStack.pop();
    return out;
  }

  /**
   * Follows the `$ref` of `k` (whose keywords are already filtered): returns
   * { target, frag } where frag is the target's JSON pointer, or null after
   * reporting why not. The caller pushes frag on ctx.refStack while it walks
   * the target.
   */
  function resolveRef(ctx, k, path) {
    var others = keysOf(k);
    if (others.length > 1) {
      ctx.err(path, "'$ref' cannot be combined with other keywords here (found: " +
        others.filter(function (x) { return x !== '$ref'; }).join(', ') + ')');
      return null;
    }
    var ref = k.$ref;
    var at = ptr(path, '$ref');
    if (typeof ref !== 'string' || ref.charAt(0) !== '#') {
      ctx.err(at, "only local references are supported, such as '#/$defs/Name'");
      return null;
    }
    var frag;
    try {
      frag = decodeURIComponent(ref.slice(1));
    } catch (e) {
      ctx.err(at, 'malformed reference ' + q(ref));
      return null;
    }
    if (frag !== '' && frag.charAt(0) !== '/') {
      ctx.err(at, "anchors are not supported; use a JSON pointer such as '#/$defs/Name'");
      return null;
    }
    var target = ctx.root;
    var tokens = frag === '' ? [] : frag.slice(1).split('/');
    for (var i = 0; i < tokens.length; i++) {
      var tok = tokens[i].replace(/~1/g, '/').replace(/~0/g, '~');
      if (target === null || typeof target !== 'object' || !hasOwn.call(target, tok)) {
        ctx.err(at, 'reference ' + q(ref) + ' does not point to anything in this schema');
        return null;
      }
      target = target[tok];
    }
    for (i = 0; i < ctx.refStack.length; i++) {
      if (ctx.refStack[i] === frag) {
        ctx.err(at, 'recursive schemas are not supported (' + q(ref) + ' refers back to itself)');
        return null;
      }
    }
    return { target: target, frag: frag };
  }

  /**
   * The schema a node stands for, following local `$ref`s without reporting
   * anything; null when a reference does not resolve (canonicalisation then
   * reports it). Used only to choose the form of a oneOf.
   */
  function peek(root, node) {
    for (var hops = 0; hops <= MAX_DEPTH; hops++) {
      if (!isPlainObject(node) || !hasOwn.call(node, '$ref')) return node;
      var ref = node.$ref;
      if (typeof ref !== 'string' || ref.charAt(0) !== '#') return null;
      var frag;
      try {
        frag = decodeURIComponent(ref.slice(1));
      } catch (e) {
        return null;
      }
      if (frag !== '' && frag.charAt(0) !== '/') return null;
      var target = root;
      var tokens = frag === '' ? [] : frag.slice(1).split('/');
      for (var i = 0; i < tokens.length; i++) {
        var tok = tokens[i].replace(/~1/g, '/').replace(/~0/g, '~');
        if (target === null || typeof target !== 'object' || !hasOwn.call(target, tok)) return null;
        target = target[tok];
      }
      node = target;
    }
    return null;
  }

  function booleanRule(ctx, k, path) {
    allowOnly(ctx, k, { type: 1 }, path, 'a boolean');
    return { kind: 'boolean' };
  }

  function integerRule(ctx, k, path) {
    allowOnly(ctx, k, { type: 1, format: 1 }, path, 'an integer');
    var f = hasOwn.call(k, 'format') ? k.format : DEFAULT_INTEGER;
    if (!hasOwn.call(INTEGER_FORMATS, f)) {
      ctx.err(ptr(path, 'format'), 'integer format must be one of int8, int16, int32, int64, ' +
        'uint8, uint16, uint32, uint64 (default int64); got ' + q(f));
      return null;
    }
    return { kind: 'integer', format: f };
  }

  function numberRule(ctx, k, path) {
    allowOnly(ctx, k, { type: 1, format: 1 }, path, 'a number');
    var f = hasOwn.call(k, 'format') ? k.format : DEFAULT_NUMBER;
    if (!hasOwn.call(NUMBER_FORMATS, f)) {
      var hint = hasOwn.call(INTEGER_FORMATS, f) ? "; for an integer use \"type\": \"integer\"" : '';
      ctx.err(ptr(path, 'format'), 'number format must be "float" (32-bit) or "double" ' +
        '(64-bit, the default); got ' + q(f) + hint);
      return null;
    }
    return { kind: 'number', format: f };
  }

  function stringRule(ctx, k, path) {
    allowOnly(ctx, k, { type: 1, format: 1, contentEncoding: 1 }, path, 'a string');
    if (hasOwn.call(k, 'contentEncoding')) {
      if (k.contentEncoding !== 'base64') {
        ctx.err(ptr(path, 'contentEncoding'), 'only "base64" (a byte string) is supported');
        return null;
      }
      return { kind: 'bytes' };
    }
    // `format` on strings (date-time, email, ...) is an annotation.
    return { kind: 'string' };
  }

  function enumRule(ctx, k, path) {
    allowOnly(ctx, k, { type: 1, 'enum': 1, 'const': 1 }, path, 'an enum');
    if (hasOwn.call(k, 'type') && k.type !== 'string') {
      ctx.err(ptr(path, 'type'), "'enum' and 'const' values must be strings, so 'type' must be 'string'");
      return null;
    }
    var values;
    var at;
    var isConst = hasOwn.call(k, 'const');
    if (isConst) {
      if (hasOwn.call(k, 'enum')) {
        ctx.err(path, "use either 'const' or 'enum', not both");
        return null;
      }
      values = [k['const']];
      at = ptr(path, 'const');
    } else {
      values = k['enum'];
      at = ptr(path, 'enum');
      if (!isArray(values) || values.length === 0) {
        ctx.err(at, 'must be a non-empty array of strings');
        return null;
      }
    }
    if (values.length > MAX_CHOICES) {
      ctx.err(at, 'more than ' + MAX_CHOICES + ' values');
      return null;
    }
    var seen = {};
    var ok = true;
    for (var i = 0; i < values.length; i++) {
      var vat = isConst ? at : ptr(at, i);
      if (typeof values[i] !== 'string') {
        ctx.err(vat, 'enum and const values must be strings');
        ok = false;
      } else if (hasLoneSurrogate(values[i])) {
        ctx.err(vat, 'value is not valid Unicode (lone surrogate)');
        ok = false;
      } else if (seen['$' + values[i]]) {
        ctx.err(vat, 'duplicate value ' + q(values[i]));
        ok = false;
      } else {
        seen['$' + values[i]] = true;
      }
    }
    return ok ? { kind: 'enum', values: values.slice() } : null;
  }

  function checkName(ctx, name, at) {
    if (/^(0|[1-9][0-9]*)$/.test(name) && Number(name) <= 4294967294) {
      ctx.err(at, 'property name ' + q(name) + ' looks like an array index; JavaScript ' +
        'reorders such keys, so the field order would be lost. Rename it');
      return false;
    }
    if (name === '__proto__') {
      ctx.err(at, "property name '__proto__' is not allowed");
      return false;
    }
    if (hasLoneSurrogate(name)) {
      ctx.err(at, 'property name is not valid Unicode (lone surrogate)');
      return false;
    }
    return true;
  }

  function objectRule(ctx, k, path) {
    allowOnly(ctx, k, { type: 1, properties: 1, required: 1 }, path, 'an object');
    var fields = objectFields(ctx, k, path);
    return fields ? { kind: 'object', fields: fields } : null;
  }

  /** The fields of an object schema's `properties`/`required`, or null. */
  function objectFields(ctx, k, path) {
    var props = hasOwn.call(k, 'properties') ? k.properties : {};
    if (!isPlainObject(props)) {
      ctx.err(ptr(path, 'properties'), 'must be an object');
      return null;
    }
    var names = keysOf(props);
    if (names.length > MAX_FIELDS) {
      ctx.err(ptr(path, 'properties'), 'more than ' + MAX_FIELDS + ' properties');
      return null;
    }
    var required = {};
    if (hasOwn.call(k, 'required')) {
      var req = k.required;
      if (!isArray(req)) {
        ctx.err(ptr(path, 'required'), 'must be an array of property names');
        return null;
      }
      for (var i = 0; i < req.length; i++) {
        var rat = ptr(ptr(path, 'required'), i);
        if (typeof req[i] !== 'string') ctx.err(rat, 'must be a string');
        else if (!hasOwn.call(props, req[i])) ctx.err(rat, q(req[i]) + " is not in 'properties'");
        else if (required['$' + req[i]]) ctx.err(rat, 'duplicate ' + q(req[i]));
        else required['$' + req[i]] = true;
      }
    }
    var fields = [];
    var ok = true;
    for (i = 0; i < names.length; i++) {
      var at = ptr(ptr(path, 'properties'), names[i]);
      if (!checkName(ctx, names[i], at)) { ok = false; continue; }
      var child = canon(ctx, props[names[i]], at, false);
      if (!child) { ok = false; continue; }
      fields.push({ name: names[i], schema: child, optional: !required['$' + names[i]] });
    }
    return ok ? fields : null;
  }

  /** True when a value of this schema always encodes to nothing. */
  function isZero(n) {
    switch (n.kind) {
      case 'enum':
        return n.values.length === 1;
      case 'object':
        return fieldsZero(n.fields);
      case 'union':
        return n.branches.length === 1 && isZero(branchBody(n, 0));
      case 'variants':
        return fieldsZero(n.fields) && n.variants.length === 1 && isZero(n.variants[0].schema);
    }
    return false;
  }

  function fieldsZero(fields) {
    for (var i = 0; i < fields.length; i++) {
      if (fields[i].optional || !isZero(fields[i].schema)) return false;
    }
    return true;
  }

  function arrayRule(ctx, k, path) {
    allowOnly(ctx, k, { type: 1, items: 1 }, path, 'an array');
    if (!hasOwn.call(k, 'items')) {
      ctx.err(path, "arrays need 'items' (the schema of every element)");
      return null;
    }
    if (isArray(k.items)) {
      ctx.err(ptr(path, 'items'), 'tuple arrays are not supported; use an object');
      return null;
    }
    var item = canon(ctx, k.items, ptr(path, 'items'), false);
    if (!item) return null;
    if (isZero(item)) {
      ctx.err(ptr(path, 'items'), 'every item would encode to nothing (it can only hold one ' +
        'value), so the array carries only its length; use an integer instead');
      return null;
    }
    return { kind: 'array', items: item };
  }

  /** A single-value enum property that is required: a candidate tag. */
  function tagValue(branch, name) {
    for (var i = 0; i < branch.fields.length; i++) {
      var f = branch.fields[i];
      if (f.name === name) {
        return !f.optional && f.schema.kind === 'enum' && f.schema.values.length === 1
          ? f.schema.values[0] : null;
      }
    }
    return null;
  }

  /** Branch i of a union as the object that is encoded: without the tag. */
  function branchBody(u, i) {
    var fields = [];
    var b = u.branches[i];
    for (var j = 0; j < b.fields.length; j++) {
      if (b.fields[j].name !== u.tag) fields.push(b.fields[j]);
    }
    return { kind: 'object', fields: fields };
  }

  // ---- oneOf: a tagged union, or a record with variants -------------------

  var VARIANT_EXAMPLE = '{"properties": {"countdown": {"type": "object", "properties": ' +
    '{"gun": {"type": "integer"}}, "required": ["gun"]}}, "required": ["countdown"]}';

  /**
   * Chooses the form of a oneOf (SPEC §2.2): with a discriminator, a tagged
   * union; else, when every alternative is a variant (one required property
   * whose schema is an object), a record with variants; else the tagged
   * union's inference. When no alternative could hold a tag either, the
   * refusal explains the variants form, the one the author most likely meant.
   */
  function oneOfRule(ctx, k, path) {
    var list = k.oneOf;
    if (hasOwn.call(k, 'discriminator') || !isArray(list) || list.length === 0) {
      return unionRule(ctx, k, path);
    }
    var miss = -1;
    var reason = null;
    var anyTag = false;
    var allObjects = true;
    for (var i = 0; i < list.length; i++) {
      var s = variantShape(ctx.root, peek(ctx.root, list[i]));
      if (!s.object) allObjects = false;
      if (s.tag) anyTag = true;
      if (s.reason !== null && miss < 0) {
        miss = i;
        reason = s.reason;
      }
    }
    if (miss < 0) return variantsRule(ctx, k, path);
    if (anyTag || !allObjects) return unionRule(ctx, k, path);
    ctx.err(ptr(path, 'oneOf'), 'no alternative has a tag property (a "const" string), so this ' +
      'oneOf is read as a record with variants, but ' + ptr(ptr(path, 'oneOf'), miss) +
      ' is not a variant: ' + reason + '. Write each alternative as one variant: a single ' +
      "property, the variant's name, listed in its 'required', whose schema is an object, e.g. " +
      VARIANT_EXAMPLE + '. For a tagged union instead, give every alternative the same required ' +
      'tag property holding a different "const" string');
    return null;
  }

  /** An object-valued schema: `type: "object"`, or a bare oneOf (variants). */
  function isObjectSchema(s) {
    return isPlainObject(s) &&
      (s.type === 'object' || (!hasOwn.call(s, 'type') && hasOwn.call(s, 'oneOf')));
  }

  function describe(s) {
    if (!isPlainObject(s)) return 'not an object schema';
    if (hasOwn.call(s, 'type')) return '{"type": ' + q(s.type) + '}';
    if (hasOwn.call(s, 'const') || hasOwn.call(s, 'enum')) return 'an enum';
    return 'a schema without "type": "object"';
  }

  /**
   * How a oneOf alternative (refs followed) fits the two forms: `object`, it
   * is an object schema (type "object" or none); `tag`, it has a property
   * that could be a union tag (a one-value enum); `reason`, why it is not a
   * variant, or null when it is one.
   */
  function variantShape(root, alt) {
    var out = { object: false, tag: false, reason: null };
    if (!isPlainObject(alt) || hasOwn.call(alt, '$ref') || hasOwn.call(alt, 'oneOf') ||
        (hasOwn.call(alt, 'type') && alt.type !== 'object')) {
      out.reason = 'it is not an object schema';
      return out;
    }
    out.object = true;
    var props = hasOwn.call(alt, 'properties') ? alt.properties : {};
    if (!isPlainObject(props)) {
      out.reason = "its 'properties' is not an object";
      return out;
    }
    var names = keysOf(props);
    for (var i = 0; i < names.length; i++) {
      var p = peek(root, props[names[i]]);
      if (isPlainObject(p) && (hasOwn.call(p, 'const') ||
          (isArray(p['enum']) && p['enum'].length === 1))) {
        out.tag = true;
      }
    }
    if (names.length === 0) {
      out.reason = 'it has no properties';
    } else if (names.length > 1) {
      out.reason = 'it has ' + names.length + ' properties (' + names.map(q).join(', ') +
        "); a variant alternative has exactly one, the variant's name";
    } else if (!isArray(alt.required) || alt.required.indexOf(names[0]) < 0) {
      out.reason = 'its property ' + q(names[0]) + " is not listed in its 'required'";
    } else if (!isObjectSchema(peek(root, props[names[0]]))) {
      out.reason = 'the schema of ' + q(names[0]) + ' is ' + describe(peek(root, props[names[0]])) +
        ', not an object; a variant holds an object, so wrap the value in one';
    }
    return out;
  }

  function variantsRule(ctx, k, path) {
    allowOnly(ctx, k, { type: 1, properties: 1, required: 1, oneOf: 1 }, path, 'an object with variants');
    if (hasOwn.call(k, 'type') && k.type !== 'object') {
      ctx.err(ptr(path, 'type'), "an object with variants (a 'oneOf' of one-property alternatives) " +
        "is an object, so 'type' may only be 'object'");
      return null;
    }
    var at = ptr(path, 'oneOf');
    var list = k.oneOf;
    if (list.length > MAX_CHOICES) {
      ctx.err(at, 'more than ' + MAX_CHOICES + ' variants');
      return null;
    }
    var fields = objectFields(ctx, k, path);
    var ok = fields !== null;
    var seen = {};
    for (var i = 0; fields && i < fields.length; i++) seen['$' + fields[i].name] = 'field';
    var variants = [];
    for (i = 0; i < list.length; i++) {
      var v = variantAlternative(ctx, list[i], ptr(at, i), seen);
      if (v) variants.push(v);
      else ok = false;
    }
    return ok ? { kind: 'variants', fields: fields, variants: variants } : null;
  }

  /** One alternative of a record with variants (known to be variant-shaped). */
  function variantAlternative(ctx, raw, path, seen) {
    var pushed = 0;
    var out = null;
    var k = keywords(ctx, raw, path, false);
    while (k && hasOwn.call(k, '$ref')) {
      var r = resolveRef(ctx, k, path);
      if (!r) { k = null; break; }
      ctx.refStack.push(r.frag);
      pushed++;
      path = r.frag;
      k = keywords(ctx, r.target, path, path === '');
    }
    if (k) out = variantBody(ctx, k, path, seen);
    while (pushed--) ctx.refStack.pop();
    return out;
  }

  function variantBody(ctx, k, path, seen) {
    allowOnly(ctx, k, { type: 1, properties: 1, required: 1 }, path, 'a variant alternative');
    var name = keysOf(k.properties)[0];
    var at = ptr(ptr(path, 'properties'), name);
    var ok = true;
    var count = 0;
    for (var j = 0; j < k.required.length; j++) {
      var rat = ptr(ptr(path, 'required'), j);
      if (k.required[j] !== name) {
        ctx.err(rat, (typeof k.required[j] === 'string' ? q(k.required[j]) : 'a non-string') +
          " is not in 'properties'; a variant alternative requires only its one property, the variant");
        ok = false;
      } else if (count++) {
        ctx.err(rat, 'duplicate ' + q(name));
        ok = false;
      }
    }
    if (!checkName(ctx, name, at)) return null;
    if (seen['$' + name] === 'field') {
      ctx.err(at, q(name) + " is both a plain property and a variant; a record's field and " +
        'variant names must all differ, so rename one');
      ok = false;
    } else if (seen['$' + name] === 'variant') {
      ctx.err(at, q(name) + ' names two variants; rename one');
      ok = false;
    }
    seen['$' + name] = 'variant';
    var schema = canon(ctx, k.properties[name], at, false);
    if (!schema) return null;
    if (schema.kind !== 'object' && schema.kind !== 'variants') {
      ctx.err(at, 'a variant holds an object (with or without variants of its own), but this is a ' +
        'tagged union; put the union in a property of an object, e.g. {"type": "object", ' +
        '"properties": {"value": <the union>}, "required": ["value"]}');
      return null;
    }
    return ok ? { name: name, schema: schema } : null;
  }

  function unionRule(ctx, k, path) {
    allowOnly(ctx, k, { oneOf: 1, discriminator: 1, type: 1 }, path, 'a oneOf');
    if (hasOwn.call(k, 'type') && k.type !== 'object') {
      ctx.err(ptr(path, 'type'), "a oneOf is a tagged union of objects, so 'type' may only be 'object'");
      return null;
    }
    var at = ptr(path, 'oneOf');
    var list = k.oneOf;
    if (!isArray(list) || list.length === 0) {
      ctx.err(at, 'must be a non-empty array of object schemas');
      return null;
    }
    if (list.length > MAX_CHOICES) {
      ctx.err(at, 'more than ' + MAX_CHOICES + ' branches');
      return null;
    }
    var branches = [];
    var ok = true;
    for (var i = 0; i < list.length; i++) {
      var b = canon(ctx, list[i], ptr(at, i), false);
      if (!b) { ok = false; continue; }
      if (b.kind !== 'object') {
        ctx.err(ptr(at, i), 'each oneOf alternative must be an object: a tagged-union branch ' +
          'with a tag property, e.g. {"type": "object", "properties": {"tag": {"const": "Name"}}, ' +
          '"required": ["tag"]}, or a variant, e.g. {"properties": {"name": {"type": "object"}}, ' +
          '"required": ["name"]}');
        ok = false;
        continue;
      }
      branches.push(b);
    }
    if (!ok) return null;

    var tag;
    if (hasOwn.call(k, 'discriminator')) {
      var d = k.discriminator;
      var dat = ptr(path, 'discriminator');
      if (!isPlainObject(d) || typeof d.propertyName !== 'string') {
        ctx.err(dat, 'must be {"propertyName": "<tag property>"}');
        return null;
      }
      var dkeys = keysOf(d);
      for (i = 0; i < dkeys.length; i++) {
        if (dkeys[i] !== 'propertyName') {
          ctx.err(ptr(dat, dkeys[i]), "only 'propertyName' is supported in 'discriminator'");
          return null;
        }
      }
      tag = d.propertyName;
    } else {
      var candidates = [];
      for (i = 0; i < branches[0].fields.length; i++) {
        var name = branches[0].fields[i].name;
        var every = true;
        for (var j = 0; j < branches.length; j++) {
          if (tagValue(branches[j], name) === null) { every = false; break; }
        }
        if (every) candidates.push(name);
      }
      if (candidates.length === 0) {
        ctx.err(at, 'this oneOf is read as a tagged union (an alternative has a "const" ' +
          'property), so it needs a tag property: the same required property in every ' +
          'branch, holding a different "const" string in each (e.g. "tag": {"const": "Active"})');
        return null;
      }
      if (candidates.length > 1) {
        ctx.err(path, 'several properties could be the tag (' + candidates.join(', ') +
          '); name one with "discriminator": {"propertyName": "..."}');
        return null;
      }
      tag = candidates[0];
    }
    var seen = {};
    for (i = 0; i < branches.length; i++) {
      var v = tagValue(branches[i], tag);
      if (v === null) {
        ctx.err(ptr(at, i), 'branch has no tag: property ' + q(tag) +
          ' must be required and hold a single "const" string');
        ok = false;
      } else if (seen['$' + v]) {
        ctx.err(ptr(at, i), 'duplicate tag value ' + q(v));
        ok = false;
      } else {
        seen['$' + v] = true;
      }
    }
    return ok ? { kind: 'union', tag: tag, branches: branches } : null;
  }

  // ---- canonical JSON and hash --------------------------------------------

  function canonicalJson(n) {
    var parts, i;
    switch (n.kind) {
      case 'boolean':
        return '{"type":"boolean"}';
      case 'integer':
        return '{"type":"integer","format":' + q(n.format) + '}';
      case 'number':
        return '{"type":"number","format":' + q(n.format) + '}';
      case 'string':
        return '{"type":"string"}';
      case 'bytes':
        return '{"type":"string","contentEncoding":"base64"}';
      case 'enum':
        parts = [];
        for (i = 0; i < n.values.length; i++) parts.push(q(n.values[i]));
        return '{"enum":[' + parts.join(',') + ']}';
      case 'object':
        return canonicalObject(n.fields) + '}';
      case 'variants':
        parts = [];
        for (i = 0; i < n.variants.length; i++) {
          var v = n.variants[i];
          parts.push('{"type":"object","properties":{' + q(v.name) + ':' + canonicalJson(v.schema) +
            '},"required":[' + q(v.name) + ']}');
        }
        return canonicalObject(n.fields) + ',"oneOf":[' + parts.join(',') + ']}';
      case 'array':
        return '{"type":"array","items":' + canonicalJson(n.items) + '}';
      case 'union':
        parts = [];
        for (i = 0; i < n.branches.length; i++) parts.push(canonicalJson(n.branches[i]));
        return '{"oneOf":[' + parts.join(',') + '],"discriminator":{"propertyName":' +
          q(n.tag) + '}}';
    }
    throw new Error('unreachable');
  }

  /** `{"type":"object","properties":{...},"required":[...]` without the closing brace. */
  function canonicalObject(fields) {
    var parts = [];
    var req = [];
    for (var i = 0; i < fields.length; i++) {
      parts.push(q(fields[i].name) + ':' + canonicalJson(fields[i].schema));
      if (!fields[i].optional) req.push(q(fields[i].name));
    }
    return '{"type":"object","properties":{' + parts.join(',') + '},"required":[' + req.join(',') + ']';
  }

  function utf8(s) {
    var out = [];
    for (var i = 0; i < s.length; i++) {
      var c = s.charCodeAt(i);
      if (c >= 0xd800 && c <= 0xdbff) {
        c = 0x10000 + ((c - 0xd800) << 10) + (s.charCodeAt(++i) - 0xdc00);
      }
      if (c < 0x80) out.push(c);
      else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 63));
      else if (c < 0x10000) out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
      else out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
    }
    return out;
  }

  var K = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
  ];

  /** SHA-256 of a byte array → array of 32 bytes. */
  function sha256(bytes) {
    var h = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
      0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    var len = bytes.length;
    var msg = bytes.slice();
    msg.push(0x80);
    while (msg.length % 64 !== 56) msg.push(0);
    var bitLenHi = Math.floor(len / 0x20000000);
    var bitLenLo = (len * 8) >>> 0;
    msg.push((bitLenHi >>> 24) & 255, (bitLenHi >>> 16) & 255, (bitLenHi >>> 8) & 255, bitLenHi & 255,
      (bitLenLo >>> 24) & 255, (bitLenLo >>> 16) & 255, (bitLenLo >>> 8) & 255, bitLenLo & 255);
    var w = new Array(64);
    for (var off = 0; off < msg.length; off += 64) {
      var t;
      for (t = 0; t < 16; t++) {
        w[t] = (msg[off + t * 4] << 24) | (msg[off + t * 4 + 1] << 16) |
          (msg[off + t * 4 + 2] << 8) | msg[off + t * 4 + 3];
      }
      for (t = 16; t < 64; t++) {
        var x = w[t - 15];
        var y = w[t - 2];
        var s0 = ((x >>> 7) | (x << 25)) ^ ((x >>> 18) | (x << 14)) ^ (x >>> 3);
        var s1 = ((y >>> 17) | (y << 15)) ^ ((y >>> 19) | (y << 13)) ^ (y >>> 10);
        w[t] = (w[t - 16] + s0 + w[t - 7] + s1) | 0;
      }
      var a = h[0], b = h[1], c = h[2], d = h[3], e = h[4], f = h[5], g = h[6], hh = h[7];
      for (t = 0; t < 64; t++) {
        var S1 = ((e >>> 6) | (e << 26)) ^ ((e >>> 11) | (e << 21)) ^ ((e >>> 25) | (e << 7));
        var ch = (e & f) ^ (~e & g);
        var t1 = (hh + S1 + ch + K[t] + w[t]) | 0;
        var S0 = ((a >>> 2) | (a << 30)) ^ ((a >>> 13) | (a << 19)) ^ ((a >>> 22) | (a << 10));
        var maj = (a & b) ^ (a & c) ^ (b & c);
        var t2 = (S0 + maj) | 0;
        hh = g; g = f; f = e; e = (d + t1) | 0;
        d = c; c = b; b = a; a = (t1 + t2) | 0;
      }
      h[0] = (h[0] + a) | 0; h[1] = (h[1] + b) | 0; h[2] = (h[2] + c) | 0; h[3] = (h[3] + d) | 0;
      h[4] = (h[4] + e) | 0; h[5] = (h[5] + f) | 0; h[6] = (h[6] + g) | 0; h[7] = (h[7] + hh) | 0;
    }
    var out = [];
    for (var i = 0; i < 8; i++) {
      out.push((h[i] >>> 24) & 255, (h[i] >>> 16) & 255, (h[i] >>> 8) & 255, h[i] & 255);
    }
    return out;
  }

  // ---- descriptor emission ------------------------------------------------

  function varint(out, v) {
    while (v >= 128) {
      var g = v % 128;
      out.push(g | 0x80);
      v = (v - g) / 128;
    }
    out.push(v);
  }

  function append(out, bytes) {
    for (var i = 0; i < bytes.length; i++) out.push(bytes[i]);
  }

  function emit(ctx, n, optional, depth, path, st) {
    if (depth > MAX_DEPTH) {
      ctx.err(path, 'nesting is deeper than ' + MAX_DEPTH + ' levels');
      return [];
    }
    st.count++;
    var flag = optional ? OPTIONAL : 0;
    var out = [];
    var body, i;
    switch (n.kind) {
      case 'boolean':
        out.push(OP.BOOL | flag);
        break;
      case 'integer':
        out.push(INTEGER_FORMATS[n.format] | flag);
        break;
      case 'number':
        out.push(NUMBER_FORMATS[n.format] | flag);
        break;
      case 'string':
        out.push(OP.STRING | flag);
        break;
      case 'bytes':
        out.push(OP.BYTES | flag);
        break;
      case 'enum':
        out.push(OP.ENUM | flag, n.values.length - 1);
        for (i = 0; i < n.values.length; i++) st.names.push(n.values[i]);
        break;
      case 'object':
        out.push(OP.OBJECT | flag);
        varint(out, n.fields.length);
        for (i = 0; i < n.fields.length; i++) st.names.push(n.fields[i].name);
        body = [];
        for (i = 0; i < n.fields.length; i++) {
          append(body, emit(ctx, n.fields[i].schema, n.fields[i].optional, depth + 1,
            ptr(path, n.fields[i].name), st));
        }
        varint(out, body.length);
        append(out, body);
        break;
      case 'variants':
        out.push(OP.VARIANTS | flag);
        varint(out, n.fields.length);
        out.push(n.variants.length - 1);
        for (i = 0; i < n.fields.length; i++) st.names.push(n.fields[i].name);
        for (i = 0; i < n.variants.length; i++) st.names.push(n.variants[i].name);
        body = [];
        for (i = 0; i < n.fields.length; i++) {
          append(body, emit(ctx, n.fields[i].schema, n.fields[i].optional, depth + 1,
            ptr(path, n.fields[i].name), st));
        }
        for (i = 0; i < n.variants.length; i++) {
          append(body, emit(ctx, n.variants[i].schema, false, depth + 1,
            ptr(path, n.variants[i].name), st));
        }
        varint(out, body.length);
        append(out, body);
        break;
      case 'array':
        out.push(OP.ARRAY | flag);
        body = emit(ctx, n.items, false, depth + 1, ptr(path, 'items'), st);
        varint(out, body.length);
        append(out, body);
        break;
      case 'union':
        out.push(OP.UNION | flag, n.branches.length - 1);
        st.names.push(n.tag);
        for (i = 0; i < n.branches.length; i++) st.names.push(tagValue(n.branches[i], n.tag));
        body = [];
        for (i = 0; i < n.branches.length; i++) {
          append(body, emit(ctx, branchBody(n, i), false, depth + 1, ptr(path, i), st));
        }
        varint(out, body.length);
        append(out, body);
        break;
    }
    return out;
  }

  /**
   * Validates and canonicalises a schema. Returns { tree, json, hash } where
   * json is the canonical text (SPEC §5) and hash its 8-byte SHA-256 prefix
   * as 16 hex chars. Throws CompileError listing every problem found.
   */
  function canonicalize(schema) {
    var ctx = new Ctx(schema);
    var tree = canon(ctx, schema, '', true);
    if (ctx.errors.length || !tree) {
      if (!ctx.errors.length) ctx.err('', 'schema rejected');
      throw new CompileError(ctx.errors);
    }
    var json = canonicalJson(tree);
    var digest = sha256(utf8(json));
    var hex = '';
    for (var i = 0; i < 8; i++) hex += (digest[i] < 16 ? '0' : '') + digest[i].toString(16);
    return { tree: tree, json: json, hash: hex, hashBytes: digest.slice(0, 8) };
  }

  /** Compiles a schema (a parsed JSON value) into descriptor bytes. */
  function compile(schema) {
    var c = canonicalize(schema);
    var ctx = new Ctx(schema);
    var st = { count: 0, names: [] };
    var nodes = emit(ctx, c.tree, false, 1, '', st);
    if (ctx.errors.length) throw new CompileError(ctx.errors);
    var out = [FORMAT_VERSION];
    append(out, c.hashBytes);
    varint(out, st.count);
    append(out, nodes);
    for (var i = 0; i < st.names.length; i++) {
      var b = utf8(st.names[i]);
      varint(out, b.length);
      append(out, b);
    }
    return new Uint8Array(out);
  }

  return {
    FORMAT_VERSION: FORMAT_VERSION,
    CompileError: CompileError,
    compile: compile,
    canonicalize: canonicalize,
    sha256: sha256
  };
}));
