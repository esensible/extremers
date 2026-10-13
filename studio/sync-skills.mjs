#!/usr/bin/env node
// sync-skills.mjs — write skills/index.json from skills/<dir>/SKILL.md, so the
// page can serve them through window.api.skills() / api.skill(name).
//
// Origin: dingo-cli/web/sync-skills.sh (its index writer and frontmatter
// reader, ported from Python to JS). The studio's skills are authored here,
// in studio/skills/, so nothing is copied in; this only (re)generates the index.
//
// Run inside the container, from studio/:
//     node sync-skills.mjs            # writes skills/index.json
//     node sync-skills.mjs --check    # exit 1 if skills/index.json is not current

import { readFileSync, readdirSync, statSync, writeFileSync, existsSync } from 'node:fs';
import { join, relative, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

/**
 * Minimal YAML frontmatter reader: top-level `key: value` scalars, including
 * folded (>, >-) and literal (|, |-) block scalars. Throws without frontmatter.
 */
export function frontmatter(text) {
  const lines = text.split('\n');
  if (!lines.length || lines[0].trim() !== '---') throw new Error('no frontmatter');
  const end = lines.findIndex((l, i) => i > 0 && l.trim() === '---');
  if (end < 0) throw new Error('unterminated frontmatter');
  const body = lines.slice(1, end);
  const out = {};
  let i = 0;
  while (i < body.length) {
    const line = body[i];
    if (!line.trim() || /^[ \t]/.test(line) || !line.includes(':')) { i++; continue; }
    const k = line.indexOf(':');
    const key = line.slice(0, k).trim();
    let val = line.slice(k + 1).trim();
    if (['>', '>-', '|', '|-'].includes(val)) {
      const block = [];
      i++;
      while (i < body.length && (/^[ \t]/.test(body[i]) || !body[i].trim())) { block.push(body[i].trim()); i++; }
      out[key] = block.filter(Boolean).join(val.startsWith('>') ? ' ' : '\n').trim();
      continue;
    }
    if (val.length >= 2 && val[0] === val[val.length - 1] && `'"`.includes(val[0])) val = val.slice(1, -1);
    out[key] = val;
    i++;
  }
  return out;
}

function walk(dir) {
  const out = [];
  for (const n of readdirSync(dir).sort()) {
    const p = join(dir, n);
    if (statSync(p).isDirectory()) out.push(...walk(p)); else out.push(p);
  }
  return out;
}

/** skills dir → [{ name, dir, description, path, chars, assets }] (dirs in name order). */
export function buildIndex(skillsDir) {
  const index = [];
  for (const d of readdirSync(skillsDir).sort()) {
    const p = join(skillsDir, d, 'SKILL.md');
    if (!existsSync(p)) continue;
    const text = readFileSync(p, 'utf8');
    let fm;
    try { fm = frontmatter(text); } catch (e) { throw new Error(`${p}: ${e.message}`); }
    if (!fm.name || !fm.description) throw new Error(`${p}: frontmatter needs name and description`);
    const adir = join(skillsDir, d, 'assets');
    const assets = existsSync(adir) ? walk(adir).map((a) => relative(dirname(skillsDir), a).split('\\').join('/')).sort() : [];
    index.push({ name: fm.name, dir: d, description: fm.description, path: `skills/${d}/SKILL.md`, chars: text.length, assets });
  }
  return index;
}

export const indexText = (index) => JSON.stringify(index, null, 2) + '\n';

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const skillsDir = join(dirname(fileURLToPath(import.meta.url)), 'skills');
  const out = join(skillsDir, 'index.json');
  const text = indexText(buildIndex(skillsDir));
  if (process.argv.includes('--check')) {
    const cur = existsSync(out) ? readFileSync(out, 'utf8') : '';
    if (cur !== text) { console.error('skills/index.json is not current: run node sync-skills.mjs'); process.exit(1); }
    console.log('skills/index.json is current');
  } else {
    writeFileSync(out, text);
    console.log(`wrote ${out}: ${JSON.parse(text).map((s) => s.name).join(', ')}`);
  }
}
