/* @ts-self-types="./engine_wasm.d.ts" */

/**
 * A compilation: [`compile`]'s result.
 */
export class Compiled {
    static __wrap(ptr) {
        const obj = Object.create(Compiled.prototype);
        obj.__wbg_ptr = ptr;
        CompiledFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        CompiledFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_compiled_free(ptr, 0);
    }
    /**
     * The program (`engine.bin`); empty if it didn't compile.
     * @returns {Uint8Array}
     */
    bytes() {
        const ret = wasm.compiled_bytes(this.__wbg_ptr);
        var v1 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v1;
    }
    /**
     * `{ok: true, memory, interface_hash, imports, program_bytes}` or
     * `{ok: false, diagnostics}` (or `{ok: false, error}`).
     * @returns {string}
     */
    json() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.compiled_json(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Whether it compiled.
     * @returns {boolean}
     */
    ok() {
        const ret = wasm.compiled_ok(this.__wbg_ptr);
        return ret !== 0;
    }
}
if (Symbol.dispose) Compiled.prototype[Symbol.dispose] = Compiled.prototype.free;

/**
 * A host and what runs it. Fields drop in order: the timer task and the
 * subscription borrow the host; the host borrows the memory, the bundles
 * and the track.
 */
export class HostHandle {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        HostHandleFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_hosthandle_free(ptr, 0);
    }
    /**
     * Moves the clock `ms` forward, stopping at each timer the host has set
     * on the way so it fires at its time: `{ok: true, now, fired}` (`now`
     * the device time after, epoch ms; `fired` how many timers came due).
     * @param {number} ms
     * @returns {string}
     */
    advance(ms) {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.hosthandle_advance(this.__wbg_ptr, ms);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * A client's message, a postcard `HostEvent` (as from the websocket):
     * `{ok: true}` or `{ok: false, refused: "Undecodable" | "NotInstalled"
     * | "NotActive" | "Engine" | "Ble"}`.
     * @param {Uint8Array} bytes
     * @returns {string}
     */
    event(bytes) {
        let deferred2_0;
        let deferred2_1;
        try {
            const ptr0 = passArray8ToWasm0(bytes, wasm.__wbindgen_malloc);
            const len0 = WASM_VECTOR_LEN;
            const ret = wasm.hosthandle_event(this.__wbg_ptr, ptr0, len0);
            deferred2_0 = ret[0];
            deferred2_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
        }
    }
    /**
     * A GPS update, `{t?, lat?, lon?, speed?, heading?}` (`t` epoch ms, the
     * receiver's time; `lat` and `lon` together, `speed` (knots) and
     * `heading` (degrees true) together): `{ok: true}` or `{ok: false,
     * error}`.
     * @param {string} update
     * @returns {string}
     */
    gps(update) {
        let deferred2_0;
        let deferred2_1;
        try {
            const ptr0 = passStringToWasm0(update, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ret = wasm.hosthandle_gps(this.__wbg_ptr, ptr0, len0);
            deferred2_0 = ret[0];
            deferred2_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
        }
    }
    /**
     * The current state's envelope, stamped now: what a websocket sends
     * at each heartbeat. Empty if the host did not start.
     * @returns {Uint8Array}
     */
    heartbeat() {
        const ret = wasm.hosthandle_heartbeat(this.__wbg_ptr);
        var v1 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v1;
    }
    /**
     * What the constructor made of each bundle: `[{bundle: "choose" |
     * index, ok: true, id, name} | {bundle, ok: false, error}]`.
     * @returns {string}
     */
    loaded() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.hosthandle_loaded(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * The evaluations the host made since the last call, oldest first, as
     * JSON: `[{t, cause: "start" | "gps" | "wake" | "client", event?,
     * accepted, changed, error?: {code, pc, span: {start, end} | null,
     * help}, instructions?, stack_high?, state, view_hex, wake,
     * unobserved?}]`. `t` the device time it was made at; `event` the
     * engine's event (not for "client": the client's bytes are the
     * caller's); `changed` whether the published value's bytes changed
     * (an envelope went out); `error.code` an `R-…` run-time error (`pc`
     * the instruction, `span` its bytes in engine.expr), or "undecodable"
     * (a client's bytes that are not one of the engine's events) or
     * "output" (an output that can't be published); `state` (plain JSON,
     * null if it can't be read), `view_hex` and `wake` as after it.
     * `unobserved: n`: n evaluations before this one were made between two
     * looks (not expected; this one is the last of them).
     * @returns {string}
     */
    log() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.hosthandle_log(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * [`with_track_capacity`](Self::with_track_capacity), from JS:
     * `new HostHandle(bundles, choose, trackCapacity?)` (default the C6's,
     * [`DEVICE_FIXES`]).
     * @param {Uint8Array[]} bundles
     * @param {Uint8Array | null} [choose]
     * @param {number | null} [track_capacity]
     */
    constructor(bundles, choose, track_capacity) {
        const ptr0 = passArrayJsValueToWasm0(bundles, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.hosthandle_new(ptr0, len0, isLikeNone(choose) ? 0 : addToExternrefTable0(choose), isLikeNone(track_capacity) ? Number.MAX_SAFE_INTEGER : (track_capacity) >>> 0);
        this.__wbg_ptr = ret;
        HostHandleFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * NMEA text: each RMC sentence in it with a valid checksum and a valid
     * fix is a GPS update, as from the receiver: `{ok: true, updates: n}`.
     * @param {string} text
     * @returns {string}
     */
    nmea(text) {
        let deferred2_0;
        let deferred2_1;
        try {
            const ptr0 = passStringToWasm0(text, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ret = wasm.hosthandle_nmea(this.__wbg_ptr, ptr0, len0);
            deferred2_0 = ret[0];
            deferred2_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
        }
    }
    /**
     * The device time, epoch ms (uptime until a GPS time sets the clock).
     * @returns {number}
     */
    now() {
        const ret = wasm.hosthandle_now(this.__wbg_ptr);
        return ret;
    }
    /**
     * [`take_envelopes`](Self::take_envelopes), from JS.
     * @returns {Uint8Array[]}
     */
    poll() {
        const ret = wasm.hosthandle_poll(this.__wbg_ptr);
        var v1 = getArrayJsValueFromWasm0(ret[0], ret[1]);
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * `{active, name, ui: {web, watch}, view_hex, wake, now, running,
     * evaluations, track, track_capacity}`: the active package (0 Choose),
     * its UI hashes (hex), its view (bitwire, hex; empty for Choose), when
     * its wake is due and the time now (device clock, epoch ms; `wake` null
     * without one); whether its engine runs (false for Choose, or an engine
     * whose first evaluation failed) and how many evaluations it has made;
     * the fixes in `gps` and the most it holds.
     * @returns {string}
     */
    state() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.hosthandle_state(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * The active engine's state, decoded from the leaves it keeps with the
     * state schema in its bundle's `source.json`: `{ok: true, state}`
     * (`state` null before the first evaluation, or for Choose) or `{ok:
     * false, error}` (no engine running, or no `source.json`).
     * @returns {string}
     */
    state_json() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.hosthandle_state_json(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
}
if (Symbol.dispose) HostHandle.prototype[Symbol.dispose] = HostHandle.prototype.free;

/**
 * A bundle: [`pack`]'s result.
 */
export class Packed {
    static __wrap(ptr) {
        const obj = Object.create(Packed.prototype);
        obj.__wbg_ptr = ptr;
        PackedFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        PackedFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_packed_free(ptr, 0);
    }
    /**
     * The bundle; empty if it wasn't made.
     * @returns {Uint8Array}
     */
    bytes() {
        const ret = wasm.packed_bytes(this.__wbg_ptr);
        var v1 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v1;
    }
    /**
     * `{ok: true, manifest: {id, name, version, abi, content_hash, ui:
     * {web, watch}, watch_len}, len}` or `{ok: false, error}`.
     * @returns {string}
     */
    json() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.packed_json(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {boolean}
     */
    ok() {
        const ret = wasm.packed_ok(this.__wbg_ptr);
        return ret !== 0;
    }
}
if (Symbol.dispose) Packed.prototype[Symbol.dispose] = Packed.prototype.free;

/**
 * The sailing builtins' declarations, as the compiler takes them.
 * @returns {string}
 */
export function builtins() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.builtins();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Compiles `source` against `interface` (JSON); `opts` is JSON
 * `{limit_bytes?, code_in_ram?}` or empty. The JSON on success:
 * `{ok: true, memory: {stack_slots, locals, output_slots, slot_bytes,
 * program_bytes, total_bytes}, interface_hash (hex), imports (the
 * builtins' signatures), program_bytes}`; diagnostics are
 * `{rule, severity, span: {start, end: {line, col, offset}} | null,
 * message, help}`.
 * @param {string} source
 * @param {string} _interface
 * @param {string} opts
 * @returns {Compiled}
 */
export function compile(source, _interface, opts) {
    const ptr0 = passStringToWasm0(source, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passStringToWasm0(_interface, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len1 = WASM_VECTOR_LEN;
    const ptr2 = passStringToWasm0(opts, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len2 = WASM_VECTOR_LEN;
    const ret = wasm.compile(ptr0, len0, ptr1, len1, ptr2, len2);
    return Compiled.__wrap(ret);
}

/**
 * A program as text, one instruction per line: `{ok: true, text}` or
 * `{ok: false, error}`.
 * @param {Uint8Array} bytes
 * @returns {string}
 */
export function disassemble(bytes) {
    let deferred2_0;
    let deferred2_1;
    try {
        const ptr0 = passArray8ToWasm0(bytes, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.disassemble(ptr0, len0);
        deferred2_0 = ret[0];
        deferred2_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
    }
}

/**
 * An engine's interface from its schemas (JSON text; `view` and `event`
 * optional): `{ok: true, interface}`, `{ok: false, diagnostics}` from the
 * profile, or `{ok: false, error}` for a schema that isn't JSON. (In JS
 * `engine_interface`: `interface` is a reserved word.)
 * @param {string} state
 * @param {string | null} [view]
 * @param {string | null} [event]
 * @returns {string}
 */
export function engine_interface(state, view, event) {
    let deferred4_0;
    let deferred4_1;
    try {
        const ptr0 = passStringToWasm0(state, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        var ptr1 = isLikeNone(view) ? 0 : passStringToWasm0(view, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        var len1 = WASM_VECTOR_LEN;
        var ptr2 = isLikeNone(event) ? 0 : passStringToWasm0(event, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        var len2 = WASM_VECTOR_LEN;
        const ret = wasm.engine_interface(ptr0, len0, ptr1, len1, ptr2, len2);
        deferred4_0 = ret[0];
        deferred4_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred4_0, deferred4_1, 1);
    }
}

/**
 * The device tables of a program (engine_profile::layout): `interface`
 * (JSON) as it was compiled against, `program` the bytecode, `view_desc`
 * the bitwire descriptor of the published value (the view, or the state
 * without one), `event_desc` the author's events' (if any). `{ok: true,
 * layout: {<field>: u16, …}, interface_hash, view_map: [{leaf, span,
 * aux}], event_map: […] | null, enums: [string] | null, bytes: {layout,
 * view_map, event_map, enums} (base64; null where absent)}` or `{ok:
 * false, error}`.
 * @param {string} _interface
 * @param {Uint8Array} program
 * @param {Uint8Array} view_desc
 * @param {Uint8Array | null} [event_desc]
 * @returns {string}
 */
export function layout(_interface, program, view_desc, event_desc) {
    let deferred5_0;
    let deferred5_1;
    try {
        const ptr0 = passStringToWasm0(_interface, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passArray8ToWasm0(program, wasm.__wbindgen_malloc);
        const len1 = WASM_VECTOR_LEN;
        const ptr2 = passArray8ToWasm0(view_desc, wasm.__wbindgen_malloc);
        const len2 = WASM_VECTOR_LEN;
        var ptr3 = isLikeNone(event_desc) ? 0 : passArray8ToWasm0(event_desc, wasm.__wbindgen_malloc);
        var len3 = WASM_VECTOR_LEN;
        const ret = wasm.layout(ptr0, len0, ptr1, len1, ptr2, len2, ptr3, len3);
        deferred5_0 = ret[0];
        deferred5_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred5_0, deferred5_1, 1);
    }
}

/**
 * The bundle of a package (`extreme-pack`'s): `manifest` is JSON `{name,
 * version}`, `files` `[{name, bytes}]` by their path in the bundle
 * (`engine.bin`, `layout.bin`, `view.desc`, `view.map`, `event.desc`,
 * `event.map`, `enums.bin`, `source.json`, `web/<name>`, `watch.js`; not
 * `manifest.bin`, which is made here: id, ABI, content and UI hashes).
 * @param {string} manifest
 * @param {Array<any>} files
 * @returns {Packed}
 */
export function pack(manifest, files) {
    const ptr0 = passStringToWasm0(manifest, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.pack(ptr0, len0, files);
    return Packed.__wrap(ret);
}

/**
 * A package's id from its name.
 * @param {string} name
 * @returns {number}
 */
export function package_id(name) {
    const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.package_id(ptr0, len0);
    return ret >>> 0;
}

/**
 * The sample runs `samples` (JSON, lang/SPEC.md §5.3: `{runs: [{inputs,
 * chain?}]}`) of the program `bytes` compiled against `interface`, on the
 * VM with the sailing builtins: `compiler::samples::run_samples`'s report
 * `{ok, runs: [{run, ok, output, tagged, stack_high, steps, chained_from?}
 * | {run, ok: false, error: "R-…", span: {start, end} | null, help} |
 * {run, ok: false, invalid}]}`, or `{ok: false, error}`.
 * @param {Uint8Array} bytes
 * @param {string} _interface
 * @param {string} samples
 * @returns {string}
 */
export function run_samples(bytes, _interface, samples) {
    let deferred4_0;
    let deferred4_1;
    try {
        const ptr0 = passArray8ToWasm0(bytes, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passStringToWasm0(_interface, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        const ptr2 = passStringToWasm0(samples, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len2 = WASM_VECTOR_LEN;
        const ret = wasm.run_samples(ptr0, len0, ptr1, len1, ptr2, len2);
        deferred4_0 = ret[0];
        deferred4_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred4_0, deferred4_1, 1);
    }
}

export function start() {
    wasm.start();
}

/**
 * A bundle's contents: `{ok: true, manifest, files: [{name, len}],
 * source: <source.json's text> | null}` or `{ok: false, error}`.
 * @param {Uint8Array} bytes
 * @returns {string}
 */
export function unpack(bytes) {
    let deferred2_0;
    let deferred2_1;
    try {
        const ptr0 = passArray8ToWasm0(bytes, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.unpack(ptr0, len0);
        deferred2_0 = ret[0];
        deferred2_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
    }
}

/**
 * `{engine_wasm, lang_spec, bytecode_version, source_sha}`: this module's
 * version, the language spec's draft, the bytecode format it emits, and
 * the monorepo commit it was built from (null if not given at build time,
 * `STUDIO_SOURCE_SHA`).
 * @returns {string}
 */
export function version() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.version();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * The watch bundle's hash (`ui.watch`, hex); all zeros for none.
 * @param {Uint8Array} bytes
 * @returns {string}
 */
export function watch_hash(bytes) {
    let deferred2_0;
    let deferred2_1;
    try {
        const ptr0 = passArray8ToWasm0(bytes, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.watch_hash(ptr0, len0);
        deferred2_0 = ret[0];
        deferred2_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
    }
}

/**
 * The Kindle page's hash (`ui.web`, hex) of `files` `[{name, bytes}]`,
 * named as in `web/` (without the prefix); `""` if `files` isn't that.
 * @param {Array<any>} files
 * @returns {string}
 */
export function web_hash(files) {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.web_hash(files);
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg___wbindgen_string_get_0380ccaa2f57f0d9: function(arg0, arg1) {
            const obj = arg1;
            const ret = typeof(obj) === 'string' ? obj : undefined;
            var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            var len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg___wbindgen_throw_41e9ee4f547fc59a: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbg_error_757e9472f8410341: function(arg0, arg1) {
            let deferred0_0;
            let deferred0_1;
            try {
                deferred0_0 = arg0;
                deferred0_1 = arg1;
                console.error(getStringFromWasm0(arg0, arg1));
            } finally {
                wasm.__wbindgen_free(deferred0_0, deferred0_1, 1);
            }
        },
        __wbg_get_31af05bd4842a84f: function() { return handleError(function (arg0, arg1) {
            const ret = Reflect.get(arg0, arg1);
            return ret;
        }, arguments); },
        __wbg_get_unchecked_288889d017702237: function(arg0, arg1) {
            const ret = arg0[arg1 >>> 0];
            return ret;
        },
        __wbg_instanceof_Uint8Array_828cef2aaacafc31: function(arg0) {
            let result;
            try {
                result = arg0 instanceof Uint8Array;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_length_7f3c00c40364105e: function(arg0) {
            const ret = arg0.length;
            return ret;
        },
        __wbg_length_d4bdea10311bd9cf: function(arg0) {
            const ret = arg0.length;
            return ret;
        },
        __wbg_new_227d7c05414eb861: function() {
            const ret = new Error();
            return ret;
        },
        __wbg_new_from_slice_9a868026ffa4208a: function(arg0, arg1) {
            const ret = new Uint8Array(getArrayU8FromWasm0(arg0, arg1));
            return ret;
        },
        __wbg_prototypesetcall_bc27214492979395: function(arg0, arg1, arg2) {
            Uint8Array.prototype.set.call(getArrayU8FromWasm0(arg0, arg1), arg2);
        },
        __wbg_stack_3b0d974bbf31e44f: function(arg0, arg1) {
            const ret = arg1.stack;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbindgen_generic_0000000000000001: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            const offset = table.grow(4);
            table.set(0, undefined);
            table.set(offset + 0, undefined);
            table.set(offset + 1, null);
            table.set(offset + 2, true);
            table.set(offset + 3, false);
        },
    };
    return {
        __proto__: null,
        "./engine_wasm_bg.js": import0,
    };
}

const CompiledFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_compiled_free(ptr, 1));
const HostHandleFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_hosthandle_free(ptr, 1));
const PackedFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_packed_free(ptr, 1));

function addToExternrefTable0(obj) {
    const idx = wasm.__externref_table_alloc();
    wasm.__wbindgen_externrefs.set(idx, obj);
    return idx;
}

function getArrayJsValueFromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    const mem = getDataViewMemory0();
    const result = [];
    for (let i = ptr; i < ptr + 4 * len; i += 4) {
        result.push(wasm.__wbindgen_externrefs.get(mem.getUint32(i, true)));
    }
    wasm.__externref_drop_slice(ptr, len);
    return result;
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
        cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
}

function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function handleError(f, args) {
    try {
        return f.apply(this, args);
    } catch (e) {
        const idx = addToExternrefTable0(e);
        wasm.__wbindgen_exn_store(idx);
    }
}

function isLikeNone(x) {
    return x === undefined || x === null;
}

function passArray8ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 1, 1) >>> 0;
    getUint8ArrayMemory0().set(arg, ptr / 1);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passArrayJsValueToWasm0(array, malloc) {
    const ptr = malloc(array.length * 4, 4) >>> 0;
    for (let i = 0; i < array.length; i++) {
        const add = addToExternrefTable0(array[i]);
        getDataViewMemory0().setUint32(ptr + 4 * i, add, true);
    }
    WASM_VECTOR_LEN = array.length;
    return ptr;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    cachedDataViewMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    wasm.__wbindgen_start();
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (!module.ok) {
            throw new Error(`failed to fetch Wasm: ${module.status} ${module.statusText} fetching '${module.url}'`);
        }

        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('engine_wasm_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };
