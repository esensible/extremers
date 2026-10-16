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

/**
 * The files nav: a project's files grouped by the part of the app they make,
 * each with the name a person reads (the path stays the row's title):
 *   [{ key: 'engine'|'kindle'|'watch'|'other'|'settings', label, files: [{ ...file, label }] }]
 * Groups with no files are left out, except the engine, the Kindle screen and
 * the watch screen, which are always there (a person adds files to them).
 * Engine: engine.expr, the schemas by name, samples.json as 'Test runs', then
 * the rest; a screen: its src/index.jsx first, then by name; studio.json is
 * 'Project settings'.
 */
export const FILE_GROUPS = [
  { key: 'engine', label: 'Engine' },
  { key: 'kindle', label: 'Kindle screen' },
  { key: 'watch', label: 'Watch screen' },
  { key: 'other', label: 'Other files' },
  { key: 'settings', label: 'Project settings' },
];
const ENGINE_NAMES = {
  'engine/engine.expr': ['engine.expr', 0],
  'engine/state.schema.json': ['State schema', 1],
  'engine/view.schema.json': ['View schema', 2],
  'engine/event.schema.json': ['Event schema', 3],
  'engine/samples.json': ['Test runs', 4],
};

/** One file's group and the name the nav shows: { group, label, rank }. */
export function fileLabel(path) {
  if (path === 'studio.json') return { group: 'settings', label: 'studio.json', rank: 0 };
  if (ENGINE_NAMES[path]) return { group: 'engine', label: ENGINE_NAMES[path][0], rank: ENGINE_NAMES[path][1] };
  if (path.startsWith('engine/')) return { group: 'engine', label: path.slice(7), rank: 10 };
  for (const g of ['kindle', 'watch']) {
    if (path === `${g}/src/index.jsx` || path === `${g}/src/index.js`) return { group: g, label: path.slice(g.length + 5), rank: 0 };
    if (path.startsWith(`${g}/src/`)) return { group: g, label: path.slice(g.length + 5), rank: 10 };
    if (path.startsWith(`${g}/`)) return { group: g, label: path.slice(g.length + 1), rank: 20 };
  }
  return { group: 'other', label: path, rank: 10 };
}

export function fileGroups(files) {
  const by = new Map(FILE_GROUPS.map((g) => [g.key, []]));
  for (const f of files) { const l = fileLabel(f.path); by.get(l.group).push({ ...f, label: l.label, rank: l.rank }); }
  const always = new Set(['engine', 'kindle', 'watch']);
  return FILE_GROUPS
    .map((g) => ({ ...g, files: by.get(g.key).sort((a, b) => a.rank - b.rank || (a.label < b.label ? -1 : a.label > b.label ? 1 : 0)) }))
    .filter((g) => g.files.length || always.has(g.key));
}

/** A tab's label: the file's nav name, prefixed by its screen ('Kindle · index.jsx'). */
export function tabLabel(path) {
  const l = fileLabel(path);
  if (l.group === 'kindle') return `Kindle · ${l.label}`;
  if (l.group === 'watch') return `Watch · ${l.label}`;
  return l.label;
}

/**
 * The project's GPS tracks, for "Replay a track…" (api.sim.replay): `.nmea`
 * files anywhere, and `.json`/`.jsonl` files outside the engine and the
 * screens (those are schemas, samples and code) and other than studio.json.
 */
