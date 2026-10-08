import SwiftUI
import XCTest
@testable import SolidWatch

final class ShadowTreeTests: XCTestCase {
    private func tree() -> (ShadowTree, Int32) {
        let t = Fx.stubTree()
        let root = t.createRoot(width: 100, height: 100)
        return (t, root)
    }

    private func children(_ s: LayoutSnapshot, _ tag: Int32) -> [Int32] { s[tag]?.children ?? [] }

    func testInsertAppendAndBefore() throws {
        let (t, root) = tree()
        let a = try t.createNode("view"), b = try t.createNode("view"), c = try t.createNode("view")
        try t.insert(parent: root, child: a, before: nil)
        try t.insert(parent: root, child: c, before: 0)
        try t.insert(parent: root, child: b, before: c)
        for (k, v) in [a: 10, b: 20, c: 30] { try t.setProp(k, "height", .number(Double(v))) }
        let s = try t.commit()
        XCTAssertEqual(children(s, root), [a, b, c])
        XCTAssertEqual(s[a]?.frame.minY, 0)
        XCTAssertEqual(s[b]?.frame.minY, 10)
        XCTAssertEqual(s[c]?.frame.minY, 30)
    }

    func testInsertOfAttachedNodeMoves() throws {
        let (t, root) = tree()
        let p1 = try t.createNode("view"), p2 = try t.createNode("view")
        let a = try t.createNode("view"), b = try t.createNode("view")
        try t.insert(parent: root, child: p1, before: nil)
        try t.insert(parent: root, child: p2, before: nil)
        try t.insert(parent: p1, child: a, before: nil)
        try t.insert(parent: p1, child: b, before: nil)
        try t.setProp(a, "height", 5)
        try t.setProp(b, "height", 7)
        var s = try t.commit()
        XCTAssertEqual(children(s, p1), [a, b])
        // Reorder within the same parent (no remove first, like Solid).
        try t.insert(parent: p1, child: b, before: a)
        s = try t.commit()
        XCTAssertEqual(children(s, p1), [b, a])
        XCTAssertEqual(s[a]?.frame.minY, 7)
        // Move to another parent.
        try t.insert(parent: p2, child: a, before: nil)
        s = try t.commit()
        XCTAssertEqual(children(s, p1), [b])
        XCTAssertEqual(children(s, p2), [a])
        XCTAssertEqual(s[p1]?.frame.height, 7)
        XCTAssertEqual(s[p2]?.frame.height, 5)
        XCTAssertEqual(s[p2]?.frame.minY, 7)
    }

    func testRemoveKeepsNodeAndReleaseFrees() throws {
        let (t, root) = tree()
        let a = try t.createNode("view"), b = try t.createNode("text"), tx = t.createText("hi")
        try t.insert(parent: root, child: a, before: nil)
        try t.insert(parent: a, child: b, before: nil)
        try t.insert(parent: b, child: tx, before: nil)
        try t.setProp(b, "alignSelf", "flex-start")
        try t.commit()
        try t.remove(parent: root, child: a)
        var s = try t.commit()
        XCTAssertEqual(t.nodeCount, 4)
        XCTAssertEqual(children(s, root), [])
        XCTAssertNil(s[a])
        // Re-inserting a removed node brings its subtree back.
        try t.insert(parent: root, child: a, before: nil)
        s = try t.commit()
        XCTAssertEqual(children(s, root), [a])
        XCTAssertEqual(s[b]?.text?.runs.map(\.text), ["hi"])
        XCTAssertEqual(s[b]?.frame.width, 19.5)
        try t.release(a)
        s = try t.commit()
        XCTAssertEqual(t.nodeCount, 3)
        XCTAssertEqual(children(s, root), [])
        XCTAssertThrowsError(try t.insert(parent: root, child: a, before: nil))
        try t.insert(parent: root, child: b, before: nil) // its child was only detached
        s = try t.commit()
        XCTAssertEqual(children(s, root), [b])
    }

    func testValidation() throws {
        let (t, root) = tree()
        let a = try t.createNode("view"), b = try t.createNode("view")
        XCTAssertThrowsError(try t.createNode("nope")) { XCTAssertEqual("\($0)", "unknown node type <nope>") }
        XCTAssertThrowsError(try t.setProp(a, "fontSize", 12)) { XCTAssertEqual("\($0)", "<view> has no prop fontSize") }
        XCTAssertThrowsError(try t.setProp(999, "width", 1))
        XCTAssertThrowsError(try t.insert(parent: root, child: a, before: b)) {
            XCTAssertEqual("\($0)", "insert: \(b) is not a child of \(root)")
        }
        XCTAssertThrowsError(try t.remove(parent: root, child: a))
        try t.insert(parent: root, child: a, before: nil)
        try t.insert(parent: a, child: b, before: nil)
        XCTAssertThrowsError(try t.insert(parent: b, child: a, before: nil)) // cycle
        let tx = t.createText("x")
        XCTAssertThrowsError(try t.setProp(tx, "color", "white"))
        // An invalid value resets to the default (like the browser host), no throw.
        try t.setProp(a, "width", "wide")
        let s = try t.commit()
        XCTAssertEqual(s[a]?.frame.width, 100)
    }

