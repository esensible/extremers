import Foundation
import SwiftUI
#if os(watchOS)
import WatchKit
#endif

/// The app-facing object: owns a `JSHost` + `ShadowTree` for one bundle,
/// diffs each committed snapshot into per-node view models (`store`) for
/// `MountView`, ticks countdowns and clocks, and plays haptics.
///
/// Loading is the app's job: hand it bundle source (and the hash you expect)
/// and feed it states. The bundle mounts once `MountView` reports its size;
/// states that arrive earlier are kept (latest wins) until then.
@MainActor
public final class SolidWatchSession: ObservableObject {
    /// Set when the bundle failed; sticky until the next `load`.
    @Published public private(set) var error: JSHostError?
    /// Per-node view models.
    public let store = NodeStore()
    /// The latest committed snapshot.
    public private(set) var snapshot: LayoutSnapshot?

    public let registry: NodeRegistry
    public var hostOptions: JSHost.Options
    public var treeOptions: ShadowTree.Options
    /// Text measurement (Core Text by default).
    public var makeMeasurer: () -> TextMeasurer = { CoreTextMeasurer() }
    /// `console.*` output and host diagnostics.
    public var onLog: ((String) -> Void)?
    /// Called after each commit has been applied to the view models.
    public var onCommit: ((LayoutSnapshot) -> Void)?
    /// Plays a haptic pattern. Defaults to `WKInterfaceDevice.play`.
    public var playHaptic: (HapticPattern) -> Void = SolidWatchSession.defaultHaptic

    public private(set) var host: JSHost?
    private var viewport: CGSize?
    private var pendingState: (JSData, Date)?
    private var tickTimer: Timer?

    public init(registry: NodeRegistry = NodeRegistry(), hostOptions: JSHost.Options = .init(),
                treeOptions: ShadowTree.Options = .init()) {
        self.registry = registry
        self.hostOptions = hostOptions
        self.treeOptions = treeOptions
    }

    /// Loads (replacing any current bundle) and, if the view size is known,
    /// mounts. Errors are published on `error`.
    public func load(_ source: Data, filename: String = "bundle.js", expectedSHA256: String? = nil) {
        unload()
        let tree = ShadowTree(registry: registry, measurer: makeMeasurer(), options: treeOptions)
        let h = JSHost(tree: tree, options: hostOptions)
        h.onLog = { [weak self] m in self?.onLog?(m) }
        tree.onWarning = { [weak self] m in self?.onLog?(m) }
        tree.onCommit = { [weak self] snap in
            MainActor.assumeIsolated { self?.committed(snap) }
        }
        host = h
        guard attempt({ try h.load(source, filename: filename, expectedSHA256: expectedSHA256) }) else { return }
        mountIfReady()
    }

    /// Delivers a state to the bundle (`__app.onState`).
    public func setState(_ state: JSData, receivedAt: Date = Date()) {
        guard error == nil, let h = host else { return }
        guard h.rootTag != nil else { pendingState = (state, receivedAt); return }
        attempt { try h.onState(state, receivedAt: receivedAt) }
        statesSinceGC += 1
        if statesSinceGC >= gcEveryStates { collectGarbage() }
    }

    /// States between garbage collections (each GC lets the renderer's
    /// FinalizationRegistry release nodes JS no longer reaches).
    public var gcEveryStates = 20
    private var statesSinceGC = 0

    /// Runs a JS GC now and the `release` calls it produces.
    public func collectGarbage() {
        statesSinceGC = 0
        guard error == nil, let h = host else { return }
        attempt { try h.collectGarbage() }
    }

    /// Sends an event to the bundle (`__app.dispatch`).
    public func dispatch(tag: Int32, name: String, payload: JSData = .null) {
        guard error == nil, let h = host, h.rootTag != nil else { return }
        attempt { _ = try h.dispatch(tag: tag, name: name, payload: payload) }
    }

    /// The view's size: mounts on the first call, re-lays out on changes.
    public func setViewport(_ size: CGSize) {
        guard size.width > 0, size.height > 0, size != viewport else { return }
        viewport = size
        if let h = host, h.rootTag != nil, error == nil {
            h.tree.setRootSize(width: size.width, height: size.height)
            attempt { try h.tree.relayout() }
        } else {
            mountIfReady()
        }
    }

    public func unload() {
        tickTimer?.invalidate()
        tickTimer = nil
        host = nil
        snapshot = nil
        error = nil
        pendingState = nil
        store.reset()
    }

    // MARK: Button presses

    /// A tap on `node` (after its confirm sheet, if any): the native press
    /// handler, then `press` to JS if `onPress` is set (README: button).
    func press(_ node: LayoutNode, tappedAt: Date) {
        if let name = node.button?.press {
            if let handler = registry.pressHandlers[name] {
                handler(PressEvent(tag: node.tag, handler: name, props: node.props, tappedAt: tappedAt))
            } else {
                onLog?("no press handler registered as \"\(name)\"")
            }
        }
        if node.button?.onPress == true {
            let ago = (Date().timeIntervalSince(tappedAt) * 1000).rounded()
            dispatch(tag: node.tag, name: "press", payload: .object(["agoMs": .number(max(0, ago))]))
        }
    }

    // MARK: Internals

    private func committed(_ snap: LayoutSnapshot) {
        snapshot = snap
        store.apply(snap)
        for h in snap.haptics { playHaptic(h.pattern) }
        scheduleTick()
        onCommit?(snap)
    }

    private func scheduleTick() {
        tickTimer?.invalidate()
        tickTimer = nil
        guard let tree = host?.tree, let next = tree.nextTick() else { return }
        let t = Timer(fire: next, interval: 0, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, self.error == nil, let tree = self.host?.tree else { return }
                let changed = tree.tick()
                for h in tree.tickHaptics { self.playHaptic(h.pattern) }
                if changed {
                    self.attempt { try tree.relayout() }
                } else {
                    self.scheduleTick()
                }
            }
        }
        t.tolerance = 0.02
        RunLoop.main.add(t, forMode: .common)
        tickTimer = t
    }

    private func mountIfReady() {
        guard error == nil, let h = host, h.isLoaded, h.rootTag == nil, let v = viewport else { return }
        guard attempt({ try h.mount(width: v.width, height: v.height) }) else { return }
        if let (s, at) = pendingState {
            pendingState = nil
            attempt { try h.onState(s, receivedAt: at) }
        }
    }

    @discardableResult
    private func attempt(_ f: () throws -> Void) -> Bool {
        do {
            try f()
            return true
        } catch let e as JSHostError {
            error = e
        } catch {
            self.error = .engine("\(error)")
        }
        tickTimer?.invalidate()
        if let e = error { onLog?("solid-watch: \(e)") }
        return false
    }

    nonisolated public static func defaultHaptic(_ p: HapticPattern) {
        #if os(watchOS)
        let t: WKHapticType
        switch p {
        case .notification: t = .notification
        case .directionUp: t = .directionUp
        case .directionDown: t = .directionDown
        case .success: t = .success
        case .failure: t = .failure
        case .retry: t = .retry
        case .start: t = .start
        case .stop: t = .stop
        case .click: t = .click
        }
        WKInterfaceDevice.current().play(t)
        #endif
    }
}
