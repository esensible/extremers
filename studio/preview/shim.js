// preview/shim.js — loaded first in the Kindle preview's page (a classic
// script: preview/rewrite.js puts <script src=".../preview/shim.js"
// data-sid="<sid>"> at the top of <head>). It stands in for the device:
//
//   window.WebSocket → a postMessage bridge to the studio (the parent page,
//     preview/kindle.js). Any URL opens (the page asks for ws://<host>/socket);
//     the socket opens asynchronously, as a real one does. The studio sends
//     the sim host's envelopes: { __preview: sid, type: 'envelope', bytes:
//     ArrayBuffer } → onmessage({ data }) (an ArrayBuffer with binaryType
//     'arraybuffer', else a Blob). send(buf) → { type: 'hostEvent', bytes }
//     to the studio, which gives it to the host (api._sim.send).
//   console.* and uncaught errors → { type: 'console' | 'error' } to the
//     studio (api.preview.status()), and still to the real console.
//
// Every message to the studio is { __preview: sid, type, … }. Plain ES5:
// the bridge must not be what breaks on an old engine.
(function (root) {
  function install(win, sid) {
    var parent = win.parent;
    var origin = win.location && win.location.origin ? win.location.origin : '*';
    var sockets = [];
    var CONNECTING = 0, OPEN = 1, CLOSING = 2, CLOSED = 3;

    function post(type, extra, transfer) {
      var m = { __preview: sid, type: type };
      if (extra) for (var k in extra) if (Object.prototype.hasOwnProperty.call(extra, k)) m[k] = extra[k];
      try { parent.postMessage(m, origin, transfer || []); } catch (e) { /* the studio is gone */ }
    }
    function later(fn) { (win.setTimeout || setTimeout)(fn, 0); }

    function toBuffer(data) {
      var tag = Object.prototype.toString.call(data);
      if (tag === '[object ArrayBuffer]') return data.slice(0);
      if (ArrayBuffer.isView && ArrayBuffer.isView(data)) return data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength);
      if (typeof data === 'string') {
        var bytes = new (win.TextEncoder || TextEncoder)().encode(data);
        return bytes.buffer.slice(0);
      }
      if (data && typeof data.length === 'number') return new Uint8Array(data).buffer;
      throw new TypeError('PreviewSocket.send: ' + Object.prototype.toString.call(data) + ' is not supported (ArrayBuffer, a typed array or a string)');
    }

    function fire(sock, type, ev) {
      ev.type = type; ev.target = sock; ev.currentTarget = sock;
      var h = sock['on' + type];
      if (typeof h === 'function') { try { h.call(sock, ev); } catch (e) { reportError(e); } }
      var ls = (sock._listeners[type] || []).slice();
      for (var i = 0; i < ls.length; i++) { try { ls[i].call(sock, ev); } catch (e) { reportError(e); } }
    }

    function PreviewSocket(url, protocols) {
      if (!(this instanceof PreviewSocket)) throw new TypeError("Failed to construct 'WebSocket': Please use the 'new' operator");
      this.url = String(url);
      this.protocol = '';
      this.extensions = '';
      this.readyState = CONNECTING;
      this.binaryType = 'blob';
      this.bufferedAmount = 0;
      this.onopen = this.onmessage = this.onclose = this.onerror = null;
      this._listeners = {};
      var self = this;
      sockets.push(this);
      later(function () {
        if (self.readyState !== CONNECTING) return;
        self.readyState = OPEN;
        fire(self, 'open', {});
        post('socket-open', { url: self.url });
      });
    }
    PreviewSocket.CONNECTING = PreviewSocket.prototype.CONNECTING = CONNECTING;
    PreviewSocket.OPEN = PreviewSocket.prototype.OPEN = OPEN;
    PreviewSocket.CLOSING = PreviewSocket.prototype.CLOSING = CLOSING;
    PreviewSocket.CLOSED = PreviewSocket.prototype.CLOSED = CLOSED;
    PreviewSocket.prototype.send = function (data) {
      if (this.readyState === CONNECTING) throw new Error("Failed to execute 'send' on 'WebSocket': Still in CONNECTING state.");
      if (this.readyState !== OPEN) return; // a closed socket drops it, as a real one does
      var buf = toBuffer(data);
      post('hostEvent', { bytes: buf }, [buf]);
    };
    PreviewSocket.prototype.close = function (code, reason) {
      if (this.readyState === CLOSING || this.readyState === CLOSED) return;
      var self = this;
      this.readyState = CLOSING;
      post('socket-close', { url: this.url });
      later(function () {
        self.readyState = CLOSED;
        for (var i = 0; i < sockets.length; i++) if (sockets[i] === self) { sockets.splice(i, 1); break; }
        fire(self, 'close', { code: code || 1000, reason: reason || '', wasClean: true });
      });
    };
    PreviewSocket.prototype.addEventListener = function (type, fn) {
      if (typeof fn !== 'function') return;
      (this._listeners[type] = this._listeners[type] || []).push(fn);
    };
    PreviewSocket.prototype.removeEventListener = function (type, fn) {
      var ls = this._listeners[type] || [];
      for (var i = 0; i < ls.length; i++) if (ls[i] === fn) { ls.splice(i, 1); return; }
    };

    function deliver(buf) {
      var open = sockets.slice();
      for (var i = 0; i < open.length; i++) {
        var s = open[i];
        if (s.readyState !== OPEN) continue;
        var data = s.binaryType === 'arraybuffer' ? buf.slice(0) : new (win.Blob || Blob)([buf]);
        fire(s, 'message', { data: data, origin: origin });
      }
      return open.length;
    }

    win.addEventListener('message', function (ev) {
      var d = ev.data;
      if (ev.source !== parent || !d || d.__preview !== sid) return;
      if (d.type === 'envelope' && Object.prototype.toString.call(d.bytes) === '[object ArrayBuffer]') deliver(d.bytes);
    });

    // ---- console and errors
    function text(a) {
      if (typeof a === 'string') return a;
      if (a && a.stack && a.message !== undefined) return String(a.stack).indexOf(String(a.message)) >= 0 ? String(a.stack) : a.message + '\n' + a.stack;
      if (a instanceof Error) return String(a);
      try { var s = JSON.stringify(a); return s === undefined ? String(a) : s; } catch (e) { return String(a); }
    }
    function reportError(e) {
      post('error', { message: text(e) });
      try { if (win.console && win.console.__real && win.console.__real.error) win.console.__real.error(e); } catch (x) { /* nothing */ }
    }
    var con = win.console;
    if (con) {
      var real = {};
      var levels = ['log', 'info', 'warn', 'error', 'debug'];
      for (var i = 0; i < levels.length; i++) {
        (function (level) {
          var orig = con[level];
          real[level] = orig;
          con[level] = function () {
            var parts = [];
            for (var j = 0; j < arguments.length; j++) parts.push(text(arguments[j]));
            post('console', { level: level, text: parts.join(' ') });
            if (typeof orig === 'function') return orig.apply(con, arguments);
          };
        })(levels[i]);
      }
      try { Object.defineProperty(con, '__real', { value: real, configurable: true }); } catch (e) { /* frozen */ }
    }
    win.addEventListener('error', function (ev) {
      var where = ev.filename ? ' (' + ev.filename + ':' + ev.lineno + ':' + ev.colno + ')' : '';
      post('error', { message: (ev.error ? text(ev.error) : String(ev.message)) + where });
    });
    win.addEventListener('unhandledrejection', function (ev) { post('error', { message: 'unhandled rejection: ' + text(ev.reason) }); });
    win.addEventListener('DOMContentLoaded', function () { post('ready', { href: win.location.href }); });
    win.addEventListener('pagehide', function () { post('unload', { href: win.location.href }); });

    win.WebSocket = PreviewSocket;
    post('shim', { href: win.location.href });
    return { PreviewSocket: PreviewSocket, sockets: sockets, deliver: deliver };
  }

  root.__installPreviewShim = install;
  var doc = root.document;
  var me = doc && doc.currentScript;
  if (me && me.getAttribute('data-sid')) root.__previewShim = install(root, me.getAttribute('data-sid'));
})(typeof window !== 'undefined' ? window : this);
