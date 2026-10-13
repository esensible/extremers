// ui.js — the page's logic that needs no DOM, so it can be tested in Node:
// escaping, the file tree, editor ids, line/column ↔ offset (for the editor's
// cursor line and, later, diagnostics spans), sizes and times, and the text
// of the status line. app.js wires these to the page.
//
// Origin: the pattern of dingo-cli/web/ui.js (pure helpers, tested in Node);
// esc() is copied verbatim.

export function esc(s) { return String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]); }

/** The editor element's id for a project path: `editor-<path>` (the path verbatim; use getElementById). */
export const editorId = (path) => `editor-${path}`;

/** Files [{ path, ... }] → rows for the file list: folders first at each level, then files, each with its depth. */
export function fileRows(files) {
  const root = { dirs: new Map(), files: [] };
  for (const f of files) {
    const parts = f.path.split('/');
    let node = root;
    for (let i = 0; i < parts.length - 1; i++) {
      if (!node.dirs.has(parts[i])) node.dirs.set(parts[i], { dirs: new Map(), files: [] });
      node = node.dirs.get(parts[i]);
    }
    node.files.push({ ...f, name: parts[parts.length - 1] });
  }
  const rows = [];
  (function walk(node, depth, prefix) {
    for (const name of [...node.dirs.keys()].sort()) {
      rows.push({ kind: 'dir', name, path: prefix + name + '/', depth });
      walk(node.dirs.get(name), depth + 1, prefix + name + '/');
    }
    for (const f of node.files.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))) rows.push({ kind: 'file', ...f, depth });
  })(root, 0, '');
  return rows;
}

/** 1-based { line, col } of a character offset in text (col counts UTF-16 units, as the textarea does). */
export function lineCol(text, offset) {
  const o = Math.max(0, Math.min(offset, text.length));
  let line = 1; let last = -1;
  for (let i = 0; i < o; i++) if (text.charCodeAt(i) === 10) { line++; last = i; }
  return { line, col: o - last };
}

/** The character offset of 1-based line/col in text (clamped to the line and the text). */
export function offsetAt(text, line, col) {
  let o = 0;
  for (let l = 1; l < line; l++) {
    const nl = text.indexOf('\n', o);
    if (nl < 0) return text.length;
    o = nl + 1;
  }
  const end = text.indexOf('\n', o);
  const lineEnd = end < 0 ? text.length : end;
  return Math.min(o + Math.max(0, col - 1), lineEnd);
}

