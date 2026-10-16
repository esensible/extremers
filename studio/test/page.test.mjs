// page.test.mjs — the page's structure (index.html and the scripts that wire
// it): every id the scripts look up exists, every id the studio-api skill
// names for reading the page exists, the menus and panels are reachable and
// closable, and the dark tokens read well.
//
//   podman exec -w /workspace/studio extremers-lang npm test

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { studioDir } from './engine-world.mjs';
import { studioTheme } from '../editor/lang.js';

const read = (...p) => readFileSync(join(studioDir, ...p), 'utf8');
const html = read('index.html');
const app = read('app.js');
const panels = read('panels.js');
// what the scripts draw into the page (ids in their templates or set in code)
const drawn = [read('ui.js'), app, panels, read('editor.js'), read('preview', 'kindle.js'), read('preview', 'watch.js')].join('\n');
const exists = (id) => html.includes(`id="${id}"`) || drawn.includes(`id="${id}"`) || drawn.includes(`'${id}'`);

test('every element app.js and panels.js look up by id exists in the page or in what they draw', () => {
  for (const [name, src] of [['app.js', app], ['panels.js', panels]]) {
    for (const [, id] of src.matchAll(/\$\('([\w-]+)'\)/g)) assert.ok(html.includes(`id="${id}"`) || drawn.includes(`id="${id}"`), `${name}: #${id}`);
  }
  // preview/index.js: the screens' ids are in the page (its Open/Close are optional: there are none now)
  const pv = read('preview', 'index.js');
  const optional = new Set(['preview-kindle-close', 'preview-watch-close', 'preview-kindle-frame']);
  for (const [, id] of pv.matchAll(/\$\('([\w-]+)'\)/g)) if (!optional.has(id)) assert.ok(html.includes(`id="${id}"`), `preview/index.js: #${id}`);
});

test('every id the studio-api skill names in "The page, for reading it" exists', () => {
  const skill = read('skills', 'studio-api', 'SKILL.md');
  const sect = skill.slice(skill.indexOf('## The page, for reading it'), skill.indexOf('## Words'));
  const ids = [...new Set([...sect.matchAll(/#([a-z][\w-]*)(?![\w<-])/g)].map((m) => m[1]))].filter((id) => !id.endsWith('-'));
  assert.ok(ids.length > 80, `found ${ids.length}`);
  for (const id of ids) assert.ok(exists(id), `#${id}`);
  // and the ids the skill's other sections lean on
  for (const id of ['device-choose', 'device-howto', 'device-firmware-file', 'project-import-file', 'project-import-name', 'device-update-yes', 'skill-view', 'diagnostics']) assert.ok(exists(id), `#${id}`);
  // the ids the brief keeps stable
  for (const id of ['device-firmware-file', 'project-import-file', 'device-update-confirm', 'device-update-text', 'device-update-yes', 'device-update-no', 'device-restore', 'device-restore-resume', 'device-choose', 'preview-kindle-waiting', 'preview-watch-waiting', 'preview-kindle-screen', 'preview-watch-screen', 'editors', 'editor-tabs']) assert.ok(html.includes(`id="${id}"`), `#${id}`);
});

test('the top bar: the project menu, Run, the box chip and help are buttons that say whether they are open', () => {
  const tag = (id) => (html.match(new RegExp(`<[a-z]+[^>]*\\bid="${id}"[^>]*>`)) ?? [])[0] ?? '';
  for (const [b, target] of [['project-button', 'project-menu'], ['box-chip', 'device'], ['help-button', 'reading']]) {
    assert.match(tag(b), new RegExp(`^<button type="button"[^>]*aria-expanded="false" aria-controls="${target}"`), b);
    assert.match(tag(target), /\bhidden\b/, `#${target} closed at first`);
  }
  assert.match(tag('run'), /^<button type="button" class="btn pri" id="run"/);
  assert.match(tag('help-button'), /aria-label="Help and guides"/);
  assert.match(tag('project-new-dialog'), /role="dialog"[^>]*hidden/);
  // the scripts keep aria-expanded true while open, and Escape / a click outside / the scrim close them
  assert.match(app, /setAttribute\('aria-expanded', 'true'\)/);
  assert.match(app, /if \(e\.key !== 'Escape'\) return;/);
  assert.match(app, /if \(pop && !e\.target\.closest\?\.\('\.popover, #project-button'\)\) closePop\(false\);/);
  assert.match(app, /\$\('scrim'\)\.onclick = \(\) => closeSide\(\);/);
  // the design's words: the bottom tabs, the clock's two settings (never "step")
  for (const w of ['>Problems ', '>Test runs ', '>Log<', '>Live<', '>Manual clock<', '>Send fix<', '>Last evaluation<', '>New file<', '>New project…<', '>Export as ZIP<', '>Delete…<', '>Rename…<']) assert.ok(html.includes(w), w);
  assert.doesNotMatch(html, /\bstep(s|ped|ping)?\b/i);
});

test('Run: the whole sequence through window.api, in order; a failure stops it at Problems', () => {
  const run = app.slice(app.indexOf('async function runAll()'), app.indexOf("$('run').onclick"));
  const at = ['window.api.engine.check()', 'window.api.ui.build(', 'window.api.sim.start(', 'window.api.preview.open('].map((s) => run.indexOf(s));
  assert.ok(at.every((i, k) => i > 0 && (k === 0 || i > at[k - 1])), `in order: ${at}`);
  assert.equal((run.match(/\{ stop\(/g) ?? []).length, 4, 'each of the four parts stops the sequence on failure');
  assert.match(run, /selectPanel\(\$\('tab-problems'\)\)/);
  assert.match(run, /panels\.firstProblem\(\)/);
});

test('the dark tokens: text reads at 4.5:1 or more on the ground and the panels; the editor matches', () => {
  const tok = (name) => (html.match(new RegExp(`--${name}: (#[0-9A-F]{6})`)) ?? [])[1];
  const lum = (hex) => { const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)); return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]; };
  const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
  assert.equal(tok('ground'), '#111418');
  assert.equal(tok('accent'), '#F2A33A');
  for (const bg of ['ground', 'chrome', 'panel', 'field', 'hover']) {
    for (const fg of ['text', 'text-2', 'muted', 'accent', 'good', 'warn', 'bad', 'info']) assert.ok(ratio(tok(fg), tok(bg)) >= 4.5, `${fg} on ${bg}: ${ratio(tok(fg), tok(bg)).toFixed(2)}`);
  }
  assert.ok(ratio(tok('on-accent'), tok('accent')) >= 4.5, 'the Run button');
  assert.match(html, /color-scheme: dark;/);
  assert.match(html, /:focus-visible \{ outline: 2px solid var\(--accent\)/);
  assert.match(html, /family=IBM\+Plex\+Sans[^"]*family=JetBrains\+Mono/);
  assert.equal(studioTheme.colors['editor.background'], tok('ground'));
});
