import Foundation
import HealthKit

/// Keeps the app, and so its BLE link, running with the wrist down: watchOS
/// suspends an app (and its Bluetooth) when the screen sleeps unless a
/// workout session is running (BLE.md "On the watch"). A sailing workout
/// runs from `start()` (the app starts looking for the race computer, so
/// it is not suspended mid-scan) to `stop()` (Quit).
///
/// watchOS runs one workout session at a time, so another workout app can
/// end this one. While wanted, an ended session is started again
/// (`WorkoutRestart` decides when) and `onNotice` says so.
///
/// No workout builder: nothing is recorded to Health, the session only keeps
/// the app in the foreground-equivalent state.
@MainActor
final class WorkoutKeeper: NSObject {
  /// A short line for the screen when the session was ended from outside.
  var onNotice: ((String) -> Void)?

  private let store = HKHealthStore()
  private var session: HKWorkoutSession?
  /// `session` reached running (it was not refused at start).
  private var sessionRan = false
  /// `session` replaces one that was ended from outside.
  private var sessionIsRestart = false

  private enum Authorization { case notAsked, asking, answered }
  private var authorization = Authorization.notAsked
  /// Between `start()` and `stop()`: a session should be running.
  private var wanted = false
  private var appActive = true
  /// Start a session the next time the app is in front.
  private var restartWhenActive = false
  private var lastRestart: Date?

  /// Starts the session unless one is running. Asks for HealthKit permission
  /// (to share workouts, which a session needs) the first time.
  func start() {
    wanted = true
    guard session == nil, HKHealthStore.isHealthDataAvailable() else { return }
    switch authorization {
    case .answered:
      begin()
    case .asking:
      // The answer starts it, if still wanted then.
      break
    case .notAsked:
      authorization = .asking
      let share: Set<HKSampleType> = [HKObjectType.workoutType()]
      store.requestAuthorization(toShare: share, read: nil) { _, _ in
        Task { @MainActor in self.authorizationAnswered() }
      }
    }
  }

  func stop() {
    wanted = false
    restartWhenActive = false
    let s = session
    session = nil
    s?.end()
  }

  /// The app came to the front or left it.
  func appActive(_ active: Bool) {
    appActive = active
    guard active, wanted, restartWhenActive, session == nil else { return }
    restartWhenActive = false
    restart()
  }

  private func authorizationAnswered() {
    authorization = .answered
    // Quit while the permission sheet was up: no workout after all.
    guard wanted else { return }
    begin()
  }

  private func begin() {
    guard wanted, session == nil else { return }
    let configuration = HKWorkoutConfiguration()
    configuration.activityType = .sailing
    configuration.locationType = .outdoor
    do {
      let s = try HKWorkoutSession(healthStore: store, configuration: configuration)
      s.delegate = self
      session = s
      sessionRan = false
      sessionIsRestart = false
      s.startActivity(with: Date())
    } catch {
      // Not authorized, or HealthKit unavailable: the app still works while
      // the screen is on.
      session = nil
    }
  }

  private func restart() {
    lastRestart = Date()
    begin()
    if session != nil { sessionIsRestart = true }
  }

  private func running(_ s: HKWorkoutSession) {
    guard session === s else { return }
    sessionRan = true
    if sessionIsRestart {
      sessionIsRestart = false
      onNotice?("Workout restarted")
    }
  }

  /// `s` ended or failed, without `stop()`.
  private func ended(_ s: HKWorkoutSession) {
    guard session === s else { return }
    let ran = sessionRan
    let wasRestart = sessionIsRestart
    session = nil
    sessionRan = false
    sessionIsRestart = false
    let decision = WorkoutRestart.after(
      wanted: wanted,
      ran: ran,
      appActive: appActive,
      sinceLastRestart: lastRestart.map { Date().timeIntervalSince($0) }
    )
    switch decision {
    case .none:
      break
    case .now:
      restart()
    case .whenActive:
      restartWhenActive = true
      // Seen only if the app is in front; otherwise the restart, when the
      // sailor comes back, says "Workout restarted".
      if ran {
        onNotice?("Workout ended elsewhere")
      } else if wasRestart {
        onNotice?("Workout could not restart: the link drops with the wrist down")
      }
    }
  }
}

// HealthKit calls these on a queue of its own.
extension WorkoutKeeper: HKWorkoutSessionDelegate {
  nonisolated func workoutSession(
    _ workoutSession: HKWorkoutSession,
    didChangeTo toState: HKWorkoutSessionState,
    from fromState: HKWorkoutSessionState,
    date: Date
  ) {
    switch toState {
    case .running:
      Task { @MainActor in self.running(workoutSession) }
    case .ended:
      Task { @MainActor in self.ended(workoutSession) }
    default:
      break
    }
  }

  nonisolated func workoutSession(_ workoutSession: HKWorkoutSession, didFailWithError error: Error) {
    Task { @MainActor in self.ended(workoutSession) }
  }
}
