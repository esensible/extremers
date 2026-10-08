import CYoga
import CoreGraphics
import Foundation

/// Applies the generated `LayoutProps` to a Yoga node: every prop at once,
/// like the browser host, so a reset prop goes back to its default.
enum YogaStyle {
    static func apply(_ p: LayoutProps, to y: YGNodeRef, isScroll: Bool) {
        YGNodeStyleSetDisplay(y, p.display == .none ? YGDisplay.none : .flex)
        YGNodeStyleSetPositionType(y, p.position == .absolute ? .absolute : .relative)
        for (d, e) in [(p.top, YGEdge.top), (p.right, .right), (p.bottom, .bottom), (p.left, .left)] {
            switch d {
            case .points(let v): YGNodeStyleSetPosition(y, e, Float(v))
            case .percent(let v): YGNodeStyleSetPositionPercent(y, e, Float(v))
            case .auto: YGNodeStyleSetPositionAuto(y, e)
            case .undefined: YGNodeStyleSetPosition(y, e, .nan)
            }
        }
        dimension(p.width, y, YGNodeStyleSetWidth, YGNodeStyleSetWidthPercent, YGNodeStyleSetWidthAuto)
        dimension(p.height, y, YGNodeStyleSetHeight, YGNodeStyleSetHeightPercent, YGNodeStyleSetHeightAuto)
        dimension(p.minWidth, y, YGNodeStyleSetMinWidth, YGNodeStyleSetMinWidthPercent, nil)
        dimension(p.minHeight, y, YGNodeStyleSetMinHeight, YGNodeStyleSetMinHeightPercent, nil)
        dimension(p.maxWidth, y, YGNodeStyleSetMaxWidth, YGNodeStyleSetMaxWidthPercent, nil)
        dimension(p.maxHeight, y, YGNodeStyleSetMaxHeight, YGNodeStyleSetMaxHeightPercent, nil)
        YGNodeStyleSetAspectRatio(y, p.aspectRatio.map(Float.init) ?? .nan)
        YGNodeStyleSetFlex(y, p.flex.map(Float.init) ?? .nan)
        YGNodeStyleSetFlexGrow(y, Float(p.flexGrow))
        YGNodeStyleSetFlexShrink(y, Float(p.flexShrink))
        dimension(p.flexBasis, y, YGNodeStyleSetFlexBasis, YGNodeStyleSetFlexBasisPercent, YGNodeStyleSetFlexBasisAuto)
        YGNodeStyleSetFlexDirection(y, {
            switch p.flexDirection {
            case .column: return .column
            case .row: return .row
            case .columnReverse: return .columnReverse
            case .rowReverse: return .rowReverse
            }
        }())
        YGNodeStyleSetFlexWrap(y, {
            switch p.flexWrap {
            case .nowrap: return .noWrap
            case .wrap: return .wrap
            case .wrapReverse: return .wrapReverse
            }
        }())
        YGNodeStyleSetJustifyContent(y, {
            switch p.justifyContent {
            case .flexStart: return .flexStart
            case .center: return .center
            case .flexEnd: return .flexEnd
            case .spaceBetween: return .spaceBetween
            case .spaceAround: return .spaceAround
            case .spaceEvenly: return .spaceEvenly
            }
        }())
        YGNodeStyleSetAlignItems(y, align(p.alignItems.rawValue))
        YGNodeStyleSetAlignSelf(y, align(p.alignSelf.rawValue))
        YGNodeStyleSetAlignContent(y, align(p.alignContent.rawValue))
        let edges: [(Dimension, Dimension, YGEdge)] = [
            (p.margin, p.padding, .all),
            (p.marginHorizontal, p.paddingHorizontal, .horizontal),
            (p.marginVertical, p.paddingVertical, .vertical),
            (p.marginTop, p.paddingTop, .top),
            (p.marginRight, p.paddingRight, .right),
            (p.marginBottom, p.paddingBottom, .bottom),
            (p.marginLeft, p.paddingLeft, .left),
        ]
        for (m, pad, e) in edges {
            switch m {
            case .points(let v): YGNodeStyleSetMargin(y, e, Float(v))
            case .percent(let v): YGNodeStyleSetMarginPercent(y, e, Float(v))
            case .auto: YGNodeStyleSetMarginAuto(y, e)
            case .undefined: YGNodeStyleSetMargin(y, e, .nan)
            }
            switch pad {
            case .points(let v): YGNodeStyleSetPadding(y, e, Float(v))
            case .percent(let v): YGNodeStyleSetPaddingPercent(y, e, Float(v))
            case .auto, .undefined: YGNodeStyleSetPadding(y, e, .nan)
            }
        }
        YGNodeStyleSetBorder(y, .all, Float(p.borderWidth))
        YGNodeStyleSetGap(y, .all, p.gap.map(Float.init) ?? .nan)
        YGNodeStyleSetGap(y, .row, p.rowGap.map(Float.init) ?? .nan)
        YGNodeStyleSetGap(y, .column, p.columnGap.map(Float.init) ?? .nan)
        let overflow: YGOverflow
        if isScroll {
            overflow = .scroll
        } else {
            switch p.overflow {
            case .visible: overflow = .visible
            case .hidden: overflow = .hidden
            case .scroll: overflow = .scroll
            }
        }
        YGNodeStyleSetOverflow(y, overflow)
        YGNodeStyleSetBoxSizing(y, p.boxSizing == .contentBox ? .contentBox : .borderBox)
    }

