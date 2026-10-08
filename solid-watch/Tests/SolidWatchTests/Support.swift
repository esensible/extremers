import Foundation
import XCTest
@testable import SolidWatch

/// Paths and helpers shared by the tests. Fixtures are read in place from
/// solid-watch/fixtures (the JS side writes them; README: Fixtures).
enum Fx {
    static let packageRoot = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    static let fixtures = packageRoot.appendingPathComponent("fixtures")

    static func text(_ rel: String) throws -> String {
        try String(contentsOf: fixtures.appendingPathComponent(rel), encoding: .utf8)
    }

    static func lines(_ rel: String) throws -> [String] {
        try text(rel).split(separator: "\n", omittingEmptySubsequences: true).map(String.init)
    }

    static func names(_ dir: String) throws -> [String] {
        try FileManager.default.contentsOfDirectory(atPath: fixtures.appendingPathComponent(dir).path).sorted()
    }

    /// The fixtures' fixed clock: T0 = 1_000_000 ms; every state arrives at
    /// T0 at mount, offset 0 until a syncClock (README: Golden mutation logs).
    static let t0 = Date(timeIntervalSince1970: 1000)

    static func stubTree(registry: NodeRegistry = NodeRegistry()) -> ShadowTree {
        let t = ShadowTree(registry: registry, measurer: StubTextMeasurer())
        t.now = { t0 }
        return t
    }
}

extension PropValue: ExpressibleByIntegerLiteral, ExpressibleByFloatLiteral, ExpressibleByStringLiteral,
    ExpressibleByBooleanLiteral, ExpressibleByArrayLiteral
{
    public init(integerLiteral v: Int) { self = .number(Double(v)) }
    public init(floatLiteral v: Double) { self = .number(v) }
    public init(stringLiteral v: String) { self = .string(v) }
    public init(booleanLiteral v: Bool) { self = .bool(v) }
    public init(arrayLiteral a: PropValue...) { self = .array(a) }
}

/// Replays mutation-log lines through a tree, the way the JS tests'
/// `replay()` does. Returns the snapshot of each commit.
func replay(_ lines: [String], into tree: ShadowTree, file: StaticString = #filePath, line: UInt = #line) throws -> [LayoutSnapshot] {
    var out: [LayoutSnapshot] = []
    // The fixtures' local clock: T0 at mount, then each #state's receipt time.
    var localMs = 1_000_000.0
    tree.now = { Date(timeIntervalSince1970: localMs / 1000) }
    for l in lines {
        guard case .array(let a) = try JSData(json: l), let op = a.first?.string else {
            XCTFail("bad log line \(l)", file: file, line: line)
            continue
        }
        func i(_ k: Int) -> Int32 { Int32(a[k].number ?? -1) }
        switch op {
        case "#mount":
            let ctx = a[2]
            let root = tree.createRoot(width: ctx["width"]?.number ?? 0, height: ctx["height"]?.number ?? 0)
            XCTAssertEqual(root, i(1), "root tag", file: file, line: line)
        case "createNode":
            XCTAssertEqual(try tree.createNode(a[2].string ?? ""), i(1), "tag of \(l)", file: file, line: line)
        case "createText":
            XCTAssertEqual(tree.createText(a[2].string ?? ""), i(1), "tag of \(l)", file: file, line: line)
        case "setProp":
            try tree.setProp(i(1), a[2].string ?? "", a[3].propValue)
        case "insert":
            try tree.insert(parent: i(1), child: i(2), before: i(3) == 0 ? nil : i(3))
        case "remove":
            try tree.remove(parent: i(1), child: i(2))
        case "#state":
            localMs = a[2].number ?? localMs
            tree.receivedState(at: tree.now())
        case "syncClock":
            tree.syncClock(deviceNowMs: a[1].number ?? 0)
        case "commit":
            out.append(try tree.commit())
        default:
            break // driver markers
        }
    }
    return out
}

/// A snapshot in the JS `layout()` shape: frames, text and content by tag.
func parityJSON(_ s: LayoutSnapshot) -> JSData {
    var frames: [JSData.Field] = [], text: [JSData.Field] = [], content: [JSData.Field] = []
    for tag in s.nodes.keys.sorted() {
        let n = s.nodes[tag]!
        let f = n.frame
        frames.append(.init(String(tag), [.number(Double(f.minX)), .number(Double(f.minY)),
                                          .number(Double(f.width)), .number(Double(f.height))]))
        if let t = n.text {
            text.append(.init(String(tag), ["fontSize": .number(t.measured.fontSize), "lines": .number(Double(t.measured.lines))]))
        }
        if n.type == "scroll" { content.append(.init(String(tag), .number(Double(n.contentHeight)))) }
    }
    return ["frames": .object(frames), "text": .object(text), "content": .object(content)]
}

/// Compact JSON for readable failure messages.
func json(_ v: JSData) -> String {
    switch v {
    case .undefined, .null, .function: return "null"
    case .bool(let b): return b ? "true" : "false"
    case .number(let d): return d == d.rounded() && abs(d) < 1e15 ? String(Int64(d)) : String(d)
    case .string(let s): return "\"\(s)\""
    case .bytes(let b): return "\(b)"
    case .array(let a): return "[" + a.map(json).joined(separator: ",") + "]"
    case .object(let f): return "{" + f.map { "\"\($0.key)\":" + json($0.value) }.joined(separator: ",") + "}"
    case .json(let s): return s
    }
}

/// Order-insensitive comparison of JSON objects (key order is not meaningful
/// in the fixtures).
func normalized(_ v: JSData) -> JSData {
    switch v {
    case .object(let f): return .object(f.sorted { $0.key < $1.key }.map { .init($0.key, normalized($0.value)) })
    case .array(let a): return .array(a.map(normalized))
    default: return v
    }
}
