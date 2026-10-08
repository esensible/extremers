import CYoga
import CoreGraphics
import Foundation

/// A mutation the tree rejected (unknown tag, type or prop, ...). Reaches
/// the bundle as a TypeError thrown by the `__host` call.
public struct MutationError: Error, CustomStringConvertible, Equatable {
    public let message: String
    public init(_ message: String) { self.message = message }
    public var description: String { message }
}

/// The shadow tree: tag -> node store, one Yoga node per laid-out node.
/// Applies `__host` mutations as given (inserting an attached node moves
/// it; a removed node is kept until `release`); `commit()` runs Yoga once and
/// publishes an immutable `LayoutSnapshot`.
///
/// It mirrors the browser host's shadow tree (README: Host interface): the
/// same validation, the same text algorithm, the same frames.
///
/// Not thread-safe: use it from one thread (the main thread in an app).
public final class ShadowTree {
    public struct Options {
        /// Yoga rounds frames to 1/pointScaleFactor points.
        public var pointScaleFactor: Float = 2
        public init() {}
    }

    final class Node {
        let tag: Int32
        let type: String
        let spec: NodeSpec?
        var props: [String: PropValue] = [:]
        var text = ""
        var children: [Int32] = []
        var parent: Int32?
        let yoga: YGNodeRef?
        unowned let tree: ShadowTree

        var layout = LayoutProps()
        var box = BoxProps()
        var textLayout = TextLayoutProps()
        var button = ButtonProps()
        var countdown = CountdownProps()
        var clock = ClockProps()
        var haptic = HapticProps()
        /// When `toMs` was set (state receipt time).
        var anchor: Date?
        /// Trigger at the previous commit; nil before the first.
        var hapticPrev: PropValue??
        var measured: MeasuredText?
        var shown = ""

        init(tag: Int32, type: String, spec: NodeSpec?, yoga: YGNodeRef?, tree: ShadowTree) {
            self.tag = tag
            self.type = type
            self.spec = spec
            self.yoga = yoga
            self.tree = tree
        }

        var isLeaf: Bool { type == "text" || type == "countdown" || type == "clock" }
    }

    public let registry: NodeRegistry
    public let measurer: TextMeasurer
    private var nodes: [Int32: Node] = [:]
    private var nextTag: Int32 = 1
    private let config: YGConfigRef
    private var dirtyStyle = Set<Int32>()
    public private(set) var rootTag: Int32?
    public private(set) var commits = 0
    /// Latest published snapshot.
    public private(set) var snapshot: LayoutSnapshot?
    /// Called with each new snapshot.
    public var onCommit: ((LayoutSnapshot) -> Void)?
    /// Invalid prop values (reset to their default, as in the browser host).
    public var onWarning: ((String) -> Void)?
    /// When the state being applied was received: `countdown`'s `toMs` is
    /// relative to it. The host sets it before each `onState`.
    public var stateReceivedAt = Date()
    /// The clock countdowns and clocks read (injectable for tests).
    public var now: () -> Date = Date.init
    /// The time `clock` shows (the same as `now` unless a test pins it).
    public lazy var wallClock: () -> Date = { [unowned self] in self.now() }
    public var calendar = Calendar.current
    /// Time spent in commit (style + Yoga + leaf text + snapshot), for profiling.
    public private(set) var commitTime: Duration = .zero
    /// The `YGNodeCalculateLayout` part of `commitTime` (incl. text measures).
    public private(set) var yogaTime: Duration = .zero

    public init(registry: NodeRegistry = NodeRegistry(), measurer: TextMeasurer = CoreTextMeasurer(),
                options: Options = Options()) {
        self.registry = registry
        self.measurer = measurer
        config = YGConfigNew()
        YGConfigSetPointScaleFactor(config, options.pointScaleFactor)
    }

    deinit {
        for n in nodes.values { if let y = n.yoga { YGNodeFree(y) } }
        nodes.removeAll()
        YGConfigFree(config)
    }

    public var nodeCount: Int { nodes.count }

    // MARK: Root

    /// Creates the root the bundle mounts into: a column of the given size.
    @discardableResult
    public func createRoot(width: CGFloat, height: CGFloat) -> Int32 {
        let tag = nextTag
        nextTag += 1
        let y = YGNodeNewWithConfig(config)!
        nodes[tag] = Node(tag: tag, type: "#root", spec: nil, yoga: y, tree: self)
        rootTag = tag
        setRootSize(width: width, height: height)
        return tag
    }