export function trackFiles(files) {
  return files.map((f) => f.path).filter((p) => /\.nmea$/i.test(p) || (/\.jsonl?$/i.test(p) && p !== 'studio.json' && !/^(engine|kindle|watch)\//.test(p))).sort();
}

/** Which screens a project has (an entry file in kindle/src or watch/src): ['kindle', 'watch'] or fewer. */
export function screensOf(files) {
  return ['kindle', 'watch'].filter((t) => files.some((f) => f.path === `${t}/src/index.jsx` || f.path === `${t}/src/index.js`));
}

/**
 * A project.list entry's second line in the project menu: the bundle it
 * makes (studio.json's name, when it is not the project's, and version) and
 * its files: 'v1 · 23 files', 'race v3 · 23 files'.
 */
export function projectParts(p) {
  const s = p.settings && !p.settings.error ? p.settings : null;
  const bundle = s ? `${s.name && s.name !== p.name ? `${s.name} ` : ''}v${s.version ?? 1}` : null;
  const files = `${p.files ?? 0} file${p.files === 1 ? '' : 's'}`;
  return bundle ? `${bundle} · ${files}` : files;
}

/**
 * A value as one short line for the Run column: JSON with spaces, keys bare
 * when they are identifiers, long lists cut ({marks: 0, mark: {lat: -33.8}}).
 */
export function compactJson(v, maxItems = 8) {
  if (v === undefined) return '—';
  if (v === null || typeof v !== 'object') return JSON.stringify(v);
  if (Array.isArray(v)) {
    const items = v.slice(0, maxItems).map((x) => compactJson(x, maxItems));
    if (v.length > maxItems) items.push(`… ${v.length - maxItems} more`);
    return `[${items.join(', ')}]`;
  }
  return `{${Object.entries(v).map(([k, x]) => `${/^[A-Za-z_$][\w$]*$/.test(k) ? k : JSON.stringify(k)}: ${compactJson(x, maxItems)}`).join(', ')}}`;
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
      case 'device.choose': case 'device.connect': return `${r.already ? 'Already connected' : 'Connected'} to ${r.deviceName ?? 'the box'}`;
      case 'device.disconnect': return 'Disconnected from the box';
      case 'device.list': return `The box holds ${r.bundles.length} bundle${r.bundles.length === 1 ? '' : 's'}${r.bundles.find((b) => b.active) ? `; active: ${r.bundles.find((b) => b.active).name}` : ''}; ${fmtBytes(r.free)} free`;
      case 'device.install': return `Installed ${r.installed.name} v${r.installed.version} on the box: ${fmtBytes(r.installed.bytes)} in ${(r.ms / 1000).toFixed(1)} s (${r.kbps} kB/s)${r.resumedFrom ? `, resumed at ${fmtBytes(r.resumedFrom)}` : ''}${r.activated ? '; active' : ''}`;
      case 'device.activate': return `Activated ${r.name ?? r.id} on the box`;
      case 'device.delete': return `Deleted ${r.name ?? r.id} from the box`;
      case 'device.get': return `Downloaded ${r.name} v${r.version} from the box: ${fmtBytes(r.sizes.bundle)}, SHA-256 checked`;
      case 'device.firmware': return `The box runs ota_${r.slot}, sha256 ${r.sha256.slice(0, 16)}…${r.pending ? ' (on trial)' : ''}`;
      case 'device.updateFirmware': return `${r.updated ? `Firmware updated: ota_${r.to.slot}, sha256 ${r.to.sha256.slice(0, 16)}…` : `Firmware not updated: ${r.note ?? 'the box runs another firmware'}`}${r.kbps ? ` (${r.kbps} kB/s)` : ''}; ${r.restored.length} bundle${r.restored.length === 1 ? '' : 's'} back${r.notRestored?.length ? `, ${r.notRestored.length} refused (${r.notRestored.map((b) => b.name).join(', ')})` : ''}${r.activated ? `; ${r.activated} active` : ''}`;
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

// ------------------------------------------------------------ the #device panel

/**
 * A transfer's progress { phase, sent, total, kbps } as one line. A firmware
 * update's phases: save (each bundle downloaded: bundle, index, of),
 * firmware (the image), reboot (pending: on trial), restore (each bundle
 * installed again; activate: the boot one).
 */
export function deviceProgressText(p) {
  if (!p) return '';
  const pct = p.total ? Math.floor((p.sent / p.total) * 100) : 0;
  const which = p.bundle ? ` ${p.bundle}${p.of ? ` (${p.index} of ${p.of})` : ''}` : '';
  const what = {
    begin: 'starting', upload: 'uploading', reconnect: 'link dropped: reconnecting to resume', commit: 'committing (the box validates it)', activate: 'activating', download: 'downloading', done: 'done',
    save: `saving${which}`,
    firmware: 'sending the firmware',
    reboot: p.pending ? 'the new firmware is on trial (it confirms itself 20 s after it boots)' : 'the box is rebooting into the firmware',
    restore: p.activate ? `activating${which}` : `restoring${which}`,
  }[p.phase] ?? p.phase;
  const bytes = p.total && p.phase !== 'reboot' && !p.activate;
  return `${what}${bytes ? ` · ${fmtBytes(p.sent)} of ${fmtBytes(p.total)} (${pct}%)` : ''}${p.kbps && p.phase !== 'reboot' ? ` · ${p.kbps} kB/s` : ''}`;
}

/** status().firmware as the panel's firmware line: 'ota_0 · sha 12ab34cd…' (', on trial' while pending). */
export function deviceFirmwareText(fw) {
  if (!fw) return 'Firmware: not asked yet (Ask the box)';
  return `Firmware: ota_${fw.slot} · sha ${String(fw.sha256).slice(0, 8)}…${fw.pending ? ' · on trial' : ''}`;
}

/**
 * The inline confirm for a firmware image dropped on the Device panel:
 * { name, len } of the file, and how many bundles the box holds (the last
 * list's, besides Choose; null: not listed).
 */
export function deviceUpdateConfirmText({ name, len }, bundles = null) {
  const n = bundles === null ? 'every bundle on the box' : bundles === 1 ? 'the 1 bundle on the box' : `the ${bundles} bundles on the box`;
  return `Update the box's firmware with ${name} (${fmtBytes(len)})? This saves ${n} in this browser, updates the firmware, then restores them; the GPS track is lost.`;
}

/** The "restore pending" banner's text from api.device.status() ('' when there is nothing to restore). */
export function deviceRestoreText(s) {
  if (!s?.restorePending) return '';
  const n = s.restoreBundles ?? 0;
  const where = { saved: 'before the firmware was committed', firmware: 'after the firmware was committed', restore: 'while the bundles were going back' }[s.restorePhase] ?? '';
  return `A firmware update of this box stopped ${where}: ${n} bundle${n === 1 ? ' is' : 's are'} saved in this browser and not back on the box yet. Resume finishes it.`;
}

/** api.device.status() as the panel's status line. */
export function deviceStatusText(s) {
  if (!s) return '';
  if (s.ok === false) return `device: ${s.error}`;
  if (!s.bluetooth) return 'Web Bluetooth is not available in this browser (desktop Chrome on http://localhost)';
  const who = s.deviceName ?? 'the box';
  let t;
  switch (s.state) {
    case 'connected': t = `Connected to ${who}${s.bonded ? ' (bonded)' : ''}`; break;
    case 'connecting': t = `Connecting to ${who}…`; break;
    case 'busy': t = `${who}: ${s.op}${s.progress && s.op !== 'connect' && s.op !== 'choose' ? ` — ${deviceProgressText(s.progress)}` : '…'}`; break;
    default: t = s.deviceName ? `Disconnected from ${who}`
      : s.remembered ? 'Not connected: this page remembers the box; Connect connects without prompts'
        : "Not connected: this page has not chosen the box yet. Start on the Kindle (Choose → Settings → Pair watch), then click 'Choose the box…' (how, in full, below)";
  }
  if (s.chunk) t += ` · chunk ${s.chunk} B${s.window ? `, window ${s.window} B` : ''}`;
  if (s.lastError && s.state !== 'connected' && s.state !== 'busy') t += ` — ${s.lastError}${s.hint ? ` (${s.hint})` : ''}`;
  return t;
}

/**
 * The header's device group from api.device.status():
 *   text        'nacra · connected' | 'nacra · disconnected' | 'nacra · connecting…' |
 *               'nacra · busy: install 42%' | 'no box chosen' | …; with more than
 *               one box remembered (status().candidates > 1) the name carries the
 *               last 4 characters of the box's id: 'nacra ·CA5Q · disconnected'
 *   tone        'ok' | 'busy' | 'idle' | 'err' (the text's colour)
 *   choose      the chooser button (#device-choose): { label, prominent } — prominent
 *               ('Choose the box…') only when Chrome remembers no box for this page;
 *               otherwise a quiet 'Choose another box…' (a different box than
 *               before: Chrome remembers each pick separately, and connect
 *               prefers the last one chosen)
 *   connect     show Connect (a remembered box, not connected)
 *   disconnect  show Disconnect (connected or connecting)
 *   disabled    the buttons are disabled (a device call is running)
 */
export const CHOOSE_LABEL = 'Choose the box…';
export const CHOOSE_AGAIN_LABEL = 'Choose another box…';

/** A BluetoothDevice.id's last 4 characters without its '::'/'=' padding (device/ble.js idTag; the page loads the device client lazily, so not imported). */
export const deviceIdTag = (id) => (typeof id === 'string' && id ? id.replace(/[:=]+$/, '').slice(-4) : '');

export function deviceHead(s) {
  if (!s) return { text: '', tone: 'idle', choose: { label: CHOOSE_LABEL, prominent: true }, connect: false, disconnect: false, disabled: false };
  if (!s.bluetooth) return { text: 'no Web Bluetooth', tone: 'err', choose: { label: CHOOSE_LABEL, prominent: true }, connect: false, disconnect: false, disabled: true };
  const tag = s.candidates > 1 ? deviceIdTag(s.deviceId) : '';
  const who = `${s.deviceName ?? 'nacra'}${tag ? ` ·${tag}` : ''}`;
  const remembered = s.remembered === true || !!s.deviceName;
  const busy = s.state === 'busy';
  let text; let tone;
  if (busy) {
    const pct = s.progress?.total && s.op !== 'connect' && s.op !== 'choose' ? ` ${Math.floor((s.progress.sent / s.progress.total) * 100)}%` : '';
    text = `${who} · busy: ${s.op}${pct}`; tone = 'busy';
  } else if (s.state === 'connected') { text = `${who} · connected`; tone = 'ok'; }
  else if (s.state === 'connecting') { text = `${who} · connecting…`; tone = 'busy'; }
  else if (remembered) { text = `${who} · disconnected`; tone = s.lastError ? 'err' : 'idle'; }
  else { text = 'no box chosen'; tone = s.lastError ? 'err' : 'idle'; }
  return {
    text, tone,
    choose: remembered ? { label: CHOOSE_AGAIN_LABEL, prominent: false } : { label: CHOOSE_LABEL, prominent: true },
    connect: remembered && s.state === 'disconnected',
    disconnect: s.state === 'connected' || s.state === 'connecting',
    disabled: busy,
  };
}

/** The colours of the apps on the box (their swatch and their part of the storage bar), in turn. */
export const APP_COLOURS = ['var(--accent)', 'var(--info)', 'var(--muted)', 'var(--good)', 'var(--warn)'];

/** An app's flags as its pill: 'Active · starts at boot' | 'Active' | 'Starts at boot' | ''. */
export function appPill(b) {
  if (b.active && b.boot) return 'Active · starts at boot';
  if (b.active) return 'Active';
  if (b.boot) return 'Starts at boot';
  return '';
}

/**
 * The box's storage from api.device.list(): used (every bundle, Choose too),
 * total (used + free) and the apps' parts for the bar.
 * → { used, total, text, parts: [{ id, name, pct, colour }] }
 */
export function deviceStorage(list) {
  const used = list.bundles.reduce((n, b) => n + (b.len ?? 0), 0);
  const total = used + (list.free ?? 0);
  const apps = list.bundles.filter((b) => !b.builtin);
  return {
    used, total,
    text: `${fmtBytes(used)} of ${fmtBytes(total)} used · the largest app that fits: ${fmtBytes(list.largestFree)}`,
    parts: apps.map((b, i) => ({ id: b.id, name: b.name, pct: total ? Math.max(0.5, (b.len / total) * 100) : 0, colour: APP_COLOURS[i % APP_COLOURS.length] })),
  };
}

/**
 * The apps of api.device.list() as the Box panel's rows (Choose, built in,
 * is the note under them). Each row `li.app`: data-id, data-name; "Make
 * active" (#device-activate-<id>, not on the active one) and the ⋯ button
 * (#device-more-<id>) with its menu (#device-menu-<id>, hidden unless
 * opts.menu is this id): "Download the bundle" (#device-get-<id>) and
 * "Remove from the box…" (#device-delete-<id>). A row whose action waits for
 * its inline confirm (confirm: { id, act }) shows #device-confirm-yes /
 * #device-confirm-no instead. Under the rows: #device-space (the storage),
 * the Choose note (#device-activate-0 when Choose is not active), and
 * #device-stale once the box changed since the list.
 * opts.project: the open project's bundle name (its row says "this project").
 */
export function deviceRowsHtml(list, confirm = null, stale = false, { menu = null, project = null } = {}) {
  if (!list) return '<p class="hint" id="device-unlisted">Connect to see the apps on the box.</p>';
  const st = deviceStorage(list);
  const colour = new Map(st.parts.map((p) => [p.id, p.colour]));
  const apps = list.bundles.filter((b) => !b.builtin);
  const rows = apps.map((b) => {
    const asking = confirm && confirm.id === b.id;
    const name = esc(b.name);
    let acts;
    if (asking && confirm.act === 'delete') acts = `<span class="confirm" role="group" aria-label="Remove ${name}">Remove ${name} from the box? <button type="button" class="btn danger solid" id="device-confirm-yes" data-act="confirm-delete" data-id="${b.id}">Remove</button><button type="button" class="btn ghost" id="device-confirm-no" data-act="cancel">Cancel</button></span>`;
    else if (asking && confirm.act === 'activate') acts = `<span class="confirm" role="group" aria-label="Make ${name} active">The box restarts on ${name}. <button type="button" class="btn pri" id="device-confirm-yes" data-act="confirm-activate" data-id="${b.id}">Make active</button><button type="button" class="btn ghost" id="device-confirm-no" data-act="cancel">Cancel</button></span>`;
    else {
      const pill = appPill(b);
      acts = `${pill ? `<span class="pill good">${esc(pill)}</span>` : ''}${b.active ? '' : `<button type="button" class="btn ghost sm" id="device-activate-${b.id}" data-act="activate" data-id="${b.id}">Make active</button>`}`;
    }
    const open = menu === b.id;
    const more = `<button type="button" class="icon-btn" id="device-more-${b.id}" data-act="more" data-id="${b.id}" aria-haspopup="menu" aria-expanded="${open}" aria-controls="device-menu-${b.id}" aria-label="More actions for ${name}"><svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><circle cx="5" cy="12" r="1.8"></circle><circle cx="12" cy="12" r="1.8"></circle><circle cx="19" cy="12" r="1.8"></circle></svg></button>`;
    const items = `<div class="menu row-menu" role="menu" id="device-menu-${b.id}" aria-label="Actions for ${name}"${open ? '' : ' hidden'}><button type="button" role="menuitem" id="device-get-${b.id}" data-act="get" data-id="${b.id}">Download the bundle</button><button type="button" role="menuitem" class="danger" id="device-delete-${b.id}" data-act="delete" data-id="${b.id}">Remove from the box…</button></div>`;
    const sub = [fmtBytes(b.len), project && b.name === project ? 'this project' : null].filter(Boolean).join(' · ');
    return `<li class="app" data-id="${b.id}" data-name="${name}"><span class="swatch" style="background:${colour.get(b.id)}"></span><div class="app-main"><span class="app-name">${name} <span class="muted">v${b.version}</span></span><span class="app-sub" title="id ${b.id} · content ${esc(b.contentHash)} · ui.web ${esc(b.ui.web)}, ui.watch ${esc(b.ui.watch)}">${esc(sub)}</span></div><span class="app-acts">${acts}</span>${more}${items}</li>`;
  }).join('');
  const choose = list.bundles.find((b) => b.builtin);
  const chooseNote = choose
    ? `<p class="note" id="device-choose-note">Choose (the app menu) is built into the firmware and always there.${choose.active ? ' It is active.' : confirm?.id === choose.id && confirm.act === 'activate'
      ? ` <span class="confirm">The box restarts on Choose. <button type="button" class="btn pri" id="device-confirm-yes" data-act="confirm-activate" data-id="${choose.id}">Make active</button><button type="button" class="btn ghost" id="device-confirm-no" data-act="cancel">Cancel</button></span>`
      : ` <button type="button" class="link" id="device-activate-${choose.id}" data-act="activate" data-id="${choose.id}">Make Choose active</button>`}</p>`
    : '';
  return `${apps.length ? `<ul class="apps">${rows}</ul>` : '<p class="hint">No apps on the box yet: Send to box puts this project there.</p>'}
<p class="hint" id="device-space">${esc(st.text)}</p>${chooseNote}${stale ? '\n<p class="warn-line" id="device-stale">The box changed since this list. <button type="button" class="link" data-act="relist">Refresh</button></p>' : ''}`;
}

/**
 * The box chip's second word, after #device-state: what the box does or what
 * to do next. → 'running race' | 'Choose the box' | 'Connect' | ''.
 */
export function deviceChipNote(s, list) {
  if (!s || !s.bluetooth) return '';
  const h = deviceHead(s);
  if (s.state === 'connected') { const a = list?.bundles?.find((b) => b.active); return a ? `running ${a.name}` : ''; }
  if (h.connect) return 'Connect';
  if (h.choose.prominent && s.state !== 'connecting' && s.state !== 'busy') return 'Choose the box';
  return '';
}

/** Seconds as 'about 37 s left' / 'about 3 min left' ('' when not known). */
export function fmtLeft(sec) {
  if (!Number.isFinite(sec) || sec < 0) return '';
  if (sec < 90) return `about ${Math.max(1, Math.round(sec))} s left`;
  return `about ${Math.round(sec / 60)} min left`;
}

/**
 * A firmware update's four stages for the Box panel's tracker, from
 * api.device.status() while it runs (state busy, op 'ota'); null otherwise.
 * → [{ key, label, state: 'done'|'active'|'waiting', detail, pct? }]
 * The stages follow progress.phase: save → firmware → reboot → restore.
 */
export const UPDATE_STAGES = [
  { key: 'save', label: 'Apps kept in this browser' },
  { key: 'firmware', label: 'Sending the firmware' },
  { key: 'reboot', label: 'Restart and trial' },
  { key: 'restore', label: 'Put the apps back' },
];
export function updateStages(s) {
  if (!s || s.state !== 'busy' || s.op !== 'ota') return null;
  const p = s.progress;
  const order = UPDATE_STAGES.map((x) => x.key);
  let at = order.indexOf(p?.phase);
  if (p?.phase === 'done') at = order.length;
  if (at < 0) at = 0;
  return UPDATE_STAGES.map((x, i) => {
    const state = i < at ? 'done' : i === at ? 'active' : 'waiting';
    let detail = '';
    let pct;
    if (x.key === 'save') detail = state === 'done' ? 'Saved in this browser' : state === 'active' ? deviceProgressText(p) || 'listing the apps on the box' : 'Every app on the box is downloaded and saved here first';
    if (x.key === 'firmware') {
      if (state === 'active' && p?.total) {
        pct = Math.floor((p.sent / p.total) * 100);
        detail = [`${fmtBytes(p.sent)} of ${fmtBytes(p.total)}`, p.kbps ? `${p.kbps} KB/s` : null, p.kbps ? fmtLeft((p.total - p.sent) / (p.kbps * 1000)) : null].filter(Boolean).join(' · ');
      } else detail = state === 'done' ? 'Sent and committed' : '';
    }
    if (x.key === 'reboot') detail = state === 'active' ? deviceProgressText(p) : 'The box restarts into the new firmware and keeps it after 20 s of running cleanly. If it fails three times it goes back to the old one by itself.';
    if (x.key === 'restore') detail = state === 'active' ? deviceProgressText(p) : 'The saved apps go back on the box, and the one it started with is made active again';
    return { ...x, state, detail, ...(pct !== undefined ? { pct } : {}) };
  });
}

/**
 * The bundles table's state { list, stale } after a device call (the page's
 * call listener; every call, the page's or an agent's): a list replaces it;
 * an install, activate or delete marks a shown list stale (the page never
 * lists again by itself: that would take the box from an agent); a disconnect
 * clears it. → the same object when nothing changed.
 */
export function deviceTableAfter(t, name, r) {
  // a firmware update that failed may still have changed the box (wiped, or the bundles put back)
  if (name === 'device.updateFirmware' && r) return t.list && !t.stale ? { ...t, stale: true } : t;
  if (!r?.ok || !name.startsWith('device.')) return t;
  if (name === 'device.list') return { list: r, stale: false };
  if (['device.install', 'device.activate', 'device.delete', 'device.updateFirmware'].includes(name)) return t.list && !t.stale ? { ...t, stale: true } : t;
  if (name === 'device.disconnect') return t.list ? { list: null, stale: false } : t;
  return t;
}
