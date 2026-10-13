// skills.test.mjs — sync-skills.mjs: the frontmatter reader, and that
// skills/index.json is what the generator makes from skills/*/SKILL.md.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';

import { frontmatter, buildIndex, indexText } from '../sync-skills.mjs';

const studioDir = join(dirname(fileURLToPath(import.meta.url)), '..');

test('frontmatter: scalars, quotes, folded and literal blocks', () => {
  const fm = frontmatter('---\nname: x\nq: "quoted: yes"\ndescription: >-\n  one\n  two\n\n  three\nlit: |\n  a\n  b\n---\nbody');
  assert.deepEqual(fm, { name: 'x', q: 'quoted: yes', description: 'one two three', lit: 'a\nb' });
  assert.throws(() => frontmatter('no frontmatter'), /no frontmatter/);
  assert.throws(() => frontmatter('---\nname: x\n'), /unterminated/);
});

test('buildIndex: one entry per SKILL.md, with assets; refuses one without name/description', () => {
  const d = mkdtempSync(join(tmpdir(), 'skills-'));
  const bText = '---\nname: bee\ndescription: B.\n---\n# B\n';
  const aText = '---\nname: ay\ndescription: >\n  A thing.\n---\n';
  mkdirSync(join(d, 'b')); writeFileSync(join(d, 'b', 'SKILL.md'), bText);
  mkdirSync(join(d, 'a', 'assets', 'img'), { recursive: true });
  writeFileSync(join(d, 'a', 'SKILL.md'), aText);
  writeFileSync(join(d, 'a', 'assets', 'img', 'x.svg'), '<svg/>');
  mkdirSync(join(d, 'not-a-skill'));
  const idx = buildIndex(d);
  const base = d.split('/').pop();
  assert.deepEqual(idx, [
    { name: 'ay', dir: 'a', description: 'A thing.', path: 'skills/a/SKILL.md', chars: aText.length, assets: [`${base}/a/assets/img/x.svg`] },
    { name: 'bee', dir: 'b', description: 'B.', path: 'skills/b/SKILL.md', chars: bText.length, assets: [] },
  ]);
  writeFileSync(join(d, 'b', 'SKILL.md'), '---\nname: bee\n---\n');
  assert.throws(() => buildIndex(d), /needs name and description/);
});

test('skills/index.json is current (run `node sync-skills.mjs` after editing a skill)', () => {
  const want = indexText(buildIndex(join(studioDir, 'skills')));
  assert.equal(readFileSync(join(studioDir, 'skills', 'index.json'), 'utf8'), want);
  const idx = JSON.parse(want);
  assert.deepEqual(idx.map((s) => s.name), ['expr-engine', 'kindle-ui', 'race-example', 'studio-api', 'watch-ui']);
  for (const s of idx) {
    assert.equal(s.name, s.dir, 'skill name = its directory');
    assert.ok(s.description.length > 40, s.name);
    const md = readFileSync(join(studioDir, s.path), 'utf8');
    // "evaluation", never "step": the word appears only where a skill forbids it
    for (const line of md.split('\n').filter((l) => /\bstep(s|ped|ping)?\b/i.test(l))) {
      assert.match(line, /never|no `step`/i, `${s.name}: ${line}`);
    }
  }
});