    /// Resizes the root; `relayout()` publishes.
    public func setRootSize(width: CGFloat, height: CGFloat) {
        guard let r = rootTag, let y = nodes[r]?.yoga else { return }
        YGNodeStyleSetWidth(y, Float(width))
        YGNodeStyleSetHeight(y, Float(height))
    }

    // MARK: Mutations

    public func createNode(_ type: String) throws -> Int32 {
        guard let spec = registry.nodes[type] else { throw MutationError("unknown node type <\(type)>") }
        let tag = nextTag
        nextTag += 1
        var y: YGNodeRef?
        if spec.layout { y = YGNodeNewWithConfig(config) }
        let n = Node(tag: tag, type: type, spec: spec, yoga: y, tree: self)
        nodes[tag] = n
        if let y {
            YGNodeSetContext(y, Unmanaged.passUnretained(n).toOpaque())
            if n.isLeaf || registry.views[type]?.measure != nil { YGNodeSetMeasureFunc(y, measureFunc) }
            dirtyStyle.insert(tag)
        }
        return tag
    }

    public func createText(_ text: String) -> Int32 {
        let tag = nextTag
        nextTag += 1
        let n = Node(tag: tag, type: "#text", spec: nil, yoga: nil, tree: self)
        n.text = text
        nodes[tag] = n
        return tag
    }

    public func setProp(_ tag: Int32, _ key: String, _ value: PropValue) throws {
        let n = try get(tag)
        if n.type == "#text" {
            guard key == "text" else { throw MutationError("raw text only takes \"text\", got \(key)") }
            switch value {
            case .string(let s): n.text = s
            case .null: n.text = "null"
            case .bool(let b): n.text = b ? "true" : "false"
            case .number(let d): n.text = d == d.rounded() && abs(d) < 1e15 ? String(Int64(d)) : String(d)
            default: n.text = "\(value)"
            }
            dirtyText(n)
            return
        }
        guard let spec = n.spec?.props[key] else { throw MutationError("<\(n.type)> has no prop \(key)") }
        if value == .null { n.props.removeValue(forKey: key) } else { n.props[key] = value }
        if key == "toMs" { n.anchor = stateReceivedAt }
        var r = PropSetResult.ok
        switch spec.group {
        case "layout": r = n.layout.set(key, value)
        case "box": r = n.box.set(key, value)
        case "textLayout": r = n.textLayout.set(key, value)
        case "textStyle": var t = TextStyleProps(); r = t.set(key, value)
        case "button": r = n.button.set(key, value)
        case "countdown": r = n.countdown.set(key, value)
        case "clock": r = n.clock.set(key, value)
        case "haptic": r = n.haptic.set(key, value)
        default: break // app fragment props: kept raw
        }
        if r == .invalid { onWarning?("<\(n.type)> \(key): invalid value \(value), using the default") }
        switch spec.kind {
        case .layout: dirtyStyle.insert(tag)
        case .measure: dirtyText(n)
        default: break
        }
    }

    /// Inserts `child` into `parent` before `before` (nil or 0 = append). A
    /// child that is already attached is moved.
    public func insert(parent: Int32, child: Int32, before: Int32?) throws {
        let p = try get(parent)
        let c = try get(child)
        var index: Int?
        if let b = before, b != 0 {
            let bn = try get(b)
            guard bn.parent == parent else { throw MutationError("insert: \(b) is not a child of \(parent)") }
        }
        if c.tag == rootTag { throw MutationError("insert: the root cannot be a child") }
        var a: Int32? = parent
        while let t = a {
            if t == child { throw MutationError("insert: \(child) is \(parent) or one of its ancestors") }
            a = nodes[t]?.parent
        }
        if c.parent != nil { detach(c) }
        if let b = before, b != 0 { index = p.children.firstIndex(of: b) }
        let i = index ?? p.children.count
        p.children.insert(child, at: i)
        c.parent = parent
        if let py = p.yoga, !p.isLeaf, let cy = c.yoga {
            let yi = p.children[..<i].reduce(0) { $0 + (nodes[$1]?.yoga != nil ? 1 : 0) }
            YGNodeInsertChild(py, cy, yi)
        }
        dirtyText(c)
    }

    /// Detaches `child` from `parent`. The node (and its subtree) is kept:
    /// the renderer may insert it again (README: Host interface).
    public func remove(parent: Int32, child: Int32) throws {
        _ = try get(parent)
        let c = try get(child)
        guard c.parent == parent else { throw MutationError("remove: \(child) is not a child of \(parent)") }
        dirtyText(c)
        detach(c)
    }

