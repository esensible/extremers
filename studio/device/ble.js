// device/ble.js — the Web Bluetooth link to the C6's bundle service
// (BLE.md "Bundles", "Security").
//
// One GATT service on the box (`nacra`), three of its characteristics: requests
// go to `event` (write with response), every answer comes back notified on
// `bundle`, a bundle's bytes travel on `data` ([offset u32 LE][bytes], written
// without response for an upload, notified for a download). This module is the
// link only: connect without the chooser (getDevices → watchAdvertisements →
// gatt.connect), the chooser for a person's click, one GATT operation at a
// time, subscribing with the pairing trigger, reconnecting after a drop, and
// Chrome's errors mapped to what a person must do. The protocol is bundles.js.
//
// Chrome grants each box a page chooses separately and getDevices() lists all
// of them, all named 'nacra' (two C6s chosen on different days are two). The
// page keeps the id of the box it last chose or connected to (localStorage,
// DEVICE_ID_KEY) and connects to that one; with several granted and none of
// them remembered (or the remembered one refused as out of range) it watches
// all of them for an advertisement and takes the first.
//
// Origins: dingo-cli/web/ble.js createBridge (the GATT queue, the remembered
// device → advertisement → connect path, reconnect with backoff, the refusal
// and stale-pairing mapping, the timeouts) and bundle/js/client.js (removed; in git history) (the UUIDs,
// subscribing first and writing a request to make the OS pair).
//
// No DOM: everything comes in through createLink({ bluetooth, log, ... }).

export const DEVICE_NAME = 'nacra';
export const SERVICE = 'e4a1c000-6b2f-4f77-9a0d-3c5e7b9d1f20';
export const EVENT = 'e4a1c002-6b2f-4f77-9a0d-3c5e7b9d1f20';
export const BUNDLE = 'e4a1c004-6b2f-4f77-9a0d-3c5e7b9d1f20';
export const DATA = 'e4a1c005-6b2f-4f77-9a0d-3c5e7b9d1f20';

/** HostEvent::Bundle(List), postcard: the write that makes the OS pair (BLE.md "Bytes"). */
export const LIST_REQUEST = Uint8Array.of(0x04, 0x00);

// What a person (or an agent asking one) must do. Kept in one place: the
// studio-api skill quotes them.
export const HINTS = {
  NO_BLUETOOTH: 'Web Bluetooth is not available here: open the studio in desktop Chrome on http://localhost:5190 (Web Bluetooth needs Chrome and a secure origin)',
  CHOOSE: "ask the person, in order: (1) if this Mac is not paired with the box yet, on the Kindle Choose → Settings → Pair watch (opens the box's pairing window for 2 minutes; named for the watch, it pairs any device, one per window); (2) click 'Choose the box…' (#device-choose, top right of the studio): Chrome's popup ('… wants to pair') only asks which device this page may talk to: pick 'nacra'; (3) accept macOS's pairing prompt that follows, if one comes. Chrome opens its chooser only from a person's click, and each browser origin must choose the box once even if the Mac is already paired. After that api.device.connect() works from script without prompts, also after a reload",
  FLAGS: 'for a remembered device without the chooser, chrome://flags/#enable-web-bluetooth-new-permissions-backend must be Enabled (then relaunch Chrome)',
  PAIR: "the box serves bundles only to a computer it has bonded with. Ask the person to open its pairing window: on the Kindle, Choose → Settings → Pair watch (2 minutes; named for the watch, it pairs any device, one device per window), or dev-pairing firmware: 120 s after boot; then api.device.connect() again, and the person accepts macOS's pairing prompt. If this computer was paired before the box lost its bond (provision, Forget all), it must first forget 'nacra' in macOS System Settings → Bluetooth (it may only be listed in the menu-bar Bluetooth menu)",
  STALE: "this computer probably holds an old pairing with 'nacra' that the box no longer has: forget 'nacra' in macOS System Settings → Bluetooth (it may only be listed in the menu-bar Bluetooth menu), open the box's pairing window (Kindle: Choose → Settings → Pair watch), then api.device.connect() and accept macOS's pairing prompt. If the box is not found at all, ask the person to click 'Choose another box…' (#device-choose) and pick 'nacra'",
  RANGE: "the box was not seen advertising: is it powered and within range? It also stops advertising while both of its connection slots are taken: the watch plus another studio tab or window holding the link (disconnect there first: its Device panel's Disconnect, or close it). Chrome remembers each box this page was given separately (two 'nacra's chosen on different days are two boxes); connect() prefers the last one chosen and otherwise takes the one that advertises. If this is a different box than before, click 'Choose another box…' (#device-choose) and pick 'nacra', then api.device.connect()",
  NOT_GRANTED: "Chrome has not granted this page the bundle service on 'nacra' (it was picked before the service existed): ask the person to click 'Choose another box…' (#device-choose) and pick 'nacra' again",
  NOT_CONNECTED: 'api.device.connect() first (api.device.status() shows the link)',
};

