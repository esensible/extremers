// json.js — JSON as a person reads it: 2-space indent, one member or element
// per line, `"key": value`, empty containers as {} and [], a trailing newline
// (the layout of JSON.stringify(v, null, 2) + '\n'). Used by api.files.write
// (every .json write), the editor (a .json file on open and on save), the
// templates and project.import, so JSON is pretty wherever a person sees it.
//
// Lossless: only the whitespace between tokens changes. Strings and numbers
// are kept as written (JSON.parse → JSON.stringify would round an int64 past
// 2^53, turn 1.0 into 1 and unescape \u sequences); the key order is kept.
// Text that does not parse is never touched: a half-edited file is not lost.
//
// No DOM: tested in Node (test/json.test.mjs).

export const isJsonPath = (path) => /\.json$/i.test(String(path ?? ''));

const TOKEN = /\s*(?:("(?:[^"\\]|\\.)*")|(-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?)|(true|false|null)|([{}[\]:,]))/y;

/**
 * JSON text → its pretty form. → { ok: true, text, changed } when it parses,
 * { ok: false, error } (the parser's message) when it does not.
 */
export function prettyJson(text) {
  const src = String(text ?? '');
  try { JSON.parse(src); } catch (e) { return { ok: false, error: e.message }; }
  const toks = [];
  TOKEN.lastIndex = 0;
  while (TOKEN.lastIndex < src.length) {
    const at = TOKEN.lastIndex;
    const m = TOKEN.exec(src);
    if (!m) {
      if (/^\s*$/.test(src.slice(at))) break; // trailing whitespace
      return { ok: false, error: `unexpected text at ${at}` }; // cannot happen after JSON.parse
    }
    toks.push(m[1] ?? m[2] ?? m[3] ?? m[4]);
  }
  const pad = (d) => '\n' + '  '.repeat(d);
  let out = '';
  let depth = 0;
  for (let i = 0; i < toks.length; i++) {
    const t = toks[i];
    if (t === '{' || t === '[') {
      const close = t === '{' ? '}' : ']';
      if (toks[i + 1] === close) { out += t + close; i++; } else out += t + pad(++depth);
    } else if (t === '}' || t === ']') out += pad(--depth) + t;
    else if (t === ',') out += ',' + pad(depth);
    else if (t === ':') out += ': ';
    else out += t;
  }
  out += '\n';
  return { ok: true, text: out, changed: out !== src };
}

/** A file's text as stored: pretty when the path is .json and the text parses; else as given. */
export function storedText(path, text) {
  if (!isJsonPath(path)) return text;
  const p = prettyJson(text);
  return p.ok ? p.text : text;
}
