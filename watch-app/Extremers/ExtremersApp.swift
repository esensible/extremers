import SwiftUI

@main
struct ExtremersApp: App {
  @StateObject private var computer = RaceComputer()

  var body: some Scene {
    WindowGroup {
      ContentView()
        .environmentObject(computer)
        .onAppear { computer.start() }
    }
  }
}
