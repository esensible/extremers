// editor-monarch.mjs — a small interpreter for the Monarch features the
// studio's grammars use (and Monaco's typescript/javascript grammar uses), so
// the tests can tokenize real files in Node, where Monaco can't run: rules
// [regex, action] and [regex, token, next], @attribute substitution in
// regexes, `cases` (@array, @default, guard regexes), group actions, next
// (@pop, @state), switchTo, include, @brackets, defaultToken and
// tokenPostfix. Line by line, as Monaco does; the state stack carries over.
// Not Monaco: a disagreement with it is this file's bug, not the grammar's.

function compileRegex(lang, re) {
  const src = re.source.replace(/@(\w+)/g, (m, name) => {
    const v = lang[name];
    if (v instanceof RegExp) return v.source;
    if (typeof v === 'string') return v;
    return m;
  });
  return new RegExp(`^(?:${src})`, re.flags.replace('g', ''));
}

function resolveCases(lang, cases, text) {
  for (const [key, val] of Object.entries(cases)) {
    if (key === '@default' || key === '@') continue;
    if (key.startsWith('@')) { if ((lang[key.slice(1)] ?? []).includes(text)) return val; continue; }
    if (new RegExp(`^(?:${key})$`).test(text)) return val;
  }
  return cases['@default'];
}

function tokenOf(lang, action, text) {
  if (typeof action === 'string') return action;
  if (action.cases) return tokenOf(lang, resolveCases(lang, action.cases, text), text);
  return action.token ?? '';
}
function nextOf(lang, action, text) {
  if (!action || typeof action === 'string') return null;
  if (action.cases) return nextOf(lang, resolveCases(lang, action.cases, text), text);
  if (action.next) return { next: action.next };
  if (action.switchTo) return { switchTo: action.switchTo };
  return null;
}

function rulesOf(lang, state, seen = new Set()) {
  if (seen.has(state)) return [];
  seen.add(state);
  const out = [];
  for (const r of lang.tokenizer[state] ?? (() => { throw new Error(`no state ${state}`); })()) {
    if (!Array.isArray(r)) { out.push(...rulesOf(lang, r.include.replace(/^@/, ''), seen)); continue; }
    const [re, action, next] = r;
    out.push({ re: compileRegex(lang, re), action, next });
  }
  return out;
}

const post = (lang, t) => (t && t !== '@brackets' ? t + (lang.tokenPostfix ?? '') : t === '@brackets' ? `delimiter.bracket${lang.tokenPostfix ?? ''}` : '');

/** text → [{ line, text, token, state }] (empty-token whitespace left out). */
export function tokenize(lang, text) {
  const cache = new Map();
  const rules = (s) => { if (!cache.has(s)) cache.set(s, rulesOf(lang, s)); return cache.get(s); };
  const stack = ['root'];
  const out = [];
  const apply = (n) => {
    if (!n) return;
    if (n.switchTo) { stack[stack.length - 1] = n.switchTo.replace(/^@/, ''); return; }
    if (n.next === '@pop') { if (stack.length > 1) stack.pop(); return; }
    stack.push(n.next.replace(/^@/, ''));
  };
  text.split('\n').forEach((line, i) => {
    let pos = 0;
    while (pos < line.length) {
      const rest = line.slice(pos);
      const state = stack[stack.length - 1];
      let hit = null;
      for (const r of rules(state)) { const m = r.re.exec(rest); if (m && m[0].length) { hit = { r, m }; break; } }
      if (!hit) {
        out.push({ line: i + 1, text: rest[0], token: post(lang, lang.defaultToken ?? ''), state });
        pos += 1;
        continue;
      }
      const { r, m } = hit;
      if (Array.isArray(r.action)) {
        let next = null;
        r.action.forEach((a, g) => {
          const t = m[g + 1] ?? '';
          if (t.length) out.push({ line: i + 1, text: t, token: post(lang, tokenOf(lang, a, t)), state });
          next = nextOf(lang, a, t) ?? next;
        });
        apply(next);
      } else {
        if (m[0].length) out.push({ line: i + 1, text: m[0], token: post(lang, tokenOf(lang, r.action, m[0])), state });
        apply(r.next ? { next: r.next } : nextOf(lang, r.action, m[0]));
      }
      pos += m[0].length;
    }
    // at the end of the line, a state's `$` rule (an unterminated string) runs once
    const eol = rules(stack[stack.length - 1]).find((x) => x.re.source === '^(?:$)');
    if (eol) apply(eol.next ? { next: eol.next } : nextOf(lang, eol.action, ''));
  });
  return out.filter((t) => t.token !== '' || t.text.trim() !== '');
}
