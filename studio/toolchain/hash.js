// toolchain/hash.js — the hashes the toolchain records, without its
// libraries (so the page can compute them without loading the bundler).

const enc = new TextEncoder();
const bytesOf = (x) => (typeof x === 'string' ? enc.encode(x) : x);

export async function sha256Hex(data) {
  const d = await crypto.subtle.digest('SHA-256', bytesOf(data));
  return [...new Uint8Array(d)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

/** SHA-256 over a project's files (path order; path, NUL, content, NUL): which sources a build was made from. */
export async function sourceSha256(files) {
  const parts = [];
  for (const f of [...files].sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0))) parts.push(enc.encode(f.path + '\0'), bytesOf(f.text ?? f.bytes ?? ''), enc.encode('\0'));
  const all = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) { all.set(p, o); o += p.length; }
  return sha256Hex(all);
}