    /// Frees a node the bundle will not use again (detaching it first if
    /// attached). Its children stay, detached, until released themselves.
    public func release(_ tag: Int32) throws {
        let n = try get(tag)
        if tag == rootTag { throw MutationError("release: the root cannot be released") }
        dirtyText(n)
        detach(n)
        for t in n.children {
            guard let c = nodes[t] else { continue }
            if let cy = c.yoga, let owner = YGNodeGetOwner(cy) { YGNodeRemoveChild(owner, cy) }
            c.parent = nil
        }
        nodes.removeValue(forKey: tag)
        dirtyStyle.remove(tag)
        if let y = n.yoga { YGNodeFree(y) }
    }

    /// Lays out and publishes a snapshot.
    @discardableResult
    public func commit() throws -> LayoutSnapshot {
        let t0 = ContinuousClock.now
        defer { commitTime += ContinuousClock.now - t0 }
        guard let r = rootTag, let root = nodes[r], let ry = root.yoga else { throw MutationError("commit: no root") }
        for t in dirtyStyle {
            guard let n = nodes[t], let y = n.yoga else { continue }
            YogaStyle.apply(n.layout, to: y, isScroll: n.type == "scroll")
        }
        dirtyStyle.removeAll()
        let ty = ContinuousClock.now
        YGNodeCalculateLayout(ry, .nan, .nan, .LTR)
        yogaTime += ContinuousClock.now - ty
        commits += 1
        // What each attached leaf draws, re-derived from its final content
        // width (Yoga may have measured it several times, and cached).
        var haptics: [LayoutSnapshot.HapticFired] = []
        walkAttached(root) { n in
            if n.isLeaf, let y = n.yoga {
                let w = YGNodeLayoutGetWidth(y) - YGNodeLayoutGetPadding(y, .left) - YGNodeLayoutGetPadding(y, .right)
                    - YGNodeLayoutGetBorder(y, .left) - YGNodeLayoutGetBorder(y, .right)
                n.measured = measureLeaf(n, maxWidth: Double(max(0, w)))
                n.shown = n.type == "text" ? "" : displayText(n)
            }
            if n.type == "haptic" {
                let t = n.props["trigger"] ?? .null
                if let prev = n.hapticPrev, t != .null, t != prev {
                    haptics.append(.init(tag: n.tag, pattern: n.haptic.pattern))
                }
                n.hapticPrev = .some(t)
            }
        }
        let snap = buildSnapshot(root: root, haptics: haptics)
        snapshot = snap
        onCommit?(snap)
        return snap
    }

    /// Re-runs layout (after `setRootSize` or `tick`) and publishes.
    @discardableResult
    public func relayout() throws -> LayoutSnapshot { try commit() }

    /// Time passed: re-derives every countdown / clock text. Returns true if
    /// any changed; then `relayout()` to measure and redraw them.
    public func tick() -> Bool {
        var changed = false
        for n in nodes.values where (n.type == "countdown" || n.type == "clock") && n.measured != nil {
            if displayText(n) != n.shown {
                if let y = n.yoga { YGNodeMarkDirty(y) }
                changed = true
            }
        }
        return changed
    }

    /// When the next countdown / clock text changes (nil if none will).
    public func nextTick() -> Date? {
        let t = now()
        var best: Date?
        func consider(_ d: Date) { if best == nil || d < best! { best = d } }
        for n in nodes.values where n.measured != nil {
            if n.type == "clock" {
                let t = wallClock()
                let unit: Calendar.Component = n.clock.format == .hourMinuteSecond24 ? .second : .minute
                if let next = calendar.nextDate(after: t, matching: unit == .second ? DateComponents(nanosecond: 0)
                                                 : DateComponents(second: 0), matchingPolicy: .nextTime) {
                    consider(next)
                }
            } else if n.type == "countdown", let toMs = n.countdown.toMs, let anchor = n.anchor {
                let target = anchor.addingTimeInterval(toMs / 1000)
                let rem = target.timeIntervalSince(t) * 1000
                switch n.countdown.format {
                case .countdown, .remaining:
                    // The shown second (rounded up) changes at each whole second before the target.
                    if rem > 0 {
                        let k = (rem / 1000).rounded(.up) - 1
                        consider(target.addingTimeInterval(-k).addingTimeInterval(0.001))
                    }
                case .elapsed:
                    let e = rem < 0 ? (-rem / 1000).rounded(.down) + 1 : 1
                    consider(target.addingTimeInterval(e).addingTimeInterval(0.001))
                }
            }
        }
        return best
    }

    // MARK: Internals

