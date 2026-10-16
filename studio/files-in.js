// files-in.js — how a local file gets into the page: dropped on a zone, or
// chosen in an <input type="file"> (a person's picker; an agent driving the
// browser sets the input's files with its file-upload tool instead of
// clicking it). Both hand the first file to one function, so a drop and a
// choice take exactly the same path. What to do with a firmware image and a
// project ZIP is here too, without the DOM, so Node tests it.

import { toBase64 } from './zip.js';
import { deviceUpdateConfirmText } from './ui.js';

/** `zone` takes a dropped file: `take(file)` with the first one; `.drag` while one is over it. */
export function takeDropped(zone, take) {
  zone.addEventListener('dragover', (e) => { e.preventDefault(); zone.classList.add('drag'); });
  zone.addEventListener('dragleave', () => zone.classList.remove('drag'));
  zone.addEventListener('drop', (e) => {
    e.preventDefault(); zone.classList.remove('drag');
    const f = e.dataTransfer?.files?.[0];
    if (f) take(f);
  });
}

/**
 * `input` (type=file) takes a chosen file: `take(file)` with the first one.
 * Its value is reset at once (the File stays readable), so choosing the same
 * file again is a change too.
 */
export function takeChosen(input, take) {
  input.addEventListener('change', () => {
    const f = input.files?.[0];
    input.value = '';
    if (f) take(f);
  });
}

/**
 * A firmware image (a dropped or chosen file) → what the inline confirm
 * offers: { ok, pending: { name, len, base64 }, text }, or { ok: false,
 * error } for a file that is not a `.bin`. `bundles`: how many the box holds
 * besides Choose (the last list's; null: not listed).
 */
export async function firmwareOffer(file, bundles = null) {
  if (!/\.bin$/i.test(file.name)) return { ok: false, error: `Firmware: ${file.name} is not a firmware image (.bin, as cargo run -p extreme-image writes)` };
  const bytes = new Uint8Array(await file.arrayBuffer());
  const pending = { name: file.name, len: bytes.length, base64: toBase64(bytes) };
  return { ok: true, pending, text: deviceUpdateConfirmText(pending, bundles) };
}

/**
 * A project ZIP (a dropped or chosen file) → { ok, name, source: { zipBase64 } }
 * for api.project.import (name: the file's, without `.zip`), or { ok: false,
 * error } for a file that is not a `.zip`.
 */
export async function projectZipSource(file) {
  if (!/\.zip$/i.test(file.name)) return { ok: false, error: `Import: ${file.name} is not a .zip` };
  return { ok: true, name: file.name.replace(/\.zip$/i, ''), source: { zipBase64: toBase64(new Uint8Array(await file.arrayBuffer())) } };
}
