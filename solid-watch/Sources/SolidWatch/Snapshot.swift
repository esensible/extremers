import CoreGraphics
import Foundation

/// What a text leaf (outermost `text`, `countdown`, `clock`) draws.
public struct TextLeaf: Equatable, Sendable {
    /// All runs, styles inherited, at the leaf's own sizes (scale 1).
    public let runs: [TextRun]
    public let measured: MeasuredText
    /// The drawn lines (runs at the measured scale), at most `measured.lines`.
    public let lines: [[TextRun]]
    public let align: TextAlign
}

/// One node of a committed tree: Yoga frame plus resolved props. Immutable.
public struct LayoutNode: Equatable, Sendable {
    public let tag: Int32
    public let type: String
    /// Relative to the parent's frame origin (border box).
    public let frame: CGRect
    /// Laid-out children in order (spans and raw text are part of `text`).
    public let children: [Int32]
    /// Raw props as last set by the bundle (app props included).
    public let props: [String: PropValue]
    public let paint: Paint
    /// Text leaves only.
    public let text: TextLeaf?
    /// Padding + border on each side.
    public let contentInsets: Insets
    /// `scroll`: the height of what it scrolls.
    public let contentHeight: CGFloat
    /// `button`'s decoded props.
    public let button: ButtonProps?

    public struct Insets: Hashable, Sendable {
        public var top, left, bottom, right: CGFloat
    }

    /// The content box, in the node's own coordinates.
    public var contentRect: CGRect {
        let i = contentInsets
        return CGRect(x: i.left, y: i.top, width: max(0, frame.width - i.left - i.right),
                      height: max(0, frame.height - i.top - i.bottom))
    }
}

/// An immutable, laid-out tree published on every commit.
public struct LayoutSnapshot: Sendable {
    public let rootTag: Int32
    public let nodes: [Int32: LayoutNode]
    /// 1 for the first commit, then counts up.
    public let sequence: Int
    /// `haptic` nodes whose trigger changed at this commit: (tag, pattern).
    public let haptics: [HapticFired]

    public struct HapticFired: Hashable, Sendable {
        public let tag: Int32
        public let pattern: HapticPattern
    }

    public var root: LayoutNode? { nodes[rootTag] }
    public subscript(tag: Int32) -> LayoutNode? { nodes[tag] }

    /// Depth-first, parent before children.
    public func walk(_ visit: (LayoutNode, Int) -> Void) {
        func go(_ t: Int32, _ depth: Int) {
            guard let n = nodes[t] else { return }
            visit(n, depth)
            for c in n.children { go(c, depth + 1) }
        }
        go(rootTag, 0)
    }

    /// One line per node: `<indent><type>#<tag> x y w h` (frames relative to
    /// the parent).
    public func frameDump() -> String {
        var out: [String] = []
        walk { n, d in
            let f = n.frame
            var line = String(repeating: "  ", count: d) + "\(n.type)#\(n.tag) \(f.minX) \(f.minY) \(f.width) \(f.height)"
            if let t = n.text {
                line += " fontSize \(t.measured.fontSize) lines \(t.measured.lines)"
                let s = t.lines.map { $0.map(\.text).joined() }.joined(separator: "⏎")
                line += " \"\(s)\""
            }
            if n.type == "scroll" { line += " content \(n.contentHeight)" }
            out.append(line)
        }
        return out.joined(separator: "\n")
    }
}
