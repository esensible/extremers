import Foundation
import os

/// A persistent rolling log for field debugging: `Documents/shell.log`, at
/// most `limit` bytes, then it is moved to `shell.log.1` (one older file
/// kept) and a new one started. Lines are timestamped (UTC, ms) and also go
/// to the unified log. Writes happen on a background queue, so logging never
/// blocks the main thread.
///
/// Copy it off the watch with
///   xcrun devicectl device copy from --device <udid> --domain-type appDataContainer
///     --domain-identifier au.esensible.extremers.watch --source Documents/shell.log --destination shell.log
final class ShellLog: @unchecked Sendable {
  static let shared = ShellLog()

  let url: URL?
  private let limit: Int
  private let queue = DispatchQueue(label: "au.esensible.extremers.shell-log", qos: .utility)
  private var handle: FileHandle?
  private var size = 0
  private let logger = Logger(subsystem: "au.esensible.extremers.watch", category: "shell-log")
  private let stamp: ISO8601DateFormatter = {
    let f = ISO8601DateFormatter()
    f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return f
  }()

  init(directory: URL? = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first,
       limit: Int = 1 << 20) {
    url = directory?.appendingPathComponent("shell.log")
    self.limit = limit
  }

  func callAsFunction(_ message: @autoclosure () -> String) {
    let line = "\(stamp.string(from: Date())) \(message())"
    logger.info("\(line, privacy: .public)")
    queue.async { self.write(line + "\n") }
  }

  /// Waits for the queued lines to be written (tests).
  func flush() {
    queue.sync {}
  }

  private func write(_ line: String) {
    guard let url else { return }
    let data = Data(line.utf8)
    if handle == nil {
      if !FileManager.default.fileExists(atPath: url.path) {
        FileManager.default.createFile(atPath: url.path, contents: nil)
      }
      handle = try? FileHandle(forWritingTo: url)
      size = Int((try? handle?.seekToEnd()) ?? 0)
    }
    if size + data.count > limit {
      try? handle?.close()
      handle = nil
      let old = url.appendingPathExtension("1")
      try? FileManager.default.removeItem(at: old)
      try? FileManager.default.moveItem(at: url, to: old)
      FileManager.default.createFile(atPath: url.path, contents: nil)
      handle = try? FileHandle(forWritingTo: url)
      size = 0
    }
    try? handle?.write(contentsOf: data)
    size += data.count
  }

  static func hex(_ bytes: [UInt8]) -> String {
    bytes.map { String(format: "%02x", $0) }.joined()
  }
}

/// Logs a main-thread stall: a background timer checks every 0.5 s that a
/// heartbeat the main queue answers came back; a stall of 1 s or more is
/// logged when it starts and again with its length when the main thread
/// answers.
final class MainThreadWatchdog: @unchecked Sendable {
  private let queue = DispatchQueue(label: "au.esensible.extremers.watchdog", qos: .utility)
  private var timer: DispatchSourceTimer?
  private var lastAnswer = DispatchTime.now()
  private var pending = false
  private var stalledSince: DispatchTime?
  private let log: ShellLog

  init(log: ShellLog = .shared) {
    self.log = log
  }

  func start() {
    guard timer == nil else { return }
    let t = DispatchSource.makeTimerSource(queue: queue)
    t.schedule(deadline: .now() + 0.5, repeating: 0.5)
    t.setEventHandler { [weak self] in self?.tick() }
    t.resume()
    timer = t
  }

  private func tick() {
    let now = DispatchTime.now()
    if pending {
      let waited = Double(now.uptimeNanoseconds - lastAnswer.uptimeNanoseconds) / 1e9
      if waited >= 1, stalledSince == nil {
        stalledSince = lastAnswer
        log("main thread stalled (no answer for \(String(format: "%.1f", waited)) s)")
      }
      return
    }
    pending = true
    lastAnswer = now
    DispatchQueue.main.async { [weak self] in
      self?.queue.async {
        guard let self else { return }
        self.pending = false
        if let since = self.stalledSince {
          let s = Double(DispatchTime.now().uptimeNanoseconds - since.uptimeNanoseconds) / 1e9
          self.log("main thread answered again after \(String(format: "%.1f", s)) s")
          self.stalledSince = nil
        }
      }
    }
  }
}
