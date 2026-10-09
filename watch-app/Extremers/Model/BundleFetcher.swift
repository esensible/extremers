import Foundation
import HostProtocol

/// A byte channel to the box's asset server: on the watch an L2CAP
/// connection-oriented channel (RaceComputer); in tests and the simulator a
/// scripted one. Main actor throughout.
@MainActor
protocol AssetChannel: AnyObject {
  /// For the log: "l2cap", ...
  var transport: String { get }
  /// For the log: the channel's parameters (PSM, ATT MTU, ...).
  var parameters: String { get }
  /// The bytes from the box, in whatever chunks they come; finishes when
  /// the box closes the channel, throws on a channel error.
  var incoming: AsyncThrowingStream<[UInt8], Error> { get }
  /// Queues bytes for the box.
  func write(_ bytes: [UInt8])
  /// Ends `incoming` with `error` (the fetch's idle timer), and closes.
  func fail(_ error: Error)
  /// Closes the channel. Idempotent.
  func close()
}

/// What opens asset channels: the race computer's link.
@MainActor
protocol AssetLink: AnyObject {
  /// A new channel to the box's asset server. Throws `AssetError`
  /// (`.linkDown` when the link is not up) or `CancellationError`.
  func openAssetChannel() async throws -> AssetChannel
}

/// One fetch's numbers, for the log.
struct FetchStats: Equatable {
  var transport = ""
  var parameters = ""
  var openMs = 0
  var totalMs = 0
  var bytes = 0
  var chunks = 0
  var maxChunk = 0

  /// "30080 B in 812 ms (open 120 ms; 37.0 kB/s; 27 chunks, max 1247 B)"
  var summary: String {
    let transferMs = max(1, totalMs - openMs)
    let rate = Double(bytes) / Double(transferMs)
    return "\(bytes) B in \(totalMs) ms (open \(openMs) ms; \(String(format: "%.1f", rate)) kB/s; "
      + "\(chunks) chunks, max \(maxChunk) B)"
  }
}

/// Fetches one bundle over one channel: open, send the request, read the
/// header and exactly `len` bytes, verify the hash prefix. Gives up when
/// nothing arrives for `idleTimeout`.
@MainActor
struct BundleFetcher {
  var idleTimeout: TimeInterval = 5
  var limit = BundleCache.defaultCap

  /// The verified bytes. `progress` gets (received, total) as bytes come.
  /// `stats` is filled in as far as the fetch got, failed or not. `opened`
  /// gets the channel once open, so whoever abandons the fetch can close it
  /// at once rather than when the cancelled task unwinds.
  func fetch(_ request: AssetRequest, over link: AssetLink, stats: inout FetchStats,
             opened: (AssetChannel) -> Void = { _ in },
             progress: (Int, Int?) -> Void) async throws -> Data {
    let clock = ContinuousClock()
    let t0 = clock.now
    func ms(_ d: Duration) -> Int { Int(d.components.seconds * 1000 + d.components.attoseconds / 1_000_000_000_000_000) }
    defer { stats.totalMs = ms(clock.now - t0) }
    let channel = try await link.openAssetChannel()
    defer { channel.close() }
    opened(channel)
    stats.transport = channel.transport
    stats.parameters = channel.parameters
    stats.openMs = ms(clock.now - t0)
    try Task.checkCancellation()

    let state = ReadState(parser: AssetResponseParser(limit: limit), lastActivity: clock.now)
    // The idle timer: fails the channel once nothing came for idleTimeout.
    let timeout = Duration.milliseconds(Int(idleTimeout * 1000))
    let watchdog = Task { @MainActor [weak channel] in
      while !Task.isCancelled {
        let idle = clock.now - state.lastActivity
        if idle >= timeout {
          channel?.fail(AssetError.timeout(received: state.parser.received, of: state.parser.total))
          return
        }
        try? await Task.sleep(for: timeout - idle)
      }
    }
    defer { watchdog.cancel() }

    channel.write(request.bytes)
    progress(0, nil)
    do {
      for try await chunk in channel.incoming {
        state.lastActivity = clock.now
        stats.chunks += 1
        stats.maxChunk = max(stats.maxChunk, chunk.count)
        try state.parser.feed(chunk)
        stats.bytes = state.parser.received
        progress(state.parser.received, state.parser.total)
        if state.parser.isComplete { break }
      }
    } catch let e as AssetError {
      throw e
    } catch {
      try Task.checkCancellation()
      throw AssetError.stream("\(error)")
    }
    try Task.checkCancellation()
    guard state.parser.isComplete else {
      throw AssetError.shortRead(received: state.parser.received, of: state.parser.total)
    }
    let got = AssetHash.prefix(state.parser.body)
    guard got == request.hash else { throw AssetError.hashMismatch(got: got) }
    return state.parser.body
  }

  /// The read's state, shared with its idle timer.
  @MainActor
  private final class ReadState {
    var parser: AssetResponseParser
    var lastActivity: ContinuousClock.Instant

    init(parser: AssetResponseParser, lastActivity: ContinuousClock.Instant) {
      self.parser = parser
      self.lastActivity = lastActivity
    }
  }
}

/// What to do after a failed fetch (`attempt` is the one that failed, from 1).
enum FetchPolicy: Equatable {
  /// Fetch again after `delay` seconds.
  case retry(delay: TimeInterval)
  /// Wait for the link to come back, then fetch again (no attempt used).
  case waitForLink
  /// The box has no watch bundle for the package.
  case missing
  /// Give up: the error screen (Try again, Exit).
  case fail

  static let transportAttempts = 3

  static func after(_ error: AssetError, attempt: Int) -> FetchPolicy {
    switch error {
    case .linkDown:
      return .waitForLink
    case .refused(AssetHeader.unknownPackage):
      return .missing
    case .hashMismatch, .refused:
      // Once more: the box may have moved on (a new envelope comes then),
      // or a bad transfer. Then the error screen.
      return attempt < 2 ? .retry(delay: 1) : .fail
    case .noChannel, .tooLarge:
      return .fail
    case .openFailed, .timeout, .shortRead, .malformed, .stream:
      return attempt < transportAttempts ? .retry(delay: 1) : .fail
    }
  }
}
