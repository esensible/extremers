import CoreGraphics
import Foundation
import SwiftUI

/// What a custom node's view gets.
public struct CustomNodeContext {
    public let node: LayoutNode
    /// The node's children, already laid out by Yoga inside its frame.
    public let children: AnyView
    /// Sends an event to the bundle: `__app.dispatch(tag, name, payload)`.
    public let dispatch: (String, JSData) -> Void

    /// Decodes the node's props into a generated props struct (fragment
    /// codegen: `var p = XProps(); _ = p.set(name, value)` per prop).
    public func decode<P>(_ empty: P, _ set: (inout P, String, PropValue) -> PropSetResult) -> P {
        var p = empty
        for (k, v) in node.props { _ = set(&p, k, v) }
        return p
    }
}

/// A button press, as delivered to a native press handler.
public struct PressEvent {
    public let tag: Int32
    /// The button's `press` prop: the handler name.
    public let handler: String
    /// All of the button's props (including ones an app fragment adds).
    public let props: [String: PropValue]
    /// When the user tapped (before any confirm sheet), on the host's
    /// local clock.
    public let tappedAt: Date
    /// The same tap on the device timeline (README: Time): `tappedAt` in
    /// epoch ms plus the clock offset from the bundle's latest `syncClock`
    /// (0 before the first one). A handler that tells the device when
    /// something happened sends this, not the local time.
    public let tappedAtDeviceMs: Double
}

/// App extensions (Fabric-components style): schema fragments (new node
/// types, props on core types, colors), the views for new node types, and
/// native press handlers. Configure it before loading a bundle; main thread.
public final class NodeRegistry {
    struct CustomView {
        let measure: (([String: PropValue], CGSize) -> CGSize)?
        let view: (CustomNodeContext) -> AnyView
    }

    private(set) var nodes: [String: NodeSpec] = CoreSchema.nodes
    private(set) var colors: [String: String] = CoreSchema.colors
    private(set) var views: [String: CustomView] = [:]
    private(set) var pressHandlers: [String: (PressEvent) -> Void] = [:]

    public init() {}

    /// Adds a schema fragment, as its generated Swift exposes it
    /// (`XSchema.nodes`, `XSchema.extends`, `XSchema.colors`).
    public func addSchema(nodes new: [String: NodeSpec] = [:], extends: [String: [String: PropSpec]] = [:],
                          colors: [String: String] = [:]) {
        for (k, v) in new {
            precondition(nodes[k] == nil, "node type \(k) defined twice")
            nodes[k] = v
        }
        for (type, props) in extends {
            guard let n = nodes[type] else { preconditionFailure("extends: unknown node type \(type)") }
            var merged = n.props
            for (k, v) in props {
                precondition(merged[k] == nil, "\(type).\(k) defined twice")
                merged[k] = v
            }
            nodes[type] = NodeSpec(type: n.type, children: n.children, native: n.native, layout: n.layout, props: merged)
        }
        for (k, v) in colors {
            precondition(self.colors[k] == nil, "color \(k) defined twice")
            self.colors[k] = v
        }
    }

    /// Adds a schema fragment from its JSON source (the same file the codegen
    /// reads): `colors`, `extends.<type>.props`, and `nodes` (with `groups`
    /// from the core schema and own `props`). For tools and demos that load
    /// fragments at run time; apps normally pass their generated table.
    public func addSchema(fragmentJSON: String) throws {
        let f = try JSData(json: fragmentJSON)
        let name = f["name"]?.string ?? "fragment"
        func prop(_ key: String, _ v: JSData, group: String) throws -> PropSpec {
            guard let kind = v["kind"]?.string.flatMap(PropKind.init(rawValue:)),
                  let type = v["type"]?.string.flatMap(PropType.init(rawValue:)) else {
                throw MutationError("fragment \(name): bad prop \(key)")
            }
            var values: [String] = []
            if case .array(let a)? = v["values"] { values = a.compactMap(\.string) }
            return PropSpec(name: key, kind: kind, type: type, group: group,
                            inherits: v["inherit"]?.bool ?? false, values: values)
        }
        func props(_ v: JSData?, group: String) throws -> [String: PropSpec] {
            guard case .object(let fields)? = v else { return [:] }
            var out: [String: PropSpec] = [:]
            for x in fields { out[x.key] = try prop(x.key, x.value, group: group) }
            return out
        }
        // Core groups, rebuilt from the core node table.
        var coreGroups: [String: [String: PropSpec]] = [:]
        for n in CoreSchema.nodes.values { for p in n.props.values { coreGroups[p.group, default: [:]][p.name] = p } }
        var groups = coreGroups
        if case .object(let gs)? = f["groups"] {
            for g in gs { groups[g.key] = try props(g.value["props"], group: g.key) }
        }
        var newNodes: [String: NodeSpec] = [:]
        if case .object(let ns)? = f["nodes"] {
            for n in ns {
                var p = try props(n.value["props"], group: n.key)
                var hasLayout = false
                if case .array(let gl)? = n.value["groups"] {
                    for g in gl.compactMap(\.string) {
                        if g == "layout" { hasLayout = true }
                        p.merge(groups[g] ?? [:]) { a, _ in a }
                    }
                }
                let children = n.value["children"]?.string.flatMap(ChildKind.init(rawValue:)) ?? ChildKind.none
                newNodes[n.key] = NodeSpec(type: n.key, children: children, native: true, layout: hasLayout, props: p)
            }
        }
        var ext: [String: [String: PropSpec]] = [:]
        if case .object(let es)? = f["extends"] {
            for e in es {
                var p = try props(e.value["props"], group: "\(name):\(e.key)")
                if case .array(let gl)? = e.value["groups"] {
                    for g in gl.compactMap(\.string) { p.merge(groups[g] ?? [:]) { a, _ in a } }
                }
                ext[e.key] = p
            }
        }
        var colors: [String: String] = [:]
        if case .object(let cs)? = f["colors"] { for c in cs { colors[c.key] = c.value.string } }
        addSchema(nodes: newNodes, extends: ext, colors: colors)
    }

    /// Registers the view of a node type added by a fragment.
    /// - measure: for a leaf whose size depends on its props; gets the max
    ///   size (infinite when unconstrained).
    /// - view: draws the node inside its Yoga frame.
    public func registerView<V: View>(
        _ type: String,
        measure: (([String: PropValue], CGSize) -> CGSize)? = nil,
        view: @escaping (CustomNodeContext) -> V
    ) {
        precondition(nodes[type]?.native == true, "\(type): add its schema fragment first (core types are built in)")
        views[type] = CustomView(measure: measure, view: { AnyView(view($0)) })
    }

    /// Registers the native handler a `button` with `press="<name>"` runs.
    public func registerPressHandler(_ name: String, _ handler: @escaping (PressEvent) -> Void) {
        pressHandlers[name] = handler
    }

    func color(_ s: String?) -> RGBA? {
        guard let s, let v = ColorParse.rgba(s, names: colors) else { return nil }
        return RGBA(rgba: v)
    }
}
