import XCTest
@testable import SolidWatch

final class JSHostTests: XCTestCase {
    private func host(_ opts: JSHost.Options = .init()) -> JSHost {
        JSHost(tree: Fx.stubTree(), options: opts)
    }

    /// A minimal bundle written against the raw host interface.
    private let tiny = """
    globalThis.__app = {
      mount(root, ctx) {
        const v = __host.createNode("view");
        __host.setProp(v, "width", ctx.width / 2);
        const t = __host.createNode("text");
        this.raw = __host.createText("0");
        __host.insert(t, this.raw, 0);
        __host.insert(v, t, 0);
        __host.insert(root, v, 0);
        __host.commit();
      },
      onState(s) {
        __host.setProp(this.raw, "text", String(s.n) + ":" + (s.b instanceof Uint8Array ? s.b.join(",") : "-"));
        __host.commit();
      },
      dispatch(tag, name, payload) { this.last = [tag, name, payload]; return true; },
    };
    """

    func testLoadMountStateAndValues() throws {
        let h = host()
        try h.load(Data(tiny.utf8), expectedSHA256: JSHost.sha256Hex(Data(tiny.utf8)))
        try h.mount(width: 100, height: 50)
        XCTAssertEqual(h.tree.snapshot?.root?.children.count, 1)
        try h.onState(["n": 7, "b": .bytes([1, 2, 255])])
        let leaf = h.tree.snapshot?.nodes.values.first { $0.type == "text" }
        XCTAssertEqual(leaf?.text?.runs.map(\.text), ["7:1,2,255"])
        XCTAssertEqual(try h.dispatch(tag: 3, name: "press", payload: ["agoMs": 12]), .bool(true))
        XCTAssertEqual(try h.eval("JSON.stringify(__app.last)"), .string("[3,\"press\",{\"agoMs\":12}]"))
        XCTAssertEqual(try h.eval("({a: [1, 'x', null, true], u: new Uint8Array([9])})"),
                       ["a": [1, "x", nil, true], "u": .bytes([9])])
    }

    func testStateKeyOrderAndJSON() throws {
        let h = host()
        try h.load(Data("globalThis.__app = { mount() {}, onState(s) { this.k = Object.keys(s).join() + '|' + s.n } }".utf8))
        try h.mount(width: 1, height: 1)
        try h.onState(try JSData(json: "{\"z\":1,\"a\":2,\"n\":123456789012}"))
        XCTAssertEqual(try h.eval("__app.k"), "z,a,n|123456789012")
        try h.onState(.json("{\"q\":0,\"b\":1,\"n\":0.1}"))
        XCTAssertEqual(try h.eval("__app.k"), "q,b,n|0.1")
    }

    func testHashMismatch() {
        let h = host()
        XCTAssertThrowsError(try h.load(Data(tiny.utf8), expectedSHA256: String(repeating: "0", count: 64))) { e in
            guard case .hashMismatch(_, let actual) = e as? JSHostError else { return XCTFail("\(e)") }
            XCTAssertEqual(actual, JSHost.sha256Hex(Data(tiny.utf8)))
        }
        XCTAssertFalse(h.isLoaded)
    }

    func testExceptionHasMessageAndStack() throws {
        let h = host()
        XCTAssertThrowsError(try h.load(Data("function boom() { throw new Error('nope') }\nboom()".utf8), filename: "b.js")) { e in
            guard case .exception(let m, let stack) = e as? JSHostError else { return XCTFail("\(e)") }
            XCTAssertEqual(m, "nope")
            XCTAssertTrue(stack.contains("boom"), stack)
        }
        // The engine is still usable afterwards.
        XCTAssertEqual(try h.eval("1+1"), 2)
    }

    func testSyntaxError() {
        let h = host()
        XCTAssertThrowsError(try h.load(Data("let = ;".utf8))) { e in
            guard case .exception = e as? JSHostError else { return XCTFail("\(e)") }
        }
    }

    func testHostErrorsReachJS() throws {
        let h = host()
        try h.load(Data(tiny.utf8))
        try h.mount(width: 10, height: 10)
        XCTAssertEqual(try h.eval("try { __host.createNode('nope'); 'no' } catch (e) { e instanceof TypeError ? e.message : 'other' }"),
                       .string("__host.createNode: unknown node type <nope>"))
        XCTAssertThrowsError(try h.eval("__host.setProp(2, 'bogus', 1)")) { e in
            guard case .exception(let m, _) = e as? JSHostError else { return XCTFail("\(e)") }
            XCTAssertEqual(m, "__host.setProp: <view> has no prop bogus")
        }
    }

    func testRunawayBundleIsInterrupted() throws {
        var o = JSHost.Options()
        o.callTimeout = .milliseconds(200)
        let h = host(o)
        let t0 = ContinuousClock.now
        XCTAssertThrowsError(try h.load(Data("for (;;) {}".utf8))) { e in
            guard case .timeout = e as? JSHostError else { return XCTFail("\(e)") }
        }
        XCTAssertLessThan(ContinuousClock.now - t0, .seconds(2))
        XCTAssertEqual(try h.eval("2+2"), 4) // usable again, with a fresh deadline
    }

    func testMemoryLimit() {
        var o = JSHost.Options()
        o.memoryLimit = 4 << 20
        let h = host(o)
        XCTAssertThrowsError(try h.load(Data("const a = []; for (;;) a.push(new Array(1000).fill(1))".utf8))) { e in
            guard case .outOfMemory = e as? JSHostError else { return XCTFail("\(e)") }
        }
    }

    func testOrderErrors() throws {
        let h = host()
        XCTAssertThrowsError(try h.mount(width: 1, height: 1)) { XCTAssertEqual($0 as? JSHostError, .notReady("mount before a bundle was loaded")) }
        try h.load(Data(tiny.utf8))
        XCTAssertThrowsError(try h.onState(1))
        XCTAssertThrowsError(try h.dispatch(tag: 1, name: "press"))
        let h2 = host()
        try h2.load(Data("globalThis.__app = {}".utf8))
        XCTAssertThrowsError(try h2.mount(width: 1, height: 1)) { e in
            guard case .exception(let m, _) = e as? JSHostError else { return XCTFail("\(e)") }
            XCTAssertEqual(m, "__app.mount is not a function")
        }
    }

    func testRecordingMatchesJSONStringify() throws {
        let h = host()
        try h.load(Data(tiny.utf8))
        h.isRecording = true
        h.recordMarker(["#mount", 1, ["width": 100, "height": 50.5]])
        try h.mount(root: h.tree.createRoot(width: 100, height: 50.5), ctx: ["width": 100, "height": 50.5])
        XCTAssertEqual(h.log, [
            "[\"#mount\",1,{\"width\":100,\"height\":50.5}]",
            "[\"createNode\",2,\"view\"]",
            "[\"setProp\",2,\"width\",50]",
            "[\"createNode\",3,\"text\"]",
            "[\"createText\",4,\"0\"]",
            "[\"insert\",3,4,0]",
            "[\"insert\",2,3,0]",
            "[\"insert\",1,2,0]",
            "[\"commit\"]",
        ])
    }
}
