import SwiftUI

@main
struct ExtremersApp: App {
  @StateObject private var computer = RaceComputer()
  @Environment(\.scenePhase) private var scenePhase

  var body: some Scene {
    WindowGroup {
      ContentView()
        .environmentObject(computer)
        .onAppear { computer.start() }
    }
    .onChange(of: scenePhase) { _, phase in
      computer.appActive(phase == .active)
    }
  }
}
