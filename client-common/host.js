// The host protocol's page logic, shared by the Kindle pages (README "Host
// protocol"): when to reload, and the host events a page sends. The bytes
// themselves are the generated bindings' (protocol-js/protocol.js); the
// websocket and the stores are each page's api.js. Nothing newer than ES5
// (the Kindle's old WebKit).
//
// The Kindle is a display kept in sync by the host: it has one entry path,
// `/`, which the host serves as the active package's page. A page shows the
// package (and version) it was served for; when an envelope names another
// package or another ui_hash, it reloads `/` and gets the right one.

// Choose's PackageId.
export var CHOOSE = 0;

// A package's PackageId: FNV-1a (32-bit) of its name, as the host and
// tools/package compute it.
export function packageId(name) {
  var h = 0x811c9dc5;
  for (var i = 0; i < name.length; i++) {
    h ^= name.charCodeAt(i);
    // h * 0x01000193 mod 2^32, without losing bits to doubles
    h = (h + ((h << 1) + (h << 4) + (h << 7) + (h << 8) + (h << 24))) >>> 0;
  }
  return h;
}

// An 8-byte hash as 16 hex digits.
export function hashHex(bytes) {
  var s = "";
  for (var i = 0; i < bytes.length; i++) s += (bytes[i] < 16 ? "0" : "") + bytes[i].toString(16);
  return s;
}

// Which package this page is: its scripts load from the package's files at
// /pkg/<id>/ (tools/package), whatever the page's own URL. `srcs`: the
// pages's script URLs (default: the document's). null if none says.
export function ownPackage(srcs) {
  if (!srcs) {
    srcs = [];
    var scripts = document.getElementsByTagName("script");
    for (var i = 0; i < scripts.length; i++) srcs.push(scripts[i].src || "");
  }
  for (var k = 0; k < srcs.length; k++) {
    var m = /\/pkg\/(\d+)\//.exec(srcs[k]);
    if (m) return parseInt(m[1], 10);
  }
  return null;
}

// For a page of package `id`: a function taking each decoded Envelope and
// returning true when the page must reload `/` (another package is active,
// or this one changed: its ui_hash differs from the first envelope's).
export function staleness(id) {
  var shown = null;
  return function (envelope) {
    var hash = hashHex(envelope.ui_hash);
    if (envelope.active !== id) return true;
    if (shown === null) shown = hash;
    return hash !== shown;
  };
}

// Reload the entry page (the host serves the active package's).
export function reloadEntry() {
  window.location.replace("/");
}

// HostEvent values (protocol.d.ts) for encodeHostEvent.
export function selectEvent(id) {
  return { tag: "Select", value: id };
}

export function backEvent() {
  return { tag: "SelectDefault", value: null };
}

// An event for package `id`'s engine: `bytes` from the package's own
// encoder.
export function engineEvent(id, bytes) {
  return { tag: "Engine", value: { package: id, bytes: bytes } };
}

export function bleEvent(command) {
  return { tag: "Ble", value: { tag: command, value: null } };
}
