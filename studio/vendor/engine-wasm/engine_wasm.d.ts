/* tslint:disable */
/* eslint-disable */

/**
 * A compilation: [`compile`]'s result.
 */
export class Compiled {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * The program (`engine.bin`); empty if it didn't compile.
     */
    bytes(): Uint8Array;
    /**
     * `{ok: true, memory, interface_hash, imports, program_bytes}` or
     * `{ok: false, diagnostics}` (or `{ok: false, error}`).
     */
    json(): string;
    /**
     * Whether it compiled.
     */
    ok(): boolean;
}

/**
 * A host and what runs it. Fields drop in order: the timer task and the
 * subscription borrow the host; the host borrows the memory and bundles.
 */
export class HostHandle {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Moves the clock `ms` forward, stopping at each timer the host has set
     * on the way so it fires at its time: `{ok: true, now, fired}` (`now`
     * the device time after, epoch ms; `fired` how many timers came due).
     */
    advance(ms: number): string;
    /**
     * A client's message, a postcard `HostEvent` (as from the websocket):
     * `{ok: true}` or `{ok: false, refused: "Undecodable" | "NotInstalled"
     * | "NotActive" | "Engine" | "Ble"}`.
     */
    event(bytes: Uint8Array): string;
    /**
     * A GPS update, `{t?, lat?, lon?, speed?, heading?}` (`t` epoch ms, the
     * receiver's time; `lat` and `lon` together, `speed` (knots) and
     * `heading` (degrees true) together): `{ok: true}` or `{ok: false,
     * error}`.
     */
    gps(update: string): string;
    /**
     * The current state's envelope, stamped now: what a websocket sends
     * at each heartbeat. Empty if the host did not start.
     */
    heartbeat(): Uint8Array;
    /**
     * What the constructor made of each bundle: `[{bundle: "choose" |
     * index, ok: true, id, name} | {bundle, ok: false, error}]`.
     */
    loaded(): string;
    /**
     * [`from_bytes`](Self::from_bytes), from JS.
     */
    constructor(bundles: Uint8Array[], choose?: Uint8Array | null);
    /**
     * NMEA text: each RMC sentence in it with a valid checksum and a valid
     * fix is a GPS update, as from the receiver: `{ok: true, updates: n}`.
     */
    nmea(text: string): string;
    /**
     * The device time, epoch ms (uptime until a GPS time sets the clock).
     */
    now(): number;
    /**
     * [`take_envelopes`](Self::take_envelopes), from JS.
     */
    poll(): Uint8Array[];
    /**
     * `{active, name, ui: {web, watch}, view_hex, wake, now}`: the active
     * package (0 Choose), its UI hashes (hex), its view (bitwire, hex;
     * empty for Choose), when its wake is due and the time now (device
     * clock, epoch ms; `wake` null without one).
     */
    state(): string;
}

/**
 * A bundle: [`pack`]'s result.
 */
export class Packed {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * The bundle; empty if it wasn't made.
     */
    bytes(): Uint8Array;
    /**
     * `{ok: true, manifest: {id, name, version, abi, content_hash, ui:
     * {web, watch}, watch_len}, len}` or `{ok: false, error}`.
     */
    json(): string;
    ok(): boolean;
}

/**
 * The sailing builtins' declarations, as the compiler takes them.
 */
export function builtins(): string;

/**
 * Compiles `source` against `interface` (JSON); `opts` is JSON
 * `{limit_bytes?, code_in_ram?}` or empty. The JSON on success:
 * `{ok: true, memory: {stack_slots, locals, output_slots, slot_bytes,
 * program_bytes, total_bytes}, interface_hash (hex), imports (the
 * builtins' signatures), program_bytes}`; diagnostics are
 * `{rule, severity, span: {start, end: {line, col, offset}} | null,
 * message, help}`.
 */
export function compile(source: string, _interface: string, opts: string): Compiled;

/**
 * A program as text, one instruction per line: `{ok: true, text}` or
 * `{ok: false, error}`.
 */
export function disassemble(bytes: Uint8Array): string;

/**
 * An engine's interface from its schemas (JSON text; `view` and `event`
 * optional): `{ok: true, interface}`, `{ok: false, diagnostics}` from the
 * profile, or `{ok: false, error}` for a schema that isn't JSON. (In JS
 * `engine_interface`: `interface` is a reserved word.)
 */
export function engine_interface(state: string, view?: string | null, event?: string | null): string;

