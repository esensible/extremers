// The host protocol's page logic, shared by the Kindle pages (README "Host
// protocol"): which page an envelope wants shown, and the host events a
// page sends. The bytes themselves are the generated bindings'
// (protocol-js/protocol.js); the websocket and the stores are each page's
// api.js. Nothing newer than ES5 (the Kindle's old WebKit).
//
// Every package's page lives at /pkg/<id>/ (Choose's at /). A page is
// loaded as <path>?h=<ui_hash>: the hash of the UIs the host had active
// when it sent the page there. An envelope for another package, or with
// another hash (the package was updated), loads the page it names instead.
// A page loaded without a hash (typed in, or the captive portal's /) takes
// the first envelope's.

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

// The page of package `id`: "/" for Choose, "/pkg/<id>/" for an app.
export function pagePath(id) {
  return id === CHOOSE ? "/" : "/pkg/" + id + "/";
}

// The PackageId of the page at `pathname` (a location's), or null.
export function pageId(pathname) {
  var path = pathname.replace(/index\.html$/, "");
  if (path === "/") return CHOOSE;
  var m = /^\/pkg\/(\d+)\/$/.exec(path);
  return m ? parseInt(m[1], 10) : null;
}

// For a page at `loc` (window.location, or { pathname, search }): a
// function taking each decoded Envelope and returning the URL to load
// instead, or null when this page is the one to show.
export function navigator(loc) {
  var here = pageId(loc.pathname);
  var m = /[?&]h=([0-9a-f]{16})/.exec(loc.search || "");
  var shown = m ? m[1] : null;
  return function (envelope) {
    var hash = hashHex(envelope.ui_hash);
    if (here !== envelope.active || (shown !== null && shown !== hash)) {
      return pagePath(envelope.active) + "?h=" + hash;
    }
    shown = hash;
    return null;
  };
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
