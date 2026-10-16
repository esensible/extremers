// Fakes for files-in.js: a drop zone or an <input type="file"> (an
// EventTarget with a classList, files and value), a File-like object, and a
// person's choice or drop on them.

/** An element enough for files-in.js: events, a classList, and whatever `extra` adds (files, value, disabled). */
export function fakeElement(extra = {}) {
  const el = new EventTarget();
  const classes = new Set();
  el.classList = { add: (c) => classes.add(c), remove: (c) => classes.delete(c), contains: (c) => classes.has(c) };
  return Object.assign(el, extra);
}

/** A File-like object: a name and its bytes. */
export const fileLike = (name, bytes) => ({ name, size: bytes.length, arrayBuffer: async () => bytes.slice().buffer });

/** Chooses `file` in a fake <input type=file> as Chrome does (files set, value the fake path), then fires change. */
export function choose(input, file) {
  input.files = file ? [file] : [];
  input.value = file ? `C:\\fakepath\\${file.name}` : '';
  input.dispatchEvent(new Event('change'));
}

/** Drops `file` on a fake zone; → the drop event (defaultPrevented: the browser was kept from opening the file). */
export function drop(zone, file) {
  const e = new Event('drop', { cancelable: true });
  e.dataTransfer = { files: file ? [file] : [] };
  zone.dispatchEvent(e);
  return e;
}
