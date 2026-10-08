// TuneSpeed compact state (BLE.md "State" and "TuneSpeed (kind 2)"):
//   [kind u8 = 2][speed u16 LE, kn x100][speed_dev i16 LE, kn x100][heading_dev i16 LE, deg x10]
// Encoder: extreme-tune/src/tune.rs `compact_state`; test/vectors.json is
// written by its Rust test. The object has the WebSocket JSON's field names.
// Returns null for another engine's state or a short buffer.
export const KIND = 2;

const u16 = (b, i) => b[i] | (b[i + 1] << 8);
const i16 = (b, i) => (u16(b, i) << 16) >> 16;

export function decode(b) {
  if (!b || b.length < 7 || b[0] !== KIND) return null;
  return {
    speed: u16(b, 1) / 100,
    speed_dev: i16(b, 3) / 100,
    heading_dev: i16(b, 5) / 10,
  };
}