    private static func align(_ s: String) -> YGAlign {
        switch s {
        case "auto": return .auto
        case "flex-start": return .flexStart
        case "center": return .center
        case "flex-end": return .flexEnd
        case "stretch": return .stretch
        case "baseline": return .baseline
        case "space-between": return .spaceBetween
        case "space-around": return .spaceAround
        case "space-evenly": return .spaceEvenly
        default: return .auto
        }
    }

    private static func dimension(
        _ d: Dimension, _ y: YGNodeRef,
        _ set: (YGNodeRef, Float) -> Void, _ percent: (YGNodeRef, Float) -> Void, _ auto: ((YGNodeRef) -> Void)?
    ) {
        switch d {
        case .points(let v): set(y, Float(v))
        case .percent(let v): percent(y, Float(v))
        case .auto: if let auto { auto(y) } else { set(y, .nan) }
        case .undefined: set(y, .nan)
        }
    }
}

/// An sRGB color with alpha, components 0...1.
public struct RGBA: Hashable, Sendable {
    public var r, g, b, a: Double
    public init(r: Double, g: Double, b: Double, a: Double = 1) {
        self.r = r; self.g = g; self.b = b; self.a = a
    }

    public init(rgba v: UInt32) {
        self.init(r: Double((v >> 24) & 0xff) / 255, g: Double((v >> 16) & 0xff) / 255,
                  b: Double((v >> 8) & 0xff) / 255, a: Double(v & 0xff) / 255)
    }

    public static let white = RGBA(r: 1, g: 1, b: 1)
}

/// Box paint of a node, resolved.
public struct Paint: Hashable, Sendable {
    public var background: RGBA?
    public var borderColor: RGBA?
    public var borderWidth: CGFloat = 0
    public var borderRadius: CGFloat = 0
    public var opacity: Double = 1
    /// `overflow: hidden` (and `scroll`): clip children to the box.
    public var clips = false
    public init() {}
}

/// The text style of a run, after inheritance (README: text style keys
/// fontSize, fontWeight, tabular, lineHeight, color, textAlign).
public struct TextStyle: Hashable, Sendable {
    public var fontSize: Double
    public var fontWeight: FontWeight
    public var tabular: Bool
    /// Points; nil = fontSize x the font's line-height ratio.
    public var lineHeight: Double?
    public var color: RGBA
    public var textAlign: TextAlign

    public init(fontSize: Double = 16, fontWeight: FontWeight = .normal, tabular: Bool = false,
                lineHeight: Double? = nil, color: RGBA = .white, textAlign: TextAlign = .left) {
        self.fontSize = fontSize
        self.fontWeight = fontWeight
        self.tabular = tabular
        self.lineHeight = lineHeight
        self.color = color
        self.textAlign = textAlign
    }
}

extension FontWeight {
    /// CSS numeric weight.
    public var numeric: Int {
        switch self {
        case .normal: return 400
        case .bold: return 700
        default: return Int(rawValue) ?? 400
        }
    }
}
