import Foundation

/// Runs a bundle through a driver script exactly like the JS recording host
/// (README: Golden mutation logs) and returns the same log, so a host run
/// can be diffed against the golden `log.jsonl` line for line.
///
/// Script: `{ ctx: {width, height}, steps: [step, ...] }`, step one of
/// `{state: <json>}`, `{stateHex: "0201ff"}`,
/// `{dispatch: {tag, name, payload}}`, `{wait: ms}`.
///
/// Like the JS driver it owns a fixed clock (the bundle's `Date` reads it)
/// and runs the bundle's timers on it: T0 at mount, step i at
/// T0 + 1000 (i + 1) (or its `localMs`, never earlier than the clock), a
/// wait moves it on; the timers due by a step's time fire first, in due
/// order, each at its due time (a delay under 1 ms counts as 1 ms).
public final class ScriptRunner {
    public struct Result {
        /// The mutation log, one JSON array per line.
        public var log: [String]
        /// The snapshot after each commit.
        public var snapshots: [LayoutSnapshot]
        /// Time to evaluate the bundle, and to mount.
        public var loadTime: Duration
        public var mountTime: Duration
        /// Time of each step (onState / dispatch, including its commit).
        public var stepTimes: [Duration]
        /// SHA-256 of the bundle bytes.
        public var sha256: String
        public var error: JSHostError?
    }

    public let host: JSHost
    public var tree: ShadowTree { host.tree }
    /// The driver's clock (ms), which the bundle's `Date` reads.
    public static let t0Ms = 1_000_000.0
    public private(set) var localMs = ScriptRunner.t0Ms
    /// Armed timers: id -> due (ms on the driver's clock).
    private var timers: [Int32: Double] = [:]
    /// Everything the bundle sent.
    public private(set) var sent: [[UInt8]] = []

    /// `Date` on `globalThis.__swNow` (the JS driver's FixedDate).
    static let fixedDate = """
    (function (g) {
      const RealDate = g.Date;
      function FixedDate(...a) {
        if (!(this instanceof FixedDate)) return new RealDate(g.__swNow).toString();
        return a.length ? new RealDate(...a) : new RealDate(g.__swNow);
      }
      FixedDate.prototype = RealDate.prototype;
      FixedDate.now = () => g.__swNow;
      FixedDate.UTC = RealDate.UTC;
      FixedDate.parse = RealDate.parse;
      g.Date = FixedDate;
    })(globalThis);
    """

    public init(registry: NodeRegistry = NodeRegistry(), measurer: TextMeasurer,
                hostOptions: JSHost.Options = .init()) {
        host = JSHost(tree: ShadowTree(registry: registry, measurer: measurer), options: hostOptions)
        host.onSetTimer = { [unowned self] id, ms in self.timers[id] = self.localMs + max(ms, 1) }
        host.onClearTimer = { [unowned self] id in self.timers.removeValue(forKey: id) }
        host.onSend = { [unowned self] bytes in self.sent.append(bytes) }
    }

    private func setClock(_ ms: Double) throws {
        localMs = ms
        try host.eval("globalThis.__swNow = \(ms == ms.rounded() ? String(Int64(ms)) : String(ms));")
    }

    /// Fires the timers due by `t` in order, then sets the clock to `t`.
    private func advance(to t: Double) throws {
        while let (id, due) = timers.min(by: { $0.value < $1.value || ($0.value == $1.value && $0.key < $1.key) }),
              due <= t {
            timers.removeValue(forKey: id)
            if due > localMs { try setClock(due) }
            host.recordMarker(["#timer", .number(Double(id)), .number(localMs)])
            try host.fireTimer(id)
        }
        if t > localMs { try setClock(t) }
    }

    public func run(bundle: Data, expectedSHA256: String? = nil, script: JSData) -> Result {
        var snaps: [LayoutSnapshot] = []
        tree.onCommit = { snaps.append($0) }
        var r = Result(log: [], snapshots: [], loadTime: .zero, mountTime: .zero, stepTimes: [],
                       sha256: JSHost.sha256Hex(bundle), error: nil)
        let clock = ContinuousClock()
        host.isRecording = true
        defer {
            host.isRecording = false
        }
        do {
            try host.eval(ScriptRunner.fixedDate, filename: "fixed-date.js")
            try setClock(ScriptRunner.t0Ms)
            let t0 = clock.now
            try host.load(bundle, expectedSHA256: expectedSHA256)
            r.loadTime = clock.now - t0
        } catch let e as JSHostError {
            r.error = e
            if case .exception(let m, _) = e { host.recordMarker(["#error", .string(m)]) }
            r.log = host.log
            return r
        } catch {
            r.error = .engine("\(error)")
            return r
        }
        do {
            let ctx = script["ctx"] ?? ["width": 198, "height": 242]
            let w = ctx["width"]?.number ?? 198, h = ctx["height"]?.number ?? 242
            let root = tree.createRoot(width: w, height: h)
            host.recordMarker(["#mount", .number(Double(root)), ctx])
            let t1 = clock.now
            try host.mount(root: root, ctx: ctx)
            r.mountTime = clock.now - t1
            guard case .array(let steps)? = script["steps"] else { throw JSHostError.engine("script has no steps") }
            for (i, s) in steps.enumerated() {
                let ts = clock.now
                if let wait = s["wait"]?.number {
                    try advance(to: localMs + wait)
                    host.recordMarker(["#wait", .number(wait), .number(localMs)])
                    r.stepTimes.append(clock.now - ts)
                    continue
                }
                // Step i happens at T0 + 1000 (i + 1) unless it sets localMs.
                try advance(to: s["localMs"]?.number ?? (ScriptRunner.t0Ms + 1000 * Double(i + 1)))
                if let st = s["state"] {
                    host.recordMarker(["#state", .number(Double(i)), .number(localMs)])
                    try host.onState(st)
                } else if let hex = s["stateHex"]?.string {
                    host.recordMarker(["#state", .number(Double(i)), .number(localMs)])
                    try host.onState(.bytes(ScriptRunner.bytes(hex: hex)))
                } else if let d = s["dispatch"] {
                    let tag = Int32(d["tag"]?.number ?? 0)
                    let name = d["name"]?.string ?? ""
                    var payload = d["payload"] ?? .null
                    if payload == .undefined { payload = .null }
                    host.recordMarker(["#dispatch", .number(Double(tag)), .string(name), payload])
                    let handled = try host.dispatch(tag: tag, name: name, payload: payload)
                    host.recordMarker(["#handled", handled])
                } else {
                    throw JSHostError.engine("bad step \(i)")
                }
                r.stepTimes.append(clock.now - ts)
            }
        } catch let e as JSHostError {
            r.error = e
            let m: String
            switch e {
            case .exception(let msg, _): m = msg
            default: m = e.description
            }
            host.recordMarker(["#error", .string(m)])
        } catch {
            r.error = .engine("\(error)")
        }
        r.log = host.log
        r.snapshots = snaps
        return r
    }

    static func bytes(hex: String) -> [UInt8] {
        var out: [UInt8] = []
        var it = hex.makeIterator()
        while let a = it.next(), let b = it.next() {
            out.append(UInt8(String([a, b]), radix: 16) ?? 0)
        }
        return out
    }
}