/**
 * The device tables of a program (engine_profile::layout): `interface`
 * (JSON) as it was compiled against, `program` the bytecode, `view_desc`
 * the bitwire descriptor of the published value (the view, or the state
 * without one), `event_desc` the author's events' (if any). `{ok: true,
 * layout: {<field>: u16, …}, interface_hash, view_map: [{leaf, span,
 * aux}], event_map: […] | null, enums: [string] | null, bytes: {layout,
 * view_map, event_map, enums} (base64; null where absent)}` or `{ok:
 * false, error}`.
 */
export function layout(_interface: string, program: Uint8Array, view_desc: Uint8Array, event_desc?: Uint8Array | null): string;

/**
 * The bundle of a package (`extreme-pack`'s): `manifest` is JSON `{name,
 * version}`, `files` `[{name, bytes}]` by their path in the bundle
 * (`engine.bin`, `layout.bin`, `view.desc`, `view.map`, `event.desc`,
 * `event.map`, `enums.bin`, `source.json`, `web/<name>`, `watch.js`; not
 * `manifest.bin`, which is made here: id, ABI, content and UI hashes).
 */
export function pack(manifest: string, files: Array<any>): Packed;

/**
 * A package's id from its name.
 */
export function package_id(name: string): number;

/**
 * The sample runs `samples` (JSON, lang/SPEC.md §5.3: `{runs: [{inputs,
 * chain?}]}`) of the program `bytes` compiled against `interface`, on the
 * VM with the sailing builtins: `compiler::samples::run_samples`'s report
 * `{ok, runs: [{run, ok, output, tagged, stack_high, steps, chained_from?}
 * | {run, ok: false, error: "R-…", span: {start, end} | null, help} |
 * {run, ok: false, invalid}]}`, or `{ok: false, error}`.
 */
export function run_samples(bytes: Uint8Array, _interface: string, samples: string): string;

export function start(): void;

/**
 * A bundle's contents: `{ok: true, manifest, files: [{name, len}],
 * source: <source.json's text> | null}` or `{ok: false, error}`.
 */
export function unpack(bytes: Uint8Array): string;

/**
 * `{engine_wasm, lang_spec, bytecode_version, source_sha}`: this module's
 * version, the language spec's draft, the bytecode format it emits, and
 * the monorepo commit it was built from (null if not given at build time,
 * `STUDIO_SOURCE_SHA`).
 */
export function version(): string;

/**
 * The watch bundle's hash (`ui.watch`, hex); all zeros for none.
 */
export function watch_hash(bytes: Uint8Array): string;

/**
 * The Kindle page's hash (`ui.web`, hex) of `files` `[{name, bytes}]`,
 * named as in `web/` (without the prefix); `""` if `files` isn't that.
 */
export function web_hash(files: Array<any>): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly _critical_section_1_0_acquire: () => number;
    readonly _critical_section_1_0_release: (a: number) => void;
    readonly __wbg_compiled_free: (a: number, b: number) => void;
    readonly __wbg_hosthandle_free: (a: number, b: number) => void;
    readonly _embassy_time_now: () => bigint;
    readonly _embassy_time_schedule_wake: (a: bigint, b: number) => void;
    readonly __wbg_packed_free: (a: number, b: number) => void;
    readonly builtins: () => [number, number];
    readonly compile: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly compiled_bytes: (a: number) => [number, number];
    readonly compiled_json: (a: number) => [number, number];
    readonly compiled_ok: (a: number) => number;
    readonly disassemble: (a: number, b: number) => [number, number];
    readonly engine_interface: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number];
    readonly hosthandle_advance: (a: number, b: number) => [number, number];
    readonly hosthandle_event: (a: number, b: number, c: number) => [number, number];
    readonly hosthandle_gps: (a: number, b: number, c: number) => [number, number];
    readonly hosthandle_heartbeat: (a: number) => [number, number];
    readonly hosthandle_loaded: (a: number) => [number, number];
    readonly hosthandle_new: (a: number, b: number, c: number) => number;
    readonly hosthandle_nmea: (a: number, b: number, c: number) => [number, number];
    readonly hosthandle_now: (a: number) => number;
    readonly hosthandle_poll: (a: number) => [number, number];
    readonly hosthandle_state: (a: number) => [number, number];
    readonly layout: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => [number, number];
    readonly pack: (a: number, b: number, c: any) => number;
    readonly package_id: (a: number, b: number) => number;
    readonly packed_bytes: (a: number) => [number, number];
    readonly packed_json: (a: number) => [number, number];
    readonly packed_ok: (a: number) => number;
    readonly run_samples: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number];
    readonly start: () => void;
    readonly unpack: (a: number, b: number) => [number, number];
    readonly version: () => [number, number];
    readonly watch_hash: (a: number, b: number) => [number, number];
    readonly web_hash: (a: any) => [number, number];
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __externref_drop_slice: (a: number, b: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
