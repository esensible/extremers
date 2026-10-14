// device/firmware.js — a firmware image checked before it goes to the box:
// the C6's app image format (extreme_protocol::firmware, what
// `cargo run -p extreme-image -- <elf>` writes as target/xiaoc6.bin): the
// magic, the ESP32-C6's chip id, whole 16-byte lines, room on the box, and
// the appended digest (the image's last 32 bytes, the SHA-256 of the rest),
// which is the firmware's identity (api.device.firmware().sha256). The box
// checks all of it again, and the segments, at its Commit; this catches
// "the ELF instead of the .bin" before anything is sent.
//
// The constants mirror extreme_protocol::firmware; the bindgen vectors'
// synthetic image (protocol-js/test/vectors.json "firmware") is checked
// with this in test/device.test.mjs, so they cannot drift apart.

import { sha256, hex } from './bundles.js';

export const IMAGE_MAGIC = 0xe9;
export const CHIP_ID_C6 = 13;
export const AT_CHIP_ID = 12;
export const DIGEST_LEN = 32;
/** The header (24 B), a segment's header (8 B), and the checksum's line: the least an image can be. */
const MIN_LEN = 24 + 8 + 16 + DIGEST_LEN;

function refused(message, hint) {
  const e = new Error(message);
  e.code = 'IMAGE';
  e.hint = hint;
  return e;
}

/**
 * checkImage(bytes, maxLen) → { len, sha256 (hex: the appended digest) }, or
 * throws an Error (.code 'IMAGE', .hint) saying what is wrong. maxLen: the
 * box's room (api.device.firmware().maxLen; null: not checked).
 */
export async function checkImage(bytes, maxLen = null) {
  const b = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  const make = 'make it from the ELF with `cargo run -p extreme-image -- target/riscv32imac-unknown-none-elf/release/xiaoc6` (it writes target/xiaoc6.bin)';
  if (b.length < MIN_LEN) throw refused(`${b.length} bytes is not a firmware image`, make);
  if (b[0] !== IMAGE_MAGIC) {
    const elf = b[0] === 0x7f && b[1] === 0x45 && b[2] === 0x4c && b[3] === 0x46;
    throw refused(elf ? 'this is the ELF, not the firmware image' : `not a firmware image (its first byte is 0x${b[0].toString(16)}, not 0xe9)`, make);
  }
  const chip = b[AT_CHIP_ID] | (b[AT_CHIP_ID + 1] << 8);
  if (chip !== CHIP_ID_C6) throw refused(`an image for chip id ${chip}, not the ESP32-C6's (${CHIP_ID_C6})`, 'build the firmware for the C6 (tgt-xiaoc6)');
  if (b.length % 16 !== 0) throw refused(`${b.length} bytes is not whole 16-byte lines: the image is cut short or has bytes after it`, make);
  if (maxLen !== null && b.length > maxLen) throw refused(`the image is ${b.length} bytes; the box has room for ${maxLen}`, 'the firmware has outgrown its app slot (tgt-xiaoc6/partitions.csv)');
  const n = b.length - DIGEST_LEN;
  const digest = hex(b.subarray(n));
  const got = hex(await sha256(b.subarray(0, n)));
  if (got !== digest) throw refused(`the image does not end with its digest (the SHA-256 of the rest is ${got}, its last 32 bytes ${digest}): cut short or changed`, make);
  return { len: b.length, sha256: digest };
}