/** Where the page keeps the id of the box it last chose or connected to. */
export const DEVICE_ID_KEY = 'studio.device.id';

/**
 * A BluetoothDevice.id (stable per origin, base64 with '::' or '=' padding)
 * as a short tag: its last 4 characters without the padding.
 */
export function idTag(id) {
  return typeof id === 'string' && id ? id.replace(/[:=]+$/, '').slice(-4) : '';
}

export class LinkError extends Error {
  constructor(message, code, hint) { super(message); this.code = code; this.hint = hint; }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function withTimeout(p, ms, what) {
  let t;
  return Promise.race([
    Promise.resolve(p).finally(() => clearTimeout(t)),
    new Promise((_, rej) => { t = setTimeout(() => rej(new Error(`${what} timed out after ${ms} ms`)), ms); }),
  ]);
}

export function errText(e) {
  if (!e) return 'unknown error';
  return (e.name && e.name !== 'Error' ? e.name + ': ' : '') + (e.message || String(e));
}

const toBytes = (dv) => new Uint8Array(dv.buffer, dv.byteOffset, dv.byteLength).slice();

// ------------------------------------------------------------- refusals ----
//
// On a link without a bond the box answers a subscription or a write request
// with ATT Insufficient Authentication (0x05) while its pairing window is open
// (the OS pairs and retries) and Insufficient Authorization (0x08) otherwise.
// What reaches the page (dingo-cli/web/ble.js, from Chromium's source; seen on
// this box from Chrome on macOS, 2026-10-13):
//  - macOS: CoreBluetooth's ATT errors are not translated, so any refused GATT
//    operation is NotSupportedError "GATT operation failed for unknown reason."
//  - platforms that translate 0x08: SecurityError "GATT operation not authorized."
// Neither says which; a refusal on the subscription is taken as "not bonded".
export function isRefusal(e) {
  const t = errText(e);
  if (e?.name === 'SecurityError') return /not authori[sz]ed/i.test(t);
  return e?.name === 'NotSupportedError' && /failed for unknown reason|not permitted|GATT Error Unknown/i.test(t);
}

/**
 * createLink({ bluetooth, log, ...options }) → link.
 *
 * Options (all optional; the defaults are for the hardware, tests shorten them):
 *   connectTimeoutMs (15000)  gatt.connect() + service discovery, each
 *   advertTimeoutMs (12000)   waiting for an advertisement from a remembered device
 *   reconnectTries (3)        attempts after an unexpected drop
 *   reconnectDelayMs (1000)   backoff base (× the attempt number)
 *   pairAttempts (6)          subscriptions tried while the OS pairs
 *   pairRetryMs (2000)        between them (a person confirms the prompt meanwhile)
 *   storage (localStorage)    where the chosen box's id is kept (getItem/setItem;
 *                             null: nowhere). Every access is guarded: a
 *                             missing or throwing storage only loses the preference.
 */
export function createLink({
  bluetooth = globalThis.navigator?.bluetooth,
  log = () => {},
  storage,
  connectTimeoutMs = 15000,
  advertTimeoutMs = 12000,
  reconnectTries = 3,
  reconnectDelayMs = 1000,
  pairAttempts = 6,
  pairRetryMs = 2000,
} = {}) {
  const L = (level, msg) => { try { log(level, `ble: ${msg}`); } catch { /* logging must never break the link */ } };

  let device = null;
  let chars = null; // { event, bundle, data }
  let state = 'disconnected'; // disconnected | connecting | connected
  let bonded = null; // true: subscribed on an encrypted link; false: refused; null: not known
  let lastError = null;
  let lastHint = null;
  let userDisconnect = false;
  let connecting = null;
  let reconnecting = null;
  let reconnects = 0;
  let droppedWhileConnecting = false;
  let remembered = null; // true: Chrome holds a permission for the box; false: none; null: not looked yet (or getDevices is missing)
  let grantedIds = null; // Set of the ids of the boxes named DEVICE_NAME Chrome has granted this page (null: not looked yet)
  const listening = new WeakSet();

  // ---- the chosen box's id (localStorage; the page works without it)
  function theStorage() {
    try { return storage === undefined ? (globalThis.localStorage ?? null) : storage; } catch { return null; }
  }
  function storedId() {
    try { return theStorage()?.getItem(DEVICE_ID_KEY) || null; } catch { return null; }
  }
  function storeId(id) {
    if (!id) return;
    try { theStorage()?.setItem(DEVICE_ID_KEY, id); } catch (e) { L('warn', `could not remember the box's id: ${errText(e)}`); }
  }
  const tagged = (dev) => `${dev.name || 'the box'} (…${idTag(dev.id)})`;
  const bundleListeners = new Set();
  const dataListeners = new Set();
  const dropListeners = new Set();
  const stateListeners = new Set();

  function emit(set, ...args) {
    for (const cb of [...set]) { try { cb(...args); } catch (e) { L('error', `listener threw: ${errText(e)}`); } }
  }
  function setState(s) {
    if (s === state) return;
    state = s;
    L('info', `state → ${s}`);
    emit(stateListeners, s);
  }
  function fail(e) {
    lastError = errText(e);
    lastHint = e?.hint ?? null;
    return e;
  }

  // ---- one GATT operation at a time (Web Bluetooth rejects concurrent ones)
  let chain = Promise.resolve();
  function gattOp(fn) {
    const p = chain.then(fn, fn);
    chain = p.then(() => {}, () => {});
    return p;
  }

  function onBundleValue(ev) { emit(bundleListeners, toBytes(ev.target.value)); }
  function onDataValue(ev) { emit(dataListeners, toBytes(ev.target.value)); }

  function onDisconnected(ev) {
    if (ev?.target && device && ev.target !== device) return; // a device since replaced
    const wasConnected = state === 'connected';
    chars = null;
    if (userDisconnect) { setState('disconnected'); return; }
    if (!wasConnected) { droppedWhileConnecting = true; return; } // the connect path reports it
    L('warn', 'the link dropped');
    emit(dropListeners);
    reconnecting = autoReconnect().finally(() => { reconnecting = null; });
  }

  async function autoReconnect() {
    setState('connecting');
    for (let attempt = 1; attempt <= reconnectTries; attempt++) {
      await sleep(reconnectDelayMs * attempt);
      if (userDisconnect) return;
      try {
        L('info', `reconnect attempt ${attempt}/${reconnectTries}`);
        await openDevice(device, { fromReconnect: true });
        reconnects++;
        lastError = null; lastHint = null;
        L('info', `reconnected (attempt ${attempt})`);
        return;
      } catch (e) {
        fail(e);
        L('warn', `reconnect attempt ${attempt} failed: ${errText(e)}`);
        if (e.code === 'NOT_BONDED' || e.code === 'NOT_GRANTED') break; // retrying cannot fix these
        setState('connecting');
      }
    }
    if (!userDisconnect) {
      lastError = `reconnecting gave up: ${lastError}`;
      setState('disconnected');
    }
  }

  /**
   * Watch advertisements on every device of `devs` at once (one
   * AbortController each) for advertTimeoutMs → the first that advertises, or
   * null. Every watch is stopped before it returns.
   */
  async function firstAdvertising(devs) {
    const watchable = devs.filter((d) => typeof d.watchAdvertisements === 'function');
    if (!watchable.length) {
      throw new LinkError(`'${devs[0]?.name || devs[0]?.id}' is not in range for Chrome, and this Chrome has no watchAdvertisements()`, 'NO_ADVERTISEMENTS', HINTS.FLAGS);
    }
    const watches = watchable.map((dev) => {
      const ac = typeof AbortController === 'function' ? new AbortController() : null;
      let handler;
      const seen = new Promise((resolve) => { handler = () => resolve(dev); dev.addEventListener('advertisementreceived', handler, { once: true }); });
      return { dev, ac, handler, seen };
    });
    try {
      const started = await Promise.allSettled(watches.map((w) => w.dev.watchAdvertisements(w.ac ? { signal: w.ac.signal } : undefined)));
      started.forEach((r, i) => { if (r.status === 'rejected') L('warn', `watchAdvertisements() on ${tagged(watches[i].dev)}: ${errText(r.reason)}`); });
      if (started.every((r) => r.status === 'rejected')) throw started[0].reason;
      let t;
      const timeout = new Promise((resolve) => { t = setTimeout(() => resolve(null), advertTimeoutMs); });
      try {
        return await Promise.race([...watches.filter((w, i) => started[i].status === 'fulfilled').map((w) => w.seen), timeout]);
      } finally { clearTimeout(t); }
    } finally {
      for (const w of watches) {
        w.dev.removeEventListener('advertisementreceived', w.handler);
        try { w.ac?.abort(); } catch { /* ignore */ }
      }
    }
  }

  async function waitForAdvertisement(dev) {
    if (!(await firstAdvertising([dev]))) {
      throw new LinkError(`waiting for an advertisement from ${dev.name || dev.id} timed out after ${advertTimeoutMs} ms`, 'NOT_IN_RANGE', HINTS.RANGE);
    }
  }

  async function gattConnect(dev) {
    const server = await withTimeout(dev.gatt.connect(), connectTimeoutMs, 'gatt.connect()');
    const service = await withTimeout(server.getPrimaryService(SERVICE), connectTimeoutMs, 'getPrimaryService(bundle service)').catch((e) => {
      if (/disconnected/i.test(errText(e))) throw e; // the link dropped (stale pairing), not a missing service
      if (/SecurityError|not allowed to access/.test(errText(e))) throw new LinkError(`Chrome has not granted this page the bundle service on ${dev.name || dev.id}`, 'NOT_GRANTED', HINTS.NOT_GRANTED);
      throw new LinkError(`connected to ${dev.name || dev.id} but its bundle service ${SERVICE} was not found (${errText(e)})`, 'NO_SERVICE', 'is the box running firmware with bundles (D4 or later)? Flash it, then api.device.connect()');
    });
    const get = async (u, what) => {
      try { return await withTimeout(service.getCharacteristic(u), connectTimeoutMs, `getCharacteristic(${what})`); } catch (e) {
        throw new LinkError(`the bundle service has no '${what}' characteristic (${errText(e)})`, 'NO_SERVICE', 'the firmware does not match BLE.md "Bundles": flash a current build');
      }
    };
    return { event: await get(EVENT, 'event'), bundle: await get(BUNDLE, 'bundle'), data: await get(DATA, 'data') };
  }

  /**
   * Subscribe to `bundle` and `data`. Both need an encrypted link with a bond
   * the box holds: on a fresh link the box asks for encryption and the OS
   * encrypts with its bond, or pairs while the box's window is open. Chrome on
   * macOS reports the first refusal as "unknown reason"; a write request to
   * `event` (a List) is what makes the OS pair, so: write one, wait, again.
   */
  async function subscribe(dev, c) {
    let last = null;
    for (let attempt = 1; attempt <= pairAttempts; attempt++) {
      try {
        for (const ch of [c.bundle, c.data]) await gattOp(() => ch.startNotifications());
        bonded = true;
        return;
      } catch (e) {
        if (droppedWhileConnecting || !dev.gatt.connected) throw e;
        if (!isRefusal(e)) throw e;
        last = e;
        if (attempt === pairAttempts) break;
        L('warn', `subscribing was refused (${errText(e)}): writing a request to make the OS pair (attempt ${attempt} of ${pairAttempts})`);
        await gattOp(() => c.event.writeValueWithResponse(LIST_REQUEST)).catch((e2) => L('info', `pairing write: ${errText(e2)}`));
        if (droppedWhileConnecting || !dev.gatt.connected) throw new Error('the link dropped while pairing');
        await sleep(pairRetryMs);
      }
    }
    bonded = false;
    throw new LinkError(`the box refused this computer: it is not bonded with 'nacra' (${pairAttempts} subscriptions refused; the last: ${errText(last)})`, 'NOT_BONDED', HINTS.PAIR);
  }

  /**
   * gatt.connect() and the bundle service on `dev`. `advert` says what to do
   * when Chrome refuses the direct connect as out of range:
   *   'wait'   watch for its advertisement, then connect again (one candidate)
   *   'direct' fail with NOT_IN_RANGE (.direct: the caller watches several)
   *   'seen'   an advertisement was just seen: no wait, a refusal is the box's
   */
  async function openLink(dev, fromReconnect, advert = 'wait') {
    let c;
    try {
      c = await gattConnect(dev);
    } catch (e) {
      const msg = errText(e);
      try { dev.gatt.disconnect(); } catch { /* ignore */ }
      if (e instanceof LinkError) throw e;
      const outOfRange = /no longer in range|not in range|out of range|Connection attempt failed/i.test(msg);
      if (advert === 'seen') {
        if (/Connection attempt failed/i.test(msg)) e.afterAdvert = true;
        throw e;
      }
      if (advert === 'direct' && outOfRange) {
        const err = new LinkError(`${tagged(dev)} refused a direct connect (${msg})`, 'NOT_IN_RANGE', HINTS.RANGE);
        err.direct = true;
        throw err;
      }
      // After a reload Chrome says "no longer in range" until it has seen an
      // advertisement; an advertisement also tells "the box is off" apart from
      // "the box is there and refuses the link" (a stale pairing).
      if (outOfRange || fromReconnect) {
        L('info', `direct connect failed (${msg}); waiting for an advertisement`);
        await waitForAdvertisement(dev);
        L('info', 'advertisement seen; connecting');
        try {
          c = await gattConnect(dev);
        } catch (e2) {
          if (/Connection attempt failed/i.test(errText(e2))) e2.afterAdvert = true;
          throw e2;
        }
      } else {
        throw e;
      }
    }
    for (const [ch, fn] of [[c.bundle, onBundleValue], [c.data, onDataValue]]) {
      ch.removeEventListener?.('characteristicvaluechanged', fn);
      ch.addEventListener('characteristicvaluechanged', fn);
    }
    await subscribe(dev, c);
    chars = c;
    lastError = null; lastHint = null;
    setState('connected');
    L('info', `connected to ${tagged(dev)}; subscribed to bundle and data`);
    storeId(dev.id);
  }

  async function openDevice(dev, { fromReconnect = false, advert = 'wait' } = {}) {
    if (!listening.has(dev)) {
      dev.addEventListener('gattserverdisconnected', onDisconnected);
      listening.add(dev);
    }
    droppedWhileConnecting = false;
    try {
      await openLink(dev, fromReconnect, advert);
    } catch (e) {
      try { dev.gatt.disconnect(); } catch { /* ignore */ }
      if (e.code === 'NOT_GRANTED' || e.code === 'NOT_BONDED' || e.code === 'NO_SERVICE' || e.code === 'NOT_IN_RANGE' || e.code === 'NO_ADVERTISEMENTS') throw e;
      if (droppedWhileConnecting || e.afterAdvert || /failed for unknown reason/i.test(errText(e))) {
        throw new LinkError(`the link dropped while connecting (${errText(e)})`, 'STALE_PAIRING', HINTS.STALE);
      }
      throw new LinkError(`connecting to ${dev.name || dev.id} failed: ${errText(e)}`, 'CONNECT_FAILED', HINTS.RANGE);
    }
  }

  function requireBluetooth() {
    if (!bluetooth) throw new LinkError('Web Bluetooth is not available in this browser', 'NO_BLUETOOTH', HINTS.NO_BLUETOOTH);
  }

  async function run(fn) {
    userDisconnect = false;
    setState('connecting');
    try {
      return await fn();
    } catch (e) {
      fail(e);
      if (state !== 'connected') setState('disconnected');
      throw e;
    }
  }

  /**
   * The person's click on "Choose the box…" (or "Choose another box…"):
   * Chrome's chooser, then connect.
   * requestDevice is called before anything is awaited (a click's user
   * activation lasts about 5 s).
   */
  async function choose() {
    requireBluetooth();
    if (connecting) throw new LinkError('a connection is being made', 'BUSY', 'wait for it (api.device.status())');
    const picking = bluetooth.requestDevice({ filters: [{ name: DEVICE_NAME }], optionalServices: [SERVICE] });
    L('info', 'opened the Bluetooth chooser');
    let dev;
    try { dev = await picking; } catch (e) {
      const why = errText(e);
      const err = /cancel/i.test(why)
        ? new LinkError(`the chooser was closed without picking a device (${why})`, 'CANCELLED', "click 'Choose the box…' (#device-choose) again and pick 'nacra' in Chrome's list (the box must be powered and in range)")
        : /gesture|activation/i.test(why)
          ? new LinkError(`Chrome would not open the chooser: ${why}`, 'NEEDS_CLICK', HINTS.CHOOSE)
          : new LinkError(`the chooser failed: ${why}`, 'CHOOSER_FAILED', HINTS.CHOOSE);
      throw fail(err);
    }
    remembered = true; // the chooser grants the page the box
    if (grantedIds) grantedIds.add(dev.id);
    storeId(dev.id); // the box connect() prefers from now on, also after a reload
    L('info', `chosen: ${tagged(dev)}`);
    if (isConnected() && (dev === device || dev.id === device?.id)) return { deviceName: dev.name, already: true };
    if (device && device !== dev) disconnect();
    device = dev;
    connecting = run(async () => { await openDevice(dev); return { deviceName: dev.name, already: false }; });
    try { return await connecting; } finally { connecting = null; }
  }

  /**
   * Several granted boxes share the name: the remembered id's box by a direct
   * connect, else (none remembered, not granted any more, or refused as out of
   * range) whichever of them advertises first.
   */
  async function openOneOf(candidates) {
    const want = storedId();
    const preferred = want ? candidates.find((d) => d.id === want) : null;
    if (preferred) {
      L('info', `${candidates.length} boxes named '${DEVICE_NAME}' granted; trying the remembered one, ${tagged(preferred)}, directly`);
      device = preferred;
      try {
        await openDevice(preferred, { advert: 'direct' });
        return preferred;
      } catch (e) {
        if (!e.direct) throw e; // the box answered: its error is the one to tell
        device = null;
        L('info', `${errText(e)}; watching all ${candidates.length} for an advertisement`);
      }
    } else {
      L('info', `${candidates.length} boxes named '${DEVICE_NAME}' granted, ${want ? `none of them the remembered …${idTag(want)}` : 'none remembered'}; watching all of them for an advertisement`);
    }
    const winner = await firstAdvertising(candidates);
    if (!winner) {
      throw new LinkError(`none of the ${candidates.length} remembered boxes named '${DEVICE_NAME}' advertised in ${advertTimeoutMs / 1000} s`, 'NOT_IN_RANGE', HINTS.RANGE);
    }
    L('info', `${tagged(winner)} advertised first of ${candidates.length}: connecting to it`);
    device = winner;
    await openDevice(winner, { advert: 'seen' });
    return winner;
  }

  /** The remembered device, never the chooser (script calls). */
  async function connect() {
    if (reconnecting) { L('info', 'connect(): a reconnect is in progress; waiting for it'); await reconnecting; }
    if (isConnected()) return { deviceName: device.name, already: true };
    if (connecting) return connecting;
    connecting = run(async () => {
      requireBluetooth();
      if (device) {
        L('info', `connecting to ${tagged(device)}`);
        await openDevice(device);
        return { deviceName: device.name, already: false };
      }
      if (typeof bluetooth.getDevices !== 'function') {
        throw new LinkError("this Chrome cannot list remembered devices (no navigator.bluetooth.getDevices), so only the chooser can connect", 'NO_GET_DEVICES', `${HINTS.FLAGS}; meanwhile ${HINTS.CHOOSE}`);
      }
      const devs = await bluetooth.getDevices();
      L('info', `getDevices(): ${devs.length ? devs.map((d) => `${d.name || '?'} (…${idTag(d.id)})`).join(', ') : 'none permitted'}`);
      const candidates = devs.filter((d) => d.name === DEVICE_NAME);
      remembered = candidates.length > 0;
      grantedIds = new Set(candidates.map((d) => d.id));
      if (!candidates.length) throw new LinkError(`no remembered '${DEVICE_NAME}': this page has never been given the box`, 'NEEDS_CHOOSER', HINTS.CHOOSE);
      if (candidates.length === 1) {
        device = candidates[0];
        L('info', `connecting to ${tagged(device)}`);
        await openDevice(device);
        return { deviceName: device.name, already: false };
      }
      const dev = await openOneOf(candidates);
      return { deviceName: dev.name, already: false };
    });
    try { return await connecting; } finally { connecting = null; }
  }

  function disconnect() {
    userDisconnect = true;
    chars = null;
    try { if (device?.gatt?.connected) device.gatt.disconnect(); } catch { /* ignore */ }
    setState('disconnected');
  }

  function isConnected() { return state === 'connected' && !!chars && !!device?.gatt?.connected; }

  function requireLink() {
    if (!isConnected()) throw new LinkError(`not connected to the box (${state})`, 'NOT_CONNECTED', HINTS.NOT_CONNECTED);
    return chars;
  }

  /** A request to `event` (write with response: the box answers on `bundle`). */
  function write(bytes) {
    const c = requireLink();
    return gattOp(() => c.event.writeValueWithResponse(bytes));
  }

  /** Bundle bytes to `data` (write without response). */
  function writeData(bytes) {
    const c = requireLink();
    return gattOp(() => c.data.writeValueWithoutResponse(bytes));
  }

  /** Resolves true once connected again (an auto-reconnect), false if it gives up or ms pass. */
  async function waitConnected(ms) {
    if (isConnected()) return true;
    if (!reconnecting) return false;
    await withTimeout(reconnecting, ms, 'reconnecting').catch(() => {});
    return isConnected();
  }

  /**
   * Whether Chrome remembers the box for this page (getDevices(): the
   * permissions only, nothing goes over the air). → true | false | null
   * (null: this Chrome has no getDevices). Tells the state listeners.
   */
  async function lookRemembered() {
    if (!bluetooth || typeof bluetooth.getDevices !== 'function') return remembered;
    if (device) { remembered = true; return true; }
    let r = remembered;
    let ids = grantedIds;
    try {
      ids = new Set((await bluetooth.getDevices()).filter((d) => d.name === DEVICE_NAME).map((d) => d.id));
      r = ids.size > 0;
    } catch (e) { L('warn', `getDevices(): ${errText(e)}`); }
    const before = candidatesCount();
    grantedIds = ids;
    if (r !== remembered || candidatesCount() !== before) { remembered = r; emit(stateListeners, state); }
    return remembered;
  }

  /** How many boxes named DEVICE_NAME Chrome has granted this page (null: not looked yet). */
  function candidatesCount() { return grantedIds ? grantedIds.size : null; }

  /** The box connect() works with: the one in hand, else the remembered id if Chrome still grants it. */
  function currentId() {
    if (device) return device.id ?? null;
    const id = storedId();
    if (!id) return null;
    return !grantedIds || grantedIds.has(id) ? id : null;
  }

  const on = (set) => (cb) => { set.add(cb); return () => set.delete(cb); };

  return {
    choose, connect, disconnect, write, writeData, waitConnected, isConnected, lookRemembered,
    onBundle: on(bundleListeners), onData: on(dataListeners), onDrop: on(dropListeners), onState: on(stateListeners),
    info: () => ({ state, deviceName: device?.name ?? null, deviceId: currentId(), candidates: candidatesCount(), bonded, remembered, lastError, hint: lastHint, reconnects, bluetooth: !!bluetooth, reconnecting: !!reconnecting }),
    get state() { return state; },
  };
}
