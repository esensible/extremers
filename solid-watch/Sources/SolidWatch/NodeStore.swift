import Foundation
import SwiftUI

/// One node's view model. Its view observes only this object, so a commit
/// re-evaluates just the views whose node (props, frame, text or child
/// list) changed.
@MainActor
public final class NodeModel: ObservableObject, Identifiable {
    public nonisolated let id: Int32
    @Published public internal(set) var node: LayoutNode

    init(_ node: LayoutNode) {
        id = node.tag
        self.node = node
    }
}

/// Per-tag view models, diffed against each committed snapshot.
@MainActor
public final class NodeStore: ObservableObject {
    /// The root's model; changes only when a new tree is mounted.
    @Published public private(set) var root: NodeModel?
    private(set) var models: [Int32: NodeModel] = [:]

    // Instrumentation (README: profiling): per apply, how many models changed;
    // and how many node view bodies SwiftUI evaluated since `resetCounters`.
    public private(set) var lastChanged = 0
    public private(set) var bodyEvaluations = 0
    public private(set) var lastBodyEvaluation: ContinuousClock.Instant?
    public private(set) var applyTime: Duration = .zero

    public init() {}

    public func model(_ tag: Int32) -> NodeModel? { models[tag] }

    /// Updates the models from `snap`; returns how many changed (incl. new).
    @discardableResult
    public func apply(_ snap: LayoutSnapshot) -> Int {
        let t0 = ContinuousClock.now
        var changed = 0
        var next: [Int32: NodeModel] = [:]
        next.reserveCapacity(snap.nodes.count)
        for (tag, n) in snap.nodes {
            if let m = models[tag] {
                if m.node != n {
                    m.node = n
                    changed += 1
                }
                next[tag] = m
            } else {
                next[tag] = NodeModel(n)
                changed += 1
            }
        }
        models = next
        if root?.id != snap.rootTag { root = next[snap.rootTag] }
        lastChanged = changed
        applyTime += ContinuousClock.now - t0
        return changed
    }

    public func reset() {
        models = [:]
        root = nil
    }

    public func resetCounters() {
        bodyEvaluations = 0
        lastBodyEvaluation = nil
        applyTime = .zero
    }

    func noteBody() {
        bodyEvaluations += 1
        lastBodyEvaluation = .now
    }
}
