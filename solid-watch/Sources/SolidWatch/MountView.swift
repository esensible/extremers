import SwiftUI

/// Draws a session's tree: every node at its Yoga frame (ZStack topLeading +
/// frame + offset). Each node view observes only its own `NodeModel`, so a
/// commit re-evaluates only the nodes that changed. Shows the error screen
/// when the bundle failed.
public struct MountView: View {
    @ObservedObject var session: SolidWatchSession
    @ObservedObject var store: NodeStore

    public init(session: SolidWatchSession) {
        self.session = session
        store = session.store
    }

    public var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .topLeading) {
                if let e = session.error {
                    SolidErrorView(error: e)
                } else if let root = store.root {
                    // Tags are unique only within one mounted tree: a new
                    // bundle (or a remount) numbers its nodes from the same
                    // tags again. Keyed by the root's model, each mount gets
                    // fresh views rather than the last tree's views fed new
                    // models.
                    NodeView(model: root, ctx: MountContext(session: session))
                        .id(ObjectIdentifier(root))
                }
            }
            .frame(width: geo.size.width, height: geo.size.height, alignment: .topLeading)
            .onAppear { session.setViewport(geo.size) }
            .onChange(of: geo.size) { _, size in session.setViewport(size) }
        }
    }
}

/// The error screen for a failed bundle.
public struct SolidErrorView: View {
    public let error: JSHostError
    public init(error: JSHostError) { self.error = error }

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 6) {
                Text(error.title).font(.headline).foregroundColor(.red)
                Text(error.description)
                    .font(.system(size: 11, design: .monospaced))
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// Shared, identity-stable inputs of every node view.
struct MountContext: Equatable {
    unowned let session: SolidWatchSession
    var store: NodeStore { session.store }

    static func == (a: MountContext, b: MountContext) -> Bool { a.session === b.session }
}

extension RGBA {
    var color: Color { Color(.sRGB, red: r, green: g, blue: b, opacity: a) }
}

struct NodeView: View {
    @ObservedObject var model: NodeModel
    let ctx: MountContext

    var body: some View {
        let n = model.node
        let _ = ctx.store.noteBody()
        content(n)
            .frame(width: n.frame.width, height: n.frame.height, alignment: .topLeading)
            .modifier(PaintModifier(paint: n.paint))
            .offset(x: n.frame.minX, y: n.frame.minY)
    }

    @ViewBuilder
    private func content(_ n: LayoutNode) -> some View {
        switch n.type {
        case "text":
            TextLeafView(node: n)
        case "scroll":
            ScrollView(.vertical, showsIndicators: false) {
                ChildrenView(node: n, ctx: ctx)
                    .frame(width: n.frame.width, height: n.contentHeight, alignment: .topLeading)
            }
            .frame(width: n.frame.width, height: n.frame.height)
        case "button":
            ButtonNodeView(node: n, session: ctx.session) { ChildrenView(node: n, ctx: ctx) }
        default:
            if let v = ctx.session.registry.views[n.type] {
                v.view(CustomNodeContext(
                    node: n, children: AnyView(ChildrenView(node: n, ctx: ctx)),
                    dispatch: { [weak session = ctx.session] name, payload in
                        session?.dispatch(tag: n.tag, name: name, payload: payload)
                    }))
            } else {
                ChildrenView(node: n, ctx: ctx)
            }
        }
    }
}

struct ChildrenView: View {
    let node: LayoutNode
    let ctx: MountContext

    var body: some View {
        ZStack(alignment: .topLeading) {
            ForEach(node.children, id: \.self) { t in
                if let m = ctx.store.model(t) { NodeView(model: m, ctx: ctx) }
            }
        }
        .frame(width: node.frame.width, height: node.type == "scroll" ? node.contentHeight : node.frame.height,
               alignment: .topLeading)
    }
}

struct PaintModifier: ViewModifier {
    let paint: Paint

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: paint.borderRadius, style: .continuous)
        let painted = content
            .background(paint.background.map { shape.fill($0.color) })
            .overlay {
                if let bc = paint.borderColor, paint.borderWidth > 0 {
                    shape.strokeBorder(bc.color, lineWidth: paint.borderWidth)
                }
            }
        if paint.clips {
            painted.clipShape(shape).opacity(paint.opacity)
        } else {
            painted.opacity(paint.opacity)
        }
    }
}

// MARK: - text

func styledText(_ runs: [TextRun]) -> Text {
    runs.reduce(Text("")) { acc, r in
        acc + Text(r.text).font(Font(Fonts.ctFont(r.style, size: r.style.fontSize)))
            .foregroundColor(r.style.color.color)
    }
}

extension TextAlign {
    var horizontal: HorizontalAlignment {
        switch self {
        case .left: return .leading
        case .center: return .center
        case .right: return .trailing
        }
    }

    var frame: Alignment {
        switch self {
        case .left: return .topLeading
        case .center: return .top
        case .right: return .topTrailing
        }
    }
}

/// Draws the lines the shadow tree measured, one `Text` per line (so the
/// breaks are the measured ones), each in a box of the measured line height.
struct TextLeafView: View {
    let node: LayoutNode

    var body: some View {
        if let t = node.text {
            let rect = node.contentRect
            VStack(alignment: t.align.horizontal, spacing: 0) {
                ForEach(Array(t.lines.enumerated()), id: \.offset) { i, line in
                    let last = i == t.lines.count - 1
                    if last && t.measured.truncated {
                        styledText(line).lineLimit(1).truncationMode(.tail)
                            .frame(maxWidth: rect.width, alignment: t.align.frame)
                            .frame(height: t.measured.lineHeight)
                    } else {
                        styledText(line).lineLimit(1).fixedSize()
                            .frame(height: t.measured.lineHeight)
                    }
                }
            }
            .frame(width: rect.width, alignment: t.align.frame)
            .offset(x: rect.minX, y: rect.minY)
        }
    }
}

// MARK: - button

struct PressStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.opacity(configuration.isPressed ? 0.6 : 1)
    }
}

struct ButtonNodeView<Label: View>: View {
    let node: LayoutNode
    let session: SolidWatchSession
    @ViewBuilder let label: () -> Label

    var body: some View {
        let disabled = node.button?.disabled ?? false
        Button {
            session.press(node)
        } label: {
            label().contentShape(Rectangle())
        }
        .buttonStyle(PressStyle())
        .disabled(disabled)
        .opacity(disabled ? 0.35 : 1)
    }
}
