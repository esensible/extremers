#if DEBUG
import Foundation
import HostProtocol

/// An asset server in memory (debug builds: the tests' fake channel and the
/// simulator's box): each request is answered by a script of steps.
@MainActor
final class ScriptedAssetLink: AssetLink {
  enum Step: Equatable {
    /// These bytes, as one chunk.
    case bytes([UInt8])
    /// A pause before the next step.
    case pause(TimeInterval)
    /// The box closes the channel.
    case close
    /// Nothing more, channel left open (the fetch's idle timer ends it).
    case stall
  }

  /// The script for a request; `attempt` counts the requests so far, from 1.
  var respond: (AssetRequest, Int) -> [Step]
  /// Opening fails with this, if set.
  var openError: AssetError?
  /// Opening takes this long.
  var openDelay: TimeInterval = 0
  private(set) var requests: [AssetRequest] = []
  private(set) var opened = 0
  /// Channels not yet closed.
  private(set) var open = 0

  init(respond: @escaping (AssetRequest, Int) -> [Step]) {
    self.respond = respond
  }

  func openAssetChannel() async throws -> AssetChannel {
    opened += 1
    if openDelay > 0 { try await Task.sleep(nanoseconds: UInt64(openDelay * 1_000_000_000)) }
    if let openError { throw openError }
    open += 1
    return ScriptedAssetChannel(link: self)
  }

  fileprivate func received(_ request: AssetRequest) -> [Step] {
    requests.append(request)
    return respond(request, requests.count)
  }

  fileprivate func closed() {
    open -= 1
  }

  /// A box's answer: `bundles` by package, served when the hash is the
  /// current one (status 2 if not, 1 for an unknown package), as the header
  /// then the bytes in `chunk`-byte pieces, `pause` apart.
  static func serve(_ bundles: [PackageId: Data], chunk: Int, pause: TimeInterval = 0) -> (AssetRequest, Int) -> [Step] {
    { request, _ in
      guard let data = bundles[request.package] else { return [.bytes(AssetHeader(status: 1, len: 0).bytes)] }
      guard AssetHash.prefix(data) == request.hash else { return [.bytes(AssetHeader(status: 2, len: 0).bytes)] }
      return response(AssetHeader(status: 0, len: UInt32(data.count)).bytes + [UInt8](data), chunk: chunk, pause: pause)
    }
  }

  /// `bytes` in `chunk`-byte pieces, `pause` apart.
  static func response(_ bytes: [UInt8], chunk: Int, pause: TimeInterval = 0) -> [Step] {
    stride(from: 0, to: bytes.count, by: chunk).flatMap { i -> [Step] in
      let piece = Step.bytes(Array(bytes[i..<min(i + chunk, bytes.count)]))
      return pause > 0 ? [.pause(pause), piece] : [piece]
    }
  }
}

@MainActor
final class ScriptedAssetChannel: AssetChannel {
  let transport = "scripted"
  let parameters = "in memory"
  let incoming: AsyncThrowingStream<[UInt8], Error>
  private let continuation: AsyncThrowingStream<[UInt8], Error>.Continuation
  private weak var link: ScriptedAssetLink?
  private var request: [UInt8] = []
  private var player: Task<Void, Never>?
  private var closed = false

  init(link: ScriptedAssetLink) {
    self.link = link
    var c: AsyncThrowingStream<[UInt8], Error>.Continuation!
    incoming = AsyncThrowingStream(bufferingPolicy: .unbounded) { c = $0 }
    continuation = c
  }

  func write(_ bytes: [UInt8]) {
    request += bytes
    var r = PostcardReader(request)
    guard let package = try? r.u32(), let hash = try? r.array(8, { r in try r.u8() }) else { return }
    request = []
    let steps = link?.received(AssetRequest(package: package, hash: hash)) ?? []
    player = Task { @MainActor [weak self] in
      for step in steps {
        guard let self, !Task.isCancelled, !self.closed else { return }
        switch step {
        case .bytes(let b): self.continuation.yield(b)
        case .pause(let s): try? await Task.sleep(nanoseconds: UInt64(s * 1_000_000_000))
        case .close: self.close(); return
        case .stall: return
        }
      }
    }
  }

  func fail(_ error: Error) {
    continuation.finish(throwing: error)
    close()
  }

  func close() {
    guard !closed else { return }
    closed = true
    player?.cancel()
    continuation.finish()
    link?.closed()
  }
}
#endif
