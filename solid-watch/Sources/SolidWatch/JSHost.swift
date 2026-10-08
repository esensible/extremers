import CSolidHost
import CryptoKit
import Foundation

/// Why a bundle call failed. Show it to the user: a broken bundle should be
/// visible, not a blank screen.
public enum JSHostError: Error, CustomStringConvertible, Equatable {
    /// The bundle's SHA-256 is not the one the app expected.
    case hashMismatch(expected: String, actual: String)
    /// The bundle threw: the error's message, and the JS stack if any.
    case exception(message: String, stack: String)
    /// A call ran past `Options.callTimeout` and was interrupted.
    case timeout(String)
    /// The JS heap hit `Options.memoryLimit`.
    case outOfMemory(String)
    /// `mount` / `onState` / `dispatch` out of order.
    case notReady(String)
    /// The engine could not be created, or a value could not cross over.
    case engine(String)

    public var description: String {
        switch self {
        case .hashMismatch(let e, let a): return "bundle hash mismatch: expected \(e), got \(a)"
        case .exception(let m, let s): return s.isEmpty ? m : "\(m)\n\(s)"
        case .timeout(let m): return "timed out: \(m)"
        case .outOfMemory(let m): return "out of memory: \(m)"
        case .notReady(let m): return m
        case .engine(let m): return "engine: \(m)"
        }
    }

    /// One-line title for an error screen.
    public var title: String {
        switch self {
        case .hashMismatch: return "Bundle hash mismatch"
        case .exception: return "Script error"
        case .timeout: return "Script timed out"
        case .outOfMemory: return "Script out of memory"
        case .notReady: return "Not ready"
        case .engine: return "Engine error"
        }
    }
}

/// Runs one bundle: a QuickJS runtime + context whose `globalThis.__host`
/// applies mutations to a `ShadowTree`.
///
/// Bundle contract: the bundle sets `globalThis.__app = { mount(rootTag,
/// {width, height}), onState(state), dispatch(tag, name, payload) }`.
///
/// Not thread-safe: use it from one thread (the main thread in an app).
public final class JSHost {
    public struct Options {
        /// JS heap limit (bytes).
        public var memoryLimit: Int = 24 << 20
        /// JS stack limit (bytes).
        public var maxStackSize: Int = 256 << 10
        /// Wall-clock limit for one call into JS (load, mount, onState,
        /// dispatch), so a runaway bundle cannot hang the UI.
        public var callTimeout: Duration = .milliseconds(1000)
        public init() {}
    }

    public let tree: ShadowTree
    public let options: Options
    /// `console.*` and `__host.log` output.
    public var onLog: ((String) -> Void)?
    public private(set) var isLoaded = false
    public private(set) var rootTag: Int32?
    /// SHA-256 (lowercase hex) of the loaded bundle.
    public private(set) var bundleSHA256: String?
    /// Time spent inside JS calls (including the host mutations they make).
    public private(set) var jsTime: Duration = .zero

    /// The mutation log while recording (README: Golden mutation logs): one
    /// `JSON.stringify([op, ...args])` line per `__host` call, plus the
    /// markers added with `recordMarker`.
    public private(set) var log: [String] = []
    public var isRecording = false {
        didSet { sw_engine_set_recording(engine, isRecording ? 1 : 0) }
    }

    private var engine: OpaquePointer?
    private let box: Box
    private var errBuf = [CChar](repeating: 0, count: 8192)

    public static var quickJSVersion: String { String(cString: sw_quickjs_version()) }