    func testPropResetToDefault() throws {
        let (t, root) = tree()
        let a = try t.createNode("view")
        try t.insert(parent: root, child: a, before: nil)
        try t.setProp(a, "width", 40)
        try t.setProp(a, "background", "#FF0000")
        var s = try t.commit()
        XCTAssertEqual(s[a]?.frame.width, 40)
        XCTAssertEqual(s[a]?.paint.background, RGBA(rgba: 0xFF00_00FF))
        try t.setProp(a, "width", .null)
        try t.setProp(a, "background", .null)
        s = try t.commit()
        XCTAssertEqual(s[a]?.frame.width, 100)
        XCTAssertNil(s[a]?.paint.background)
    }

    func testTextUpdateRemeasures() throws {
        let (t, root) = tree()
        let tx = try t.createNode("text"), raw = t.createText("ab")
        try t.insert(parent: root, child: tx, before: nil)
        try t.insert(parent: tx, child: raw, before: nil)
        try t.setProp(tx, "alignSelf", "flex-start")
        var s = try t.commit()
        XCTAssertEqual(s[tx]?.text?.measured.width, 2 * 0.6 * 16)
        XCTAssertEqual(s[tx]?.frame.width, 19.5) // Yoga rounds to the 0.5 pt grid
        try t.setProp(raw, "text", "abcd")
        s = try t.commit()
        XCTAssertEqual(s[tx]?.text?.measured.width, 4 * 0.6 * 16)
        XCTAssertEqual(s[tx]?.frame.width, 38.5)
        XCTAssertEqual(s[tx]?.text?.runs.map(\.text), ["abcd"])
    }

    func testHapticFiresOnTriggerChange() throws {
        let (t, root) = tree()
        let h = try t.createNode("haptic")
        try t.insert(parent: root, child: h, before: nil)
        try t.setProp(h, "pattern", "success")
        try t.setProp(h, "trigger", 0)
        XCTAssertEqual(try t.commit().haptics, []) // the initial value doesn't play
        XCTAssertEqual(try t.commit().haptics, [])
        try t.setProp(h, "trigger", 1)
        XCTAssertEqual(try t.commit().haptics, [.init(tag: h, pattern: .success)])
        try t.setProp(h, "trigger", .null)
        XCTAssertEqual(try t.commit().haptics, [])
    }

    func testCountdownTicks() throws {
        let t = Fx.stubTree()
        var now = Fx.t0
        t.now = { now }
        let root = t.createRoot(width: 100, height: 100)
        let c = try t.createNode("countdown")
        try t.insert(parent: root, child: c, before: nil)
        try t.setProp(c, "toMs", 2500)
        var s = try t.commit()
        XCTAssertEqual(s[c]?.text?.runs.map(\.text), ["00:03"])
        XCTAssertEqual(t.nextTick(), Fx.t0.addingTimeInterval(0.501))
        now = Fx.t0.addingTimeInterval(0.4)
        XCTAssertFalse(t.tick())
        now = Fx.t0.addingTimeInterval(0.6)
        XCTAssertTrue(t.tick())
        s = try t.relayout()
        XCTAssertEqual(s[c]?.text?.runs.map(\.text), ["00:02"])
    }

    func testTimeFormats() {
        XCTAssertEqual(TimeFormat.countdown(.countdown, remainingMs: 65000), "01:05")
        XCTAssertEqual(TimeFormat.countdown(.countdown, remainingMs: 1), "00:01")
        XCTAssertEqual(TimeFormat.countdown(.countdown, remainingMs: -5), "00:00")
        XCTAssertEqual(TimeFormat.countdown(.remaining, remainingMs: 61500), "1:02")
        XCTAssertEqual(TimeFormat.countdown(.remaining, remainingMs: 3_725_000), "1:02:05")
        XCTAssertEqual(TimeFormat.countdown(.elapsed, remainingMs: -3_725_000), "1:02:05")
        XCTAssertEqual(TimeFormat.countdown(.elapsed, remainingMs: 500), "0:00")
        XCTAssertEqual(TimeFormat.clock(.hourMinute12, date: Fx.localDate), "9:41")
        XCTAssertEqual(TimeFormat.clock(.hourMinuteSecond24, date: Fx.localDate), "09:41:07")
    }

    func testCustomNodeTypeFromFragment() throws {
        let reg = NodeRegistry()
        reg.addSchema(
            nodes: ["strip": NodeSpec(type: "strip", children: .none, native: true, layout: true, props: [
                "level": PropSpec(name: "level", kind: .paint, type: .number, group: "strip", inherits: false, values: []),
            ].merging(CoreSchema.nodes["view"]!.props) { a, _ in a })],
            extends: ["button": ["bytes": PropSpec(name: "bytes", kind: .behavior, type: .bytes, group: "x:button", inherits: false, values: [])]]
        )
        reg.registerView("strip", measure: { props, _ in CGSize(width: 10 * (props["level"].flatMap(PropDecode.number) ?? 0), height: 4) }) { _ in
            EmptyView()
        }
        let t = Fx.stubTree(registry: reg)
        let root = t.createRoot(width: 100, height: 100)
        let s1 = try t.createNode("strip"), b = try t.createNode("button")
        try t.insert(parent: root, child: s1, before: nil)
        try t.insert(parent: root, child: b, before: nil)
        try t.setProp(s1, "level", 3)
        try t.setProp(s1, "alignSelf", "flex-start")
        try t.setProp(b, "bytes", [1, 2])
        let s = try t.commit()
        XCTAssertEqual(s[s1]?.frame.size, CGSize(width: 30, height: 4))
        XCTAssertEqual(s[b]?.props["bytes"], .array([.number(1), .number(2)]))
    }
}
