// engine/locate.js — positions for diagnostics: UTF-8 byte offsets (the
// compiler's spans) → character offsets, character offsets → 1-based
// line/col, and where a JSON pointer points inside a JSON file (schema and
// samples diagnostics), so every diagnostic can carry a span in its file.

/** The UTF-16 index of UTF-8 byte offset `byte` in `text` (clamped). */
export function charIndex(text, byte) {
  let b = 0;
  for (let i = 0; i < text.length; i++) {
    if (b >= byte) return i;
    const c = text.codePointAt(i);
    b += c < 0x80 ? 1 : c < 0x800 ? 2 : c < 0x10000 ? 3 : 4;
    if (c >= 0x10000) i++;
  }
  return text.length;
}

/** 1-based { line, col, offset } of character offset `o` (col in UTF-16 units, as editors count). */
export function position(text, o) {
  const off = Math.max(0, Math.min(o, text.length));
  let line = 1; let last = -1;
  for (let i = 0; i < off; i++) if (text.charCodeAt(i) === 10) { line++; last = i; }
  return { line, col: off - last, offset: off };
}

/** A span over characters [start, end) of text: { line, col, offset, end: { line, col, offset } }. */
export function spanAt(text, start, end = start) {
  const s = position(text, start);
  const e = position(text, Math.max(start, end));
  return { ...s, end: e };
}

/** A span from UTF-8 byte offsets (the compiler's and the VM's). */
export const spanOfBytes = (text, start, end) => spanAt(text, charIndex(text, start), charIndex(text, end));

/**
 * Where the value at JSON pointer `ptr` ('' the root, '/oneOf/0/properties')
 * starts and ends in JSON `text`: { start, end } (character offsets) for the
 * deepest part of the pointer that exists; null if the text isn't JSON.
 */
export function locatePointer(text, ptr) {
  const want = ptr === '' ? [] : ptr.replace(/^\//, '').split('/').map((t) => t.replace(/~1/g, '/').replace(/~0/g, '~'));
  let i = 0;
  const ws = () => { while (i < text.length && ' \t\r\n'.includes(text[i])) i++; };
  const str = () => {
    const s = i; i++;
    while (i < text.length && text[i] !== '"') i += text[i] === '\\' ? 2 : 1;
    i++;
    try { return JSON.parse(text.slice(s, i)); } catch { throw new Error('bad string'); }
  };
  // Skips one value. `rest`: the pointer's tokens still to follow inside it
  // (null: not on the path). Returns the span of the deepest match.
  function value(rest) {
    ws();
    const start = i;
    let found = null;
    const into = (key) => (rest && rest.length && key === rest[0] ? rest.slice(1) : null);
    const c = text[i];
    if (c === '{') {
      i++; ws();
      if (text[i] === '}') i++;
      else {
        for (;;) {
          ws(); const k = str(); ws();
          if (text[i] !== ':') throw new Error('expected :');
          i++;
          const sub = into(k);
          const r = value(sub);
          if (sub) found = r;
          ws();
          if (text[i] === ',') { i++; continue; }
          if (text[i] === '}') { i++; break; }
          throw new Error('expected , or }');
        }
      }
    } else if (c === '[') {
      i++; ws();
      if (text[i] === ']') i++;
      else {
        for (let n = 0; ; n++) {
          const sub = into(String(n));
          const r = value(sub);
          if (sub) found = r;
          ws();
          if (text[i] === ',') { i++; continue; }
          if (text[i] === ']') { i++; break; }
          throw new Error('expected , or ]');
        }
      }
    } else if (c === '"') {
      str();
    } else {
      const m = /^(-?\d+(\.\d+)?([eE][+-]?\d+)?|true|false|null)/.exec(text.slice(i, i + 64));
      if (!m) throw new Error('bad value');
      i += m[0].length;
    }
    if (!rest) return null;
    return found ?? { start, end: i };
  }
  try { return value(want); } catch { return null; }
}

/** A JSON.parse error's character offset (V8: "at position N"), or 0. */
export function jsonErrorOffset(err, text) {
  const m = /position (\d+)/.exec(err?.message ?? '');
  if (m) return Number(m[1]);
  const lc = /line (\d+) column (\d+)/.exec(err?.message ?? '');
  if (lc) {
    let o = 0;
    for (let l = 1; l < Number(lc[1]); l++) { const nl = text.indexOf('\n', o); if (nl < 0) break; o = nl + 1; }
    return o + Number(lc[2]) - 1;
  }
  return 0;
}