    private func get(_ tag: Int32) throws -> Node {
        guard let n = nodes[tag] else { throw MutationError("unknown tag \(tag)") }
        return n
    }

    private func detach(_ c: Node) {
        guard let pt = c.parent, let p = nodes[pt] else { return }
        p.children.removeAll { $0 == c.tag }
        if let cy = c.yoga, let owner = YGNodeGetOwner(cy) { YGNodeRemoveChild(owner, cy) }
        c.parent = nil
    }

    /// Content of a text leaf changed: dirty the outermost text-ish node above.
    private func dirtyText(_ n: Node) {
        var leaf: Node?
        var x: Node? = n
        while let c = x {
            if c.isLeaf { leaf = c } else if c.type != "#text" { break }
            x = c.parent.flatMap { nodes[$0] }
        }
        if let y = leaf?.yoga, YGNodeHasMeasureFunc(y) { YGNodeMarkDirty(y) }
    }

    private func walkAttached(_ n: Node, _ f: (Node) -> Void) {
        f(n)
        for t in n.children { if let c = nodes[t] { walkAttached(c, f) } }
    }

    // MARK: Text

    private static let styleKeys = ["fontSize", "fontWeight", "tabular", "lineHeight", "color", "textAlign"]

    private func styleProps(_ n: Node, _ inherited: TextStyleProps?) -> TextStyleProps {
        var s = inherited ?? TextStyleProps()
        for k in ShadowTree.styleKeys {
            if let v = n.props[k] { _ = s.set(k, v) } else if inherited == nil { _ = s.set(k, .null) }
        }
        return s
    }

    private func style(_ s: TextStyleProps) -> TextStyle {
        TextStyle(fontSize: s.fontSize, fontWeight: s.fontWeight, tabular: s.tabular, lineHeight: s.lineHeight,
                  color: registry.color(s.color) ?? .white, textAlign: s.textAlign)
    }

    func runs(_ n: Node) -> [TextRun] {
        if n.type == "countdown" || n.type == "clock" {
            let s = displayText(n)
            return s.isEmpty ? [] : [TextRun(s, style(styleProps(n, nil)))]
        }
        var out: [TextRun] = []
        func walk(_ node: Node, _ inherited: TextStyleProps?) {
            let s = styleProps(node, inherited)
            let st = style(s)
            for t in node.children {
                guard let c = nodes[t] else { continue }
                if c.type == "#text" { out.append(TextRun(c.text, st)) } else if c.type == "text" { walk(c, s) }
            }
        }
        walk(n, nil)
        return out
    }

    func displayText(_ n: Node) -> String {
        if n.type == "countdown" {
            guard let toMs = n.countdown.toMs, let anchor = n.anchor else { return "" }
            let remaining = TimeFormat.jsRound(toMs - now().timeIntervalSince(anchor) * 1000)
            return TimeFormat.countdown(n.countdown.format, remainingMs: remaining)
        }
        return TimeFormat.clock(n.clock.format, date: wallClock(), calendar: calendar)
    }

    private func measureLeaf(_ n: Node, maxWidth: Double) -> MeasuredText {
        let fontSize: Double
        if case .number(let f)? = n.props["fontSize"] { fontSize = f } else { fontSize = TextStyleProps().fontSize }
        return TextMeasure.measure(measurer, runs(n), fontSize: fontSize,
                                   numberOfLines: Int(n.textLayout.numberOfLines), minScale: n.textLayout.minScale,
                                   maxWidth: maxWidth)
    }

    fileprivate func measure(_ n: Node, _ w: Float, _ wm: YGMeasureMode, _ h: Float, _ hm: YGMeasureMode) -> YGSize {
        let maxW = wm == .undefined || w.isNaN ? Double.infinity : Double(w)
        if n.isLeaf {
            let m = measureLeaf(n, maxWidth: maxW)
            return YGSize(width: Float(m.width), height: Float(m.height))
        }
        let maxH = hm == .undefined || h.isNaN ? CGFloat.infinity : CGFloat(h)
        let s = registry.views[n.type]?.measure?(n.props, CGSize(width: CGFloat(maxW), height: maxH)) ?? .zero
        return YGSize(width: Float(s.width), height: Float(s.height))
    }

