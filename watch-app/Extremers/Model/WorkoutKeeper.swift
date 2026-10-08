import Foundation
import HealthKit

/// Keeps the app, and so its BLE link, running with the wrist down: watchOS
/// suspends an app (and its Bluetooth) when the screen sleeps unless a
/// workout session is running (BLE.md "On the watch"). A sailing workout
/// starts when the race computer connects and ends on `stop()`.
///
/// No workout builder: nothing is recorded to Health, the session only keeps
/// the app in the foreground-equivalent state.
@MainActor
final class WorkoutKeeper: NSObject {
  private let store = HKHealthStore()
  private var session: HKWorkoutSession?
  private var authorizationRequested = false

  /// Starts the session unless one is running. Asks for HealthKit permission
  /// (to share workouts, which a session needs) the first time.
  func start() {
    guard session == nil, HKHealthStore.isHealthDataAvailable() else { return }
    if authorizationRequested {
      begin()
      return
    }
    authorizationRequested = true
    let share: Set<HKSampleType> = [HKObjectType.workoutType()]
    store.requestAuthorization(toShare: share, read: nil) { _, _ in
      Task { @MainActor in self.begin() }
    }
  }

  func stop() {
    session?.end()
    session = nil
  }

  private func begin() {
    guard session == nil else { return }
    let configuration = HKWorkoutConfiguration()
    configuration.activityType = .sailing
    configuration.locationType = .outdoor
    do {
      let s = try HKWorkoutSession(healthStore: store, configuration: configuration)
      s.delegate = self
      s.startActivity(with: Date())
      session = s
    } catch {
      // Not authorized, or HealthKit unavailable: the app still works while
      // the screen is on.
      session = nil
    }
  }

  private func ended(_ ended: HKWorkoutSession) {
    if session === ended { session = nil }
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
    guard toState == .ended else { return }
    Task { @MainActor in self.ended(workoutSession) }
  }

  nonisolated func workoutSession(_ workoutSession: HKWorkoutSession, didFailWithError error: Error) {
    Task { @MainActor in self.ended(workoutSession) }
  }
}
