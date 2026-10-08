import SwiftUI

@main
struct ExtremersApp: App {
  @StateObject private var computer: RaceComputer
  @StateObject private var shell: Shell
  @Environment(\.scenePhase) private var scenePhase
  /// Logs main-thread stalls to shell.log.
  private static let watchdog = MainThreadWatchdog()

  init() {
    let info = Bundle.main.infoDictionary
    ShellLog.shared("launch: Extremers \(info?["CFBundleShortVersionString"] ?? "?") (\(info?["CFBundleVersion"] ?? "?")), "
      + "bundles \(CompiledBundles.main?.names ?? [])")
    Self.watchdog.start()
    Shell.registerFont()
    let computer = RaceComputer()
    // The bundles' send(bytes) go to the box; the box's states go to the
    // shell, which runs the active package's bundle.
    let shell = Shell(source: CompiledBundles.main) { [weak computer] bytes in computer?.send(bytes) }
    computer.onState = { [weak shell] bytes, envelope in shell?.receive(bytes, envelope: envelope) }
    computer.onForget = { [weak shell] in shell?.reset() }
    _computer = StateObject(wrappedValue: computer)
    _shell = StateObject(wrappedValue: shell)
  }

  var body: some Scene {
    WindowGroup {
      NavigationStack {
        ContentView()
          .toolbar {
            ToolbarItem(placement: .topBarTrailing) { ShellMenuButton() }
          }
      }
      .environmentObject(computer)
      .environmentObject(shell)
      .onAppear {
        #if DEBUG
        // no Bluetooth or workout: the simulator's box instead
        if ProcessInfo.processInfo.arguments.contains("-SimulateBox") {
          SimulatedBox.run(computer)
          return
        }
        #endif
        computer.start()
      }
    }
    .onChange(of: scenePhase) { _, phase in
      computer.appActive(phase == .active)
    }
  }
}
