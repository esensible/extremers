import SolidWatch
import SwiftUI
import os

/// solid-watch demo: at launch, runs every fixture bundle through its golden
/// driver script and diffs the mutation log against the golden one (the
/// integrity check), then shows one bundle live, cycling its fixture states
/// and timing each update. Results are printed (prefix "DEMO "), logged
/// (subsystem au.esensible.solidwatch.demo) and written to
/// Documents/demo-results.txt.
@main
struct DemoApp: App {
    var body: some Scene {
        WindowGroup { DemoView() }
    }
}

struct DemoView: View {
    @StateObject private var session = SolidWatchSession(registry: Live.registry())
    @State private var report: [String] = []
    @State private var showReport = false

    var body: some View {
        ZStack {
            MountView(session: session)
            if showReport {
                ScrollView {
                    VStack(alignment: .leading, spacing: 1) {
                        ForEach(Array(report.enumerated()), id: \.offset) { _, l in
                            Text(l).font(.system(size: 9, design: .monospaced))
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                }
                .background(Color.black)
            }
        }
        .onTapGesture(count: 2) { showReport.toggle() }
        .task {
            // Let the first frame settle so the baseline footprint includes the UI.
            try? await Task.sleep(nanoseconds: 1_000_000_000)
            let out = Out()
            Integrity.runAll(out)
            report = out.lines
            await Live.run(session, out)
            report = out.lines
            out.save()
            out("DONE")
        }
    }
}

enum Fixtures {
    /// name -> directory: the package's fixtures/mutations/* plus LocalFixtures/*.
    static var dirs: [String: URL] {
        var out: [String: URL] = [:]
        for (res, sub) in [("fixtures", "mutations"), ("LocalFixtures", "")] {
            guard let base = Bundle.main.url(forResource: res, withExtension: nil) else { continue }
            let dir = sub.isEmpty ? base : base.appendingPathComponent(sub)
            for n in (try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? [] {
                let d = dir.appendingPathComponent(n)
                if FileManager.default.fileExists(atPath: d.appendingPathComponent("bundle.js").path) { out[n] = d }
            }
        }
        return out
    }

    static var mutationNames: [String] { dirs.keys.sorted() }

    /// Registers the schema font (Inter) bundled with the app.
    static func registerFont(_ out: Out) {
        guard let url = Bundle.main.url(forResource: "Inter-roman", withExtension: "ttf") else {
            out("font: Inter-roman.ttf not bundled; using the system font")
            return
        }
        do {
            let fams = try Fonts.register(url)
            Fonts.family = CoreSchema.fontFamily
            let size = (try? FileManager.default.attributesOfItem(atPath: url.path)[.size] as? Int) ?? 0
            out("font: registered \(fams) from \(url.lastPathComponent) (\(size) B)")
        } catch {
            out("font: registration failed: \(error)")
        }
    }

    struct Fixture {
        let name: String
        let bundle: Data
        let sha256: String
        let script: JSData
        let golden: [String]
        let fragment: String?
    }

    static func load(_ name: String) throws -> Fixture {
        guard let d = dirs[name] else { throw CocoaError(.fileNoSuchFile) }
        let bundle = try Data(contentsOf: d.appendingPathComponent("bundle.js"))
        let shaLine = try String(contentsOf: d.appendingPathComponent("bundle.js.sha256"), encoding: .utf8)
        let sha = String(shaLine.split(separator: " ").first ?? "")
        let script = try JSData(json: String(contentsOf: d.appendingPathComponent("script.json"), encoding: .utf8))
        let golden = try String(contentsOf: d.appendingPathComponent("log.jsonl"), encoding: .utf8)
            .split(separator: "\n", omittingEmptySubsequences: true).map(String.init)
        let frag = try? String(contentsOf: d.appendingPathComponent("fragment.json"), encoding: .utf8)
        return Fixture(name: name, bundle: bundle, sha256: sha, script: script, golden: golden, fragment: frag)
    }

    /// A registry with the fixture's app schema fragment, if it has one.
    static func registry(_ f: Fixture) throws -> NodeRegistry {
        let r = NodeRegistry()
        if let frag = f.fragment { try r.addSchema(fragmentJSON: frag) }
        return r
    }
}

enum Integrity {
    @MainActor
    static func runAll(_ out: Out) {
        #if arch(arm64_32)
        out("arch arm64_32")
        #elseif arch(arm64)
        out("arch arm64")
        #endif
        out("quickjs-ng \(JSHost.quickJSVersion)")
        Fixtures.registerFont(out)
        let fp0 = Mem.footprint()
        out("mem start: footprint \(Mem.mb(fp0))")
        var allMatch = true
        for name in Fixtures.mutationNames {
            do {
                let f = try Fixtures.load(name)
                let measurer = CoreTextMeasurer()
                let runner = ScriptRunner(registry: try Fixtures.registry(f), measurer: measurer)
                let fpA = Mem.footprint()
                let r = runner.run(bundle: f.bundle, expectedSHA256: f.sha256, script: f.script)
                let fpB = Mem.footprint()
                let hashOK = r.sha256 == f.sha256
                let match = r.log == f.golden
                allMatch = allMatch && match && hashOK
                out("[\(name)] bundle \(f.bundle.count) B sha256 \(r.sha256) expected \(f.sha256) -> \(hashOK ? "OK" : "MISMATCH")")
                out("[\(name)] log \(r.log.count) lines vs golden \(f.golden.count): \(match ? "MATCH" : "DIFF")")
                if !match {
                    let i = Array(zip(r.log, f.golden)).firstIndex { $0.0 != $0.1 } ?? min(r.log.count, f.golden.count)
                    out("[\(name)]   first diff at line \(i + 1)")
                    out("[\(name)]   watch:  \(i < r.log.count ? r.log[i] : "<end>")")
                    out("[\(name)]   golden: \(i < f.golden.count ? f.golden[i] : "<end>")")
                }
                if let e = r.error { out("[\(name)] bundle error (expected for bad-text): \(e.description.prefix(120))") }
                let steps = r.stepTimes.map(Self.ms)
                let avg = steps.isEmpty ? 0 : steps.reduce(0, +) / Double(steps.count)
                out(String(format: "[%@] load %.2f ms, mount %.2f ms, %d steps avg %.2f ms max %.2f ms",
                           name, ms(r.loadTime), ms(r.mountTime), steps.count, avg, steps.max() ?? 0))
                out(String(format: "[%@] commit %.2f ms total (yoga %.2f), js-side calls %.2f ms",
                           name, ms(runner.tree.commitTime), ms(runner.tree.yogaTime), ms(runner.host.jsTime)))
                let u = runner.host.memoryUsage()
                out("[\(name)] phys_footprint delta \(Mem.dmb(fpA, fpB)), js heap \(Mem.mb(UInt64(u.malloc_size))) objs \(u.obj_count)")
                out("[\(name)] coretext cache \(measurer.cacheHits) hits / \(measurer.cacheMisses) misses")
                if let s = r.snapshots.last {
                    out("[\(name)] frames of the mounted tree (last commit #\(s.sequence)):")
                    for l in s.frameDump().split(separator: "\n") { out("[\(name)]   \(l)") }
                }
            } catch {
                allMatch = false
                out("[\(name)] FAILED to load fixture: \(error)")
            }
        }
        out("INTEGRITY: \(allMatch ? "ALL MATCH" : "MISMATCH")")
        out("mem after integrity: footprint \(Mem.mb(Mem.footprint())) (\(Mem.dmb(fp0, Mem.footprint())))")
    }

    static func ms(_ d: Duration) -> Double {
        let c = d.components
        return Double(c.seconds) * 1000 + Double(c.attoseconds) / 1e15
    }
}

/// Shows a fixture bundle live and times each state update, split into JS,
/// Yoga, snapshot, diff/apply into the per-node view models, and SwiftUI
/// render (node view bodies evaluated, and time from the end of the update
/// to the last body evaluation).
enum Live {
    @MainActor
    static func run(_ session: SolidWatchSession, _ out: Out) async {
        let names = Fixtures.mutationNames
        for name in names.contains("tune") ? ["tune"] : ["basic", "list"] { await run(name, session, out) }
    }

    /// The live bundle's registry (with its fragment, if any).
    static func registry() -> NodeRegistry {
        if Fixtures.mutationNames.contains("tune"), let f = try? Fixtures.load("tune"), let r = try? Fixtures.registry(f) {
            return r
        }
        return NodeRegistry()
    }

    static func states(_ script: JSData) -> [JSData] {
        guard case .array(let steps)? = script["steps"] else { return [] }
        return steps.compactMap { s in
            if let st = s["state"] { return st }
            if let hex = s["stateHex"]?.string {
                var b: [UInt8] = []
                var i = hex.startIndex
                while i < hex.endIndex, let j = hex.index(i, offsetBy: 2, limitedBy: hex.endIndex) {
                    b.append(UInt8(hex[i..<j], radix: 16) ?? 0)
                    i = j
                }
                return .bytes(b)
            }
            return nil
        }
    }

    @MainActor
    static func run(_ name: String, _ session: SolidWatchSession, _ out: Out) async {
        guard let f = try? Fixtures.load(name) else { out("live: no fixture \(name)"); return }
        let all = states(f.script)
        // Tune has ~100 states: show the first 20 live, once each.
        let states = Array(all.prefix(20))
        let fp0 = Mem.footprint()
        let t0 = ContinuousClock.now
        session.load(f.bundle, expectedSHA256: f.sha256)
        // Mount happens when MountView reports its size.
        while session.snapshot == nil && session.error == nil {
            try? await Task.sleep(nanoseconds: 10_000_000)
        }
        out(String(format: "live[%@]: load+mount %.2f ms (incl. waiting for the view size)", name,
                   Integrity.ms(ContinuousClock.now - t0)))
        if let e = session.error { out("live error: \(e)"); return }
        try? await Task.sleep(nanoseconds: 500_000_000)
        out("live: mem after mount \(Mem.dmb(fp0, Mem.footprint()))")
        guard let host = session.host else { return }
        let tree = host.tree
        for round in 0..<(all.count > 20 ? 1 : 3) {
            for (i, s) in states.enumerated() {
                session.store.resetCounters()
                let js0 = host.jsTime, c0 = tree.commitTime, y0 = tree.yogaTime
                let start = ContinuousClock.now
                session.setState(s)
                let end = ContinuousClock.now
                let changed = session.store.lastChanged
                let apply = session.store.applyTime
                try? await Task.sleep(nanoseconds: 400_000_000)
                let bodies = session.store.bodyEvaluations
                let render = session.store.lastBodyEvaluation.map { Integrity.ms($0 - end) } ?? 0
                let js = Integrity.ms(host.jsTime - js0), commit = Integrity.ms(tree.commitTime - c0)
                let yoga = Integrity.ms(tree.yogaTime - y0), app = Integrity.ms(apply)
                out(String(format: "live r%d s%d: total %.2f ms = js %.2f + yoga %.2f + snapshot %.2f + apply %.2f; changed %d models, %d node bodies, last body +%.2f ms",
                           round, i, Integrity.ms(end - start), js - commit, yoga, commit - yoga - app, app,
                           changed, bodies, render))
                try? await Task.sleep(nanoseconds: 600_000_000)
            }
        }
        if let m = tree.measurer as? CoreTextMeasurer {
            out("live: coretext cache \(m.cacheHits) hits / \(m.cacheMisses) misses")
        }
        out("live: mem end \(Mem.dmb(fp0, Mem.footprint()))")
    }
}

enum Mem {
    static func footprint() -> UInt64 {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<integer_t>.size)
        let kr = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        return kr == KERN_SUCCESS ? UInt64(info.phys_footprint) : 0
    }

    static func mb(_ b: UInt64) -> String { String(format: "%.2f MB", Double(b) / 1_048_576) }
    static func dmb(_ a: UInt64, _ b: UInt64) -> String {
        String(format: "%+.2f MB", (Double(b) - Double(a)) / 1_048_576)
    }
}

final class Out {
    private(set) var lines: [String] = []
    private let log = Logger(subsystem: "au.esensible.solidwatch.demo", category: "demo")
    func callAsFunction(_ s: String) {
        lines.append(s)
        print("DEMO " + s)
        log.notice("DEMO \(s, privacy: .public)")
    }
    func save() {
        guard let dir = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first else { return }
        try? lines.joined(separator: "\n").appending("\n")
            .write(to: dir.appendingPathComponent("demo-results.txt"), atomically: true, encoding: .utf8)
    }
}
