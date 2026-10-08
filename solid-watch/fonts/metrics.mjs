// Prints the vertical metrics of a TrueType font, its variation axes and
// whether it has the tnum feature and some glyphs we rely on. Used to derive
// the line-height ratio both hosts use (README: Font).
//   node fonts/metrics.mjs fonts/Inter-roman.ttf
import { readFileSync } from "node:fs";

const buf = readFileSync(process.argv[2]);
const u16 = (o) => buf.readUInt16BE(o), i16 = (o) => buf.readInt16BE(o), u32 = (o) => buf.readUInt32BE(o);
const tables = {};
for (let i = 0, n = u16(4); i < n; i++) {
  const r = 12 + i * 16;
  tables[buf.toString("latin1", r, r + 4)] = u32(r + 8);
}
const head = tables["head"], hhea = tables["hhea"], os2 = tables["OS/2"];
const upm = u16(head + 18);
const asc = i16(hhea + 4), desc = i16(hhea + 6), gap = i16(hhea + 8);
const typoAsc = i16(os2 + 68), typoDesc = i16(os2 + 70), typoGap = i16(os2 + 72);
const winAsc = u16(os2 + 74), winDesc = u16(os2 + 76);
const fsSelection = u16(os2 + 62);
console.log({ unitsPerEm: upm, hhea: { asc, desc, gap }, typo: { typoAsc, typoDesc, typoGap }, win: { winAsc, winDesc }, useTypoMetrics: !!(fsSelection & 0x80) });
console.log("hhea line height ratio (asc - desc + gap) / upm =", (asc - desc + gap) / upm);
console.log("ascent ratio =", asc / upm, "descent ratio =", -desc / upm);

// fvar axes
if (tables["fvar"]) {
  const f = tables["fvar"];
  const axesOff = f + u16(f + 4), count = u16(f + 8), size = u16(f + 10);
  for (let i = 0; i < count; i++) {
    const a = axesOff + i * size;
    const fx = (o) => buf.readInt32BE(o) / 65536;
    console.log("axis", buf.toString("latin1", a, a + 4), "min", fx(a + 4), "default", fx(a + 8), "max", fx(a + 12));
  }
} else console.log("no fvar (not variable)");

// GSUB feature list
if (tables["GSUB"]) {
  const g = tables["GSUB"], fl = g + u16(g + 6), n = u16(fl);
  const tags = new Set();
  for (let i = 0; i < n; i++) tags.add(buf.toString("latin1", fl + 2 + i * 6, fl + 6 + i * 6));
  console.log("GSUB features:", [...tags].join(" "));
}

// cmap: format 4 lookup for a few code points
const cmap = tables["cmap"];
let sub = null;
for (let i = 0, n = u16(cmap + 2); i < n; i++) {
  const r = cmap + 4 + i * 8;
  const off = cmap + u32(r + 4);
  if (u16(off) === 4) { sub = off; break; }
}
function has(cp) {
  const segX2 = u16(sub + 6), ends = sub + 14, starts = ends + segX2 + 2;
  for (let i = 0; i < segX2 / 2; i++) if (cp <= u16(ends + i * 2)) return cp >= u16(starts + i * 2);
  return false;
}
for (const [name, cp] of [["minus U+2212", 0x2212], ["degree U+00B0", 0xb0], ["ellipsis U+2026", 0x2026], ["tilde", 0x7e]]) console.log(name, has(cp) ? "yes" : "NO");