export function fmtBytes(n) {
  if (!Number.isFinite(n)) return '—';
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(2)} MB`;
}

export function fmtAgo(ms) {
  if (!Number.isFinite(ms) || ms < 0) return '';
  if (ms < 5000) return 'just now';
  if (ms < 60000) return `${Math.round(ms / 1000)} s ago`;
  if (ms < 3600000) return `${Math.round(ms / 60000)} min ago`;
  return `${Math.round(ms / 3600000)} h ago`;
}

/** What the editor shows for a file kind (the mode label in the cursor line). */
export function languageOf(path) {
  if (/\.expr$/.test(path)) return 'expr';
  if (/\.jsx$/.test(path)) return 'JSX';
  if (/\.m?js$/.test(path)) return 'JavaScript';
  if (/\.json$/.test(path)) return 'JSON';
  if (/\.css$/.test(path)) return 'CSS';
  if (/\.svg$/.test(path)) return 'SVG';
  if (/\.md$/.test(path)) return 'Markdown';
  return 'text';
}

// ------------------------------------------------- diagnostics and the sim ----

/** A diagnostic's place: 'file:line:col' (or the file alone). */
export const diagWhere = (d) => (d.span ? `${d.path}:${d.span.line}:${d.span.col}` : d.path ?? '');

/** A diagnostic as one line: "rule · file:line:col · message · help". */
export const diagText = (d) => [d.rule ?? '—', diagWhere(d), d.message, d.help ? `help: ${d.help}` : null].filter(Boolean).join(' · ');

/** data-span of a diagnostic row: 'line:col-endLine:endCol' ('' without a span). */
export const diagSpanAttr = (d) => (d.span ? `${d.span.line}:${d.span.col}-${d.span.end?.line ?? d.span.line}:${d.span.end?.col ?? d.span.col}` : '');

/** The editor's span of a diagnostic: { startLine, startCol, endLine, endCol } (1:1 without one). */
export function diagRange(d) {
  if (!d.span) return { startLine: 1, startCol: 1, endLine: 1, endCol: 1 };
  const end = d.span.end ?? d.span;
  // an empty span still marks one character
  const same = end.line === d.span.line && end.col <= d.span.col;
  return { startLine: d.span.line, startCol: d.span.col, endLine: end.line, endCol: same ? d.span.col + 1 : end.col };
}

/** Diagnostics → Map(path → editor markers [{ startLine, startCol, endLine, endCol, message, severity, rule }]). */
export function markersByPath(diagnostics) {
  const out = new Map();
  for (const d of diagnostics ?? []) {
    if (!d.path) continue;
    const m = { ...diagRange(d), message: d.help ? `${d.message}\nhelp: ${d.help}` : d.message, severity: d.severity === 'warning' ? 'warning' : d.severity === 'info' ? 'info' : 'error', rule: d.rule ?? '' };
    if (!out.has(d.path)) out.set(d.path, []);
    out.get(d.path).push(m);
  }
  return out;
}

/**
 * The author's events of an engine interface (engine.interface().interface),
 * for the sim's composer: [{ name, fields: [{ name, kind: 'integer'|'number'|'boolean'|'string'|'enum'|'object', values?, required, format? }] }].
 * The built-in gps and wake are left out (the sim's GPS inputs and advance do those).
 */
export function eventVariants(iface) {
  const ev = iface?.inputs?.find((i) => i.name === 'event');
  const out = [];
  for (const alt of ev?.schema?.oneOf ?? []) {
    const name = Object.keys(alt.properties ?? {})[0];
    if (!name || name === 'gps' || name === 'wake') continue;
    const s = alt.properties[name] ?? {};
    const req = new Set(s.required ?? []);
    const fields = Object.entries(s.properties ?? {}).map(([f, fs]) => ({
      name: f,
      kind: fs.enum || fs.const !== undefined ? 'enum' : fs.type ?? 'object',
      ...(fs.enum ? { values: fs.enum } : fs.const !== undefined ? { values: [fs.const] } : {}),
      ...(fs.format ? { format: fs.format } : {}),
      required: req.has(f),
    }));
    out.push({ name, fields });
  }
  return out;
}

/**
 * The composer's inputs → the event: { [name]: { field: value } }. Empty
 * optional fields are left out; an empty int64 field is the device time `now`
 * (the taps' `…_at`). Throws with a message naming the field.
 */
export function composeEvent(variant, raw, now) {
  const v = {};
  for (const f of variant.fields) {
    const s = String(raw[f.name] ?? '').trim();
    if (s === '') {
      if (f.kind === 'integer' && f.format === 'int64' && now !== undefined) { v[f.name] = now; continue; }
      if (f.required) throw new Error(`${variant.name}.${f.name} is required`);
      continue;
    }
    if (f.kind === 'integer') { if (!/^[+-]?\d+$/.test(s)) throw new Error(`${variant.name}.${f.name}: an integer`); v[f.name] = Number(s); } else if (f.kind === 'number') { const n = Number(s); if (!Number.isFinite(n)) throw new Error(`${variant.name}.${f.name}: a number`); v[f.name] = n; } else if (f.kind === 'boolean') v[f.name] = s === 'true'; else if (f.kind === 'object') { try { v[f.name] = JSON.parse(s); } catch { throw new Error(`${variant.name}.${f.name}: JSON`); } } else v[f.name] = s;
  }
  return { [variant.name]: v };
}

/** A device time as UTC 'HH:MM:SS.mmm' (uptime before the clock is set: '+s.mmm s'). */
export function fmtDeviceTime(t) {
  if (!Number.isFinite(t)) return '—';
  if (t < 1e11) return `+${(t / 1000).toFixed(3)} s`;
  return new Date(t).toISOString().slice(11, 23);
}

/** An event as a short label: 'line_stbd', 'gps fix+velocity', 'bump_seq {seconds: 300, …}'. */
export function eventLabel(ev) {
  if (!ev || typeof ev !== 'object') return String(ev);
  const [k] = Object.keys(ev);
  const v = ev[k] ?? {};
  if (k === 'gps') return `gps ${[v.fix ? 'fix' : null, v.velocity ? 'velocity' : null].filter(Boolean).join('+') || '(empty)'}`;
  const body = Object.entries(v).map(([a, b]) => `${a}: ${JSON.stringify(b)}`).join(', ');
  return body ? `${k} {${body}}` : k;
}

/** One status line for an api result: what a person (and an agent reading #status) sees. */
export function statusLine(name, r) {
  if (!r) return '';
  if (r.ok) {
    switch (name) {
      case 'project.create': return `Created ${r.name} from the ${r.template} template (${r.files.length} files)`;
      case 'project.open': return `Opened ${r.name} (${r.files.length} files)`;
      case 'project.delete': return `Deleted ${r.deleted}`;
      case 'project.rename': return `Renamed ${r.from} to ${r.name}`;
      case 'project.export': return `Exported ${r.name}: ${r.fileName}, ${fmtBytes(r.bytes)}`;
      case 'project.import': return `Imported ${r.name} (${r.files.length} files)`;
      case 'files.write': return `Saved ${r.path}`;
      case 'files.delete': return `Deleted ${r.path}`;
      case 'files.rename': return `Renamed ${r.from} to ${r.path}`;
      case 'ui.build': return `Built ${Object.entries(r.targets).map(([t, x]) => `${t} (${fmtBytes(x.files.reduce((n, f) => n + f.size, 0))})`).join(' and ')} in ${r.ms} ms`;
      case 'engine.check': return `Engine checked: compiles (${fmtBytes(r.compile.programBytes)}), ${r.runs.length} runs complete`;
      case 'engine.compile': return `Engine compiles: ${fmtBytes(r.programBytes)}, interface ${r.interfaceHash}`;
      case 'sim.start': return `Sim started (${r.clock} clock) at ${fmtDeviceTime(r.now)}${r.uiSource === 'placeholder' ? ' — placeholder page until api.ui.build' : ''}`;
      case 'sim.event': case 'sim.gps': case 'sim.advance': {
        const ev = r.evaluations ?? [];
        const bad = ev.filter((e) => !e.accepted);
        return `Sim ${fmtDeviceTime(r.now)}: ${ev.length} evaluation${ev.length === 1 ? '' : 's'}${bad.length ? `, ${bad.length} failed (${bad[0].error?.code ?? 'refused'})` : ''}${r.accepted === false ? ` — refused: ${r.refused}` : ''}${r.wake ? `; wake at ${fmtDeviceTime(r.wake)}` : ''}`;
      }
      case 'sim.replay': return `Replayed ${r.fixes}${r.done ? '' : ` of ${r.total}`} fixes from ${r.path}: ${r.evaluations} evaluations, now ${fmtDeviceTime(r.now)}`;
      case 'sim.reset': return 'Sim reset';
      case 'bundle.pack': return `Packed ${r.name} v${r.version}: ${fmtBytes(r.sizes.bundle)}, id ${r.id}${r.uiSource === 'placeholder' ? ' (placeholder page)' : ''}`;
      default: return `${name}: ok`;
    }
  }
  return `${name} failed: ${r.error}${r.hint ? ` — ${r.hint}` : ''}`;
}

/**
 * The #build-log panel for an api.ui.build() result: per target its status,
 * files with sizes, checks and time; then the diagnostics. Stable hooks:
 * #build-<target> (data-ok), rows [data-check=<name>] (data-ok), [data-rule].
 */
export function buildLogHtml(r) {
  if (!r) return '';
  if (!r.targets) return `<p class="build-diag error">${esc(r.error ?? 'the build did not run')}${r.hint ? ` — ${esc(r.hint)}` : ''}</p>`;
  const parts = [];
  for (const [t, x] of Object.entries(r.targets)) {
    const total = x.files.reduce((n, f) => n + f.size, 0);
    parts.push(`<section class="build-target" id="build-${esc(t)}" data-ok="${x.ok}">
