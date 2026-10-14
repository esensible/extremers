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
  if (!fw) return 'Firmware: not asked yet (Firmware asks the box)';
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
        : "Not connected: this page has not chosen the box yet. Start on the Kindle (Choose → Settings → Pair watch), then click 'Choose the box…' at the top right (in full below)";
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

/**
 * The bundles of api.device.list() as a table. Each row: data-id, data-name;
 * its buttons [data-act=activate|delete|get] (ids device-<act>-<id>); a row
 * whose action waits for its inline confirm (confirm: { id, act }) shows
 * device-confirm-yes / device-confirm-no instead.
 */
export function deviceRowsHtml(list, confirm = null, stale = false) {
  if (!list) return '<p class="hint">List shows the bundles on the box.</p>';
  const rows = list.bundles.map((b) => {
    const flags = [b.active && 'active', b.boot && 'boots', b.builtin && 'built in'].filter(Boolean).join(', ');
    const asking = confirm && confirm.id === b.id;
    const verb = { activate: 'Activate', delete: 'Delete' }[confirm?.act];
    const acts = asking
      ? `<span class="confirm">${esc(verb)} ${esc(b.name)}${confirm.act === 'activate' ? ' (the box restarts on it)' : ''}? <button type="button" class="btn danger solid" id="device-confirm-yes" data-act="confirm-${esc(confirm.act)}" data-id="${b.id}">${esc(verb)}</button><button type="button" class="btn" id="device-confirm-no" data-act="cancel">Cancel</button></span>`
      : [
        b.active ? '' : `<button type="button" class="btn" id="device-activate-${b.id}" data-act="activate" data-id="${b.id}">Activate</button>`,
        b.builtin ? '' : `<button type="button" class="btn danger" id="device-delete-${b.id}" data-act="delete" data-id="${b.id}">Delete…</button>`,
        `<button type="button" class="btn" id="device-get-${b.id}" data-act="get" data-id="${b.id}">Get</button>`,
      ].join('');
    return `<tr data-id="${b.id}" data-name="${esc(b.name)}"><td>${esc(b.name)}</td><td class="mono">${b.id}</td><td>${b.version}</td><td>${fmtBytes(b.len)}</td><td class="mono" title="ui.web ${esc(b.ui.web)}, ui.watch ${esc(b.ui.watch)}">${esc(b.contentHash)}</td><td>${esc(flags)}</td><td class="acts">${acts}</td></tr>`;
  }).join('');
  return `<table class="dev-table"><thead><tr><th>name</th><th>id</th><th>v</th><th>bytes</th><th>content hash</th><th>flags</th><th></th></tr></thead><tbody>${rows}</tbody></table>
<p class="hint" id="device-space">${fmtBytes(list.free)} free; the largest bundle that fits: ${fmtBytes(list.largestFree)}</p>${stale ? '\n<p class="hint c-danger" id="device-stale">The box changed since this list: List again.</p>' : ''}`;
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