    public static func sha256Hex(_ data: Data) -> String {
        SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    public init(tree: ShadowTree, options: Options = Options()) {
        self.tree = tree
        self.options = options
        box = Box()
        box.host = self
        var cb = sw_host_callbacks()
        cb.user = Unmanaged.passUnretained(box).toOpaque()
        cb.log = { user, msg, len in
            Box.get(user).host?.onLog?(str(msg, len))
        }
        cb.create_node = { user, type, len in
            Box.get(user).run { try $0.tree.createNode(str(type, len)) } ?? -1
        }
        cb.create_text = { user, text, len in
            Box.get(user).run { $0.tree.createText(str(text, len)) } ?? -1
        }
        cb.set_prop = { user, tag, key, keylen, value, valuelen in
            Box.get(user).status { h in
                let v: JSData
                do { v = try JSDataCodec.decode(UnsafeRawBufferPointer(start: value, count: valuelen)) } catch {
                    throw MutationError("setProp: undecodable value")
                }
                try h.tree.setProp(tag, str(key, keylen), v.propValue)
            }
        }
        cb.insert = { user, p, c, b in
            Box.get(user).status { try $0.tree.insert(parent: p, child: c, before: b == 0 ? nil : b) }
        }
        cb.remove = { user, p, c in
            Box.get(user).status { try $0.tree.remove(parent: p, child: c) }
        }
        cb.commit = { user in
            Box.get(user).status { _ = try $0.tree.commit() }
        }
        cb.sync_clock = { user, ms in
            Box.get(user).status { $0.tree.syncClock(deviceNowMs: ms) }
        }
        cb.release = { user, tag in
            Box.get(user).status { h in
                h.tree.release(tag)
                h.releasedTags.append(tag)
            }
        }
        cb.record = { user, line, len in
            Box.get(user).host?.log.append(str(line, len))
        }
        var opt = sw_engine_options(memory_limit: options.memoryLimit, max_stack_size: options.maxStackSize)
        engine = sw_engine_new(&cb, &opt)
        box.engine = engine
    }

    deinit { sw_engine_free(engine) }

    /// Evaluates the bundle. If `expectedSHA256` (hex) is given, the bytes
    /// must hash to it or nothing is evaluated.
    public func load(_ source: Data, filename: String = "bundle.js", expectedSHA256: String? = nil) throws {
        let actual = JSHost.sha256Hex(source)
        if let exp = expectedSHA256, exp.lowercased() != actual {
            throw JSHostError.hashMismatch(expected: exp.lowercased(), actual: actual)
        }
        guard String(data: source, encoding: .utf8) != nil else { throw JSHostError.engine("bundle is not UTF-8") }
        try eval(source, filename: filename)
        bundleSHA256 = actual
        isLoaded = true
    }

    /// Evaluates a global script; returns its completion value.
    @discardableResult
    public func eval(_ source: Data, filename: String = "<eval>") throws -> JSData {
        guard let engine else { throw JSHostError.engine("no engine") }
        var bytes = [UInt8](source)
        bytes.append(0) // JS_Eval needs src[len] == 0
        var res: UnsafeMutablePointer<UInt8>?
        var resLen = 0
        let t0 = ContinuousClock.now
        let rc = bytes.withUnsafeBufferPointer { p in
            p.withMemoryRebound(to: CChar.self) { cp in
                sw_engine_eval(engine, cp.baseAddress, source.count, filename, timeoutMs,
                               &res, &resLen, &errBuf, errBuf.count)
            }
        }
        jsTime += ContinuousClock.now - t0
        return try finish(rc, res, resLen)
    }

    @discardableResult
    public func eval(_ source: String, filename: String = "<eval>") throws -> JSData {
        try eval(Data(source.utf8), filename: filename)
    }

    /// Creates the root (`width` x `height`) and calls `__app.mount(rootTag,
    /// {width, height})`.
    public func mount(width: CGFloat, height: CGFloat) throws {
        guard isLoaded else { throw JSHostError.notReady("mount before a bundle was loaded") }
        guard rootTag == nil else { throw JSHostError.notReady("already mounted") }
        let root = tree.createRoot(width: width, height: height)
        rootTag = root
        try mount(root: root, ctx: ["width": .number(Double(width)), "height": .number(Double(height))])
    }

    /// `__app.mount(root, ctx)` with an explicit ctx (tests, replays).
    public func mount(root: Int32, ctx: JSData) throws {
        rootTag = root
        try callApp("mount", [.number(Double(root)), ctx])
    }

    /// Calls `__app.onState(state)`. `receivedAt` is when the app received
    /// the state (local clock); the bundle's `syncClock(deviceNowMs)` sets the
    /// device clock offset against it (README: Time).
    public func onState(_ state: JSData, receivedAt: Date? = nil) throws {
        guard rootTag != nil else { throw JSHostError.notReady("onState before mount") }
        tree.receivedState(at: receivedAt ?? tree.now())
        try callApp("onState", [state])
    }

    /// Calls `__app.dispatch(tag, name, payload)`; returns its result
    /// (the renderer returns whether a handler ran).
    @discardableResult
    public func dispatch(tag: Int32, name: String, payload: JSData = .null) throws -> JSData {
        guard rootTag != nil else { throw JSHostError.notReady("dispatch before mount") }
        return try callApp("dispatch", [.number(Double(tag)), .string(name), payload])
    }

    /// Calls `__app[method](...args)` and returns the result.
    @discardableResult
    public func callApp(_ method: String, _ args: [JSData]) throws -> JSData {
        guard let engine else { throw JSHostError.engine("no engine") }
        let enc = JSDataCodec.encode(.array(args))
        var res: UnsafeMutablePointer<UInt8>?
        var resLen = 0
        let t0 = ContinuousClock.now
        let rc = enc.withUnsafeBufferPointer { p in
            sw_engine_call_app(engine, method, p.baseAddress, p.count, timeoutMs, &res, &resLen, &errBuf, errBuf.count)
        }
        jsTime += ContinuousClock.now - t0
        return try finish(rc, res, resLen)
    }

    /// Appends `JSON.stringify(args)` (computed by the engine) to the log,
    /// e.g. `["#state",0]`.
    public func recordMarker(_ args: [JSData]) {
        log.append(stringify(.array(args)) ?? "null")
    }

    public func clearLog() { log.removeAll() }

    /// `JSON.stringify(value)` by the engine.
    public func stringify(_ v: JSData) -> String? {
        let enc = JSDataCodec.encode(v)
        guard let p = enc.withUnsafeBufferPointer({ sw_engine_stringify(engine, $0.baseAddress, $0.count) }) else { return nil }
        defer { sw_free(p) }
        return String(cString: p)
    }

    /// Tags `release` was called for, in order.
    public private(set) var releasedTags: [Int32] = []

    /// Runs a GC and then the jobs it queued, so the renderer's
    /// FinalizationRegistry delivers `release(tag)` for unreachable nodes.
    public func collectGarbage() throws {
        let t0 = ContinuousClock.now
        let rc = sw_engine_gc(engine, timeoutMs, &errBuf, errBuf.count)
        jsTime += ContinuousClock.now - t0
        _ = try finish(rc, nil, 0)
    }

    public func memoryUsage() -> sw_mem_usage {
        var u = sw_mem_usage()
        sw_engine_memory(engine, &u)
        return u
    }

    private var timeoutMs: Double {
        let c = options.callTimeout.components
        return Double(c.seconds) * 1000 + Double(c.attoseconds) / 1e15
    }

    private func finish(_ rc: Int32, _ res: UnsafeMutablePointer<UInt8>?, _ len: Int) throws -> JSData {
        defer { if let res { sw_free(res) } }
        let msg = String(cString: errBuf)
        switch Int(rc) {
        case Int(SW_OK):
            guard let res else { return .undefined }
            do { return try JSDataCodec.decode(UnsafeRawBufferPointer(start: res, count: len)) } catch {
                throw JSHostError.engine("undecodable result")
            }
        case Int(SW_ERR_TIMEOUT): throw JSHostError.timeout(msg)
        case Int(SW_ERR_OOM): throw JSHostError.outOfMemory(msg)
        case Int(SW_ERR_EXCEPTION):
            throw JSHostError.exception(message: msg, stack: String(cString: sw_engine_last_stack(engine)))
        default: throw JSHostError.engine(msg.isEmpty ? "error \(rc)" : msg)
        }
    }

    private final class Box {
        weak var host: JSHost?
        var engine: OpaquePointer?

        static func get(_ p: UnsafeMutableRawPointer?) -> Box {
            Unmanaged<Box>.fromOpaque(p!).takeUnretainedValue()
        }

        /// Runs a mutation; on error hands the message to the engine (the
        /// JS call then throws it) and returns nil.
        func run<T>(_ f: (JSHost) throws -> T) -> T? {
            guard let h = host else { return nil }
            do { return try f(h) } catch {
                sw_engine_set_host_error(engine, "\(error)")
                return nil
            }
        }

        /// `run` for mutations without a result: 0 = ok, 1 = failed.
        func status(_ f: (JSHost) throws -> Void) -> Int32 {
            run(f) == nil ? 1 : 0
        }
    }
}

private func str(_ p: UnsafePointer<CChar>?, _ len: Int) -> String {
    guard let p else { return "" }
    return String(decoding: UnsafeRawBufferPointer(start: p, count: len), as: UTF8.self)
}
