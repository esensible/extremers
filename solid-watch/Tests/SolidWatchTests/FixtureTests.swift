import XCTest
@testable import SolidWatch

/// Layout parity with the browser host (yoga-layout + the stub measurer),
/// and golden mutation logs.
final class FixtureTests: XCTestCase {
    /// fixtures/layout/*.json: replay `log`, compare the last commit's
    /// frames / text / content with `expected`, exactly.
    func testLayoutParity() throws {
        let names = try Fx.names("layout").filter { $0.hasSuffix(".json") }
        XCTAssertFalse(names.isEmpty)
        for name in names {
            let fx = try JSData(json: Fx.text("layout/\(name)"))
            XCTAssertEqual(fx["measure"], "stub", name)
            XCTAssertEqual(fx["pointScaleFactor"], 2, name)
            guard case .array(let log)? = fx["log"] else { XCTFail(name); continue }
            let tree = Fx.stubTree()
            let snaps = try replay(log.compactMap(\.string), into: tree)
            guard let last = snaps.last, let expected = fx["expected"] else { XCTFail("\(name): no commit"); continue }
            XCTAssertEqual(json(normalized(parityJSON(last))), json(normalized(expected)), name)
        }
    }

    /// fixtures/mutations/*/log.jsonl replayed through the tree gives
    /// frames.jsonl, commit by commit.
    func testGoldenLogFrames() throws {
        for name in try Fx.names("mutations") {
            let log = try Fx.lines("mutations/\(name)/log.jsonl")
            let expected = try Fx.lines("mutations/\(name)/frames.jsonl").map { try JSData(json: $0) }
            let snaps = try replay(log, into: Fx.stubTree())
            XCTAssertEqual(snaps.count, expected.count, "\(name): commits")
            for (i, (s, e)) in zip(snaps, expected).enumerated() {
                var mine = parityJSON(s)
                if case .object(var f) = mine {
                    f.append(.init("commit", .number(Double(i))))
                    f.append(.init("haptics", .array(s.haptics.map {
                        ["tag": .number(Double($0.tag)), "pattern": .string($0.pattern.rawValue)]
                    })))
                    mine = .object(f)
                }
                XCTAssertEqual(json(normalized(mine)), json(normalized(e)), "\(name) commit \(i)")
            }
        }
    }

    /// The real bundles run in JSHost (QuickJS on this platform) produce the
    /// golden log (made by qjs in the container) line for line, and the
    /// bundle hash matches the recorded one.
    func testBundlesProduceGoldenLogs() throws {
        for name in try Fx.names("mutations") {
            let dir = Fx.fixtures.appendingPathComponent("mutations/\(name)")
            let bundle = try Data(contentsOf: dir.appendingPathComponent("bundle.js"))
            let sha = String(try Fx.text("mutations/\(name)/bundle.js.sha256").split(separator: " ")[0])
            let script = try JSData(json: Fx.text("mutations/\(name)/script.json"))
            let golden = try Fx.lines("mutations/\(name)/log.jsonl")
            let runner = ScriptRunner(measurer: StubTextMeasurer())
            runner.tree.now = { Fx.t0 }
            runner.tree.wallClock = { Fx.localDate }
            let r = runner.run(bundle: bundle, expectedSHA256: sha, script: script)
            XCTAssertEqual(r.sha256, sha, name)
            XCTAssertEqual(r.log.count, golden.count, "\(name): line count")
            for (i, (a, b)) in zip(r.log, golden).enumerated() where a != b {
                XCTFail("\(name) line \(i + 1):\n  host:   \(a)\n  golden: \(b)")
                break
            }
            if name == "bad-text" {
                XCTAssertNotNil(r.error, name)
            } else {
                XCTAssertNil(r.error, "\(name): \(String(describing: r.error))")
            }
        }
    }
}
