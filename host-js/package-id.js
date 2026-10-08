// A package's id (host contract: PackageId): FNV-1a 32-bit of its name, as
// the host computes it. Choose is 0.
export const CHOOSE = 0;

export function packageId(name) {
  var h = 0x811c9dc5;
  for (var i = 0; i < name.length; i++) {
    h ^= name.charCodeAt(i) & 0xff;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h >>> 0;
}