    /// The drawn lines of a leaf: the measurer's breaks when it reports them.
    private func drawnLines(_ runs: [TextRun], _ m: MeasuredText, maxWidth: Double, single: Bool) -> [[TextRun]] {
        let scaled = runs.map { r -> TextRun in
            var s = r.style
            s.fontSize *= m.scale
            if let lh = s.lineHeight { s.lineHeight = lh * m.scale }
            return TextRun(r.text, s)
        }
        if single || m.lines <= 1 { return m.lines == 0 ? [] : [scaled] }
        guard let ranges = measurer.lines(runs, scale: m.scale, maxWidth: maxWidth).ranges else { return [scaled] }
        var out: [[TextRun]] = []
        let total = scaled.reduce(0) { $0 + $1.text.utf16.count }
        for (i, r) in ranges.prefix(m.lines).enumerated() {
            let end = (i == m.lines - 1 && m.truncated) ? total : r.upperBound
            out.append(slice(scaled, r.lowerBound..<end))
        }
        return out
    }

    private func slice(_ runs: [TextRun], _ range: Range<Int>) -> [TextRun] {
        var out: [TextRun] = []
        var pos = 0
        for r in runs {
            let u = Array(r.text.utf16)
            let lo = max(range.lowerBound, pos), hi = min(range.upperBound, pos + u.count)
            if lo < hi {
                var t = String(decoding: u[(lo - pos)..<(hi - pos)], as: UTF16.self)
                if t.hasSuffix("\n") { t.removeLast() }
                out.append(TextRun(t.replacingOccurrences(of: "\n", with: " "), r.style))
            }
            pos += u.count
        }
        return out
    }

    // MARK: Snapshot

    private func buildSnapshot(root: Node, haptics: [LayoutSnapshot.HapticFired]) -> LayoutSnapshot {
        var out: [Int32: LayoutNode] = [:]
        func go(_ n: Node) {
            guard let y = n.yoga else { return }
            let frame = CGRect(x: CGFloat(YGNodeLayoutGetLeft(y)), y: CGFloat(YGNodeLayoutGetTop(y)),
                               width: CGFloat(YGNodeLayoutGetWidth(y)), height: CGFloat(YGNodeLayoutGetHeight(y)))
            func edge(_ e: YGEdge) -> CGFloat { CGFloat(YGNodeLayoutGetPadding(y, e) + YGNodeLayoutGetBorder(y, e)) }
            let insets = LayoutNode.Insets(top: edge(.top), left: edge(.left), bottom: edge(.bottom), right: edge(.right))
            var kids: [Int32] = []
            var leaf: TextLeaf?
            var content: CGFloat = 0
            if n.isLeaf {
                if let m = n.measured {
                    let r = runs(n)
                    let single = n.textLayout.minScale < 1 || Int(n.textLayout.numberOfLines) == 1
                    let maxW = Double(max(0, frame.width - insets.left - insets.right))
                    let align = styleProps(n, nil).textAlign
                    leaf = TextLeaf(runs: r, measured: m, lines: drawnLines(r, m, maxWidth: maxW, single: single), align: align)
                }
            } else {
                for t in n.children {
                    guard let c = nodes[t], c.yoga != nil else { continue }
                    kids.append(t)
                    go(c)
                }
            }
            if n.type == "scroll" {
                var bottom: CGFloat = 0
                for t in kids {
                    guard let cy = nodes[t]?.yoga else { continue }
                    bottom = max(bottom, CGFloat(YGNodeLayoutGetTop(cy) + YGNodeLayoutGetHeight(cy) + YGNodeLayoutGetMargin(cy, .bottom)))
                }
                content = max(frame.height, bottom + insets.bottom)
            }
            var paint = Paint()
            paint.background = registry.color(n.box.background)
            paint.borderColor = registry.color(n.box.borderColor)
            paint.borderWidth = CGFloat(n.layout.borderWidth)
            paint.borderRadius = CGFloat(n.box.borderRadius)
            paint.opacity = n.box.opacity
            paint.clips = n.type == "scroll" || n.layout.overflow == .hidden
            out[n.tag] = LayoutNode(
                tag: n.tag, type: n.type, frame: frame, children: kids, props: n.props, paint: paint,
                text: leaf, contentInsets: insets, contentHeight: content,
                button: n.type == "button" ? n.button : nil)
        }
        go(root)
        return LayoutSnapshot(rootTag: root.tag, nodes: out, sequence: commits, haptics: haptics)
    }
}

private let measureFunc: YGMeasureFunc = { yn, w, wm, h, hm in
    guard let yn, let ctx = YGNodeGetContext(yn) else { return YGSize(width: 0, height: 0) }
    let n = Unmanaged<ShadowTree.Node>.fromOpaque(ctx).takeUnretainedValue()
    return n.tree.measure(n, w, wm, h, hm)
}