<h3><span class="${x.ok ? 'ok' : 'bad'}">${x.ok ? '✓' : '✗'}</span>${esc(t === 'kindle' ? 'Kindle' : 'watch')}<span class="hint">${x.ok ? `${x.files.length} files, ${fmtBytes(total)}` : 'failed'} · ${x.ms} ms${x.stats ? ` · ${x.stats.modules} modules (${x.stats.transformed} compiled, ${x.stats.cached} cached)` : ''}</span></h3>
${x.files.length ? `<table>${x.files.map((f) => `<tr><td>${esc(t === 'kindle' ? `web/${f.name}` : f.name)}</td><td>${fmtBytes(f.size)}</td><td class="hint" title="sha256 ${esc(f.sha256)}">${esc(f.sha256.slice(0, 12))}…</td></tr>`).join('')}</table>` : ''}
${x.checks.length ? `<ul>${x.checks.map((c) => `<li data-check="${esc(c.name)}" data-ok="${c.ok}"><span class="${c.ok ? 'ok' : 'bad'}">${c.ok ? '✓' : '✗'}</span> ${esc(c.name)}: ${esc(c.message)}</li>`).join('')}</ul>` : ''}
</section>`);
  }
  const diags = r.diagnostics ?? [];
  if (diags.length) {
    parts.push(diags.map((d) => `<div class="build-diag ${d.severity === 'warning' ? 'warning' : 'error'}" data-rule="${esc(d.rule)}">${esc(d.target)} ${esc(d.rule)} ${esc(d.file ?? '')}${d.line ? `:${d.line}:${d.col ?? 1}` : ''} ${esc(d.message)}</div>`).join(''));
  }
  parts.push(`<p class="hint">${esc(r.name)} · ${esc(r.publicPath ?? '')} · ${r.ms} ms · ${esc(r.toolchain ?? '')}</p>`);
  return parts.join('\n');
}
