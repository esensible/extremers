import CoreBluetooth
import Foundation

/// The asset channel over an L2CAP connection-oriented channel: its input
/// and output streams, scheduled on the main run loop (common modes, so a
/// scroll does not hold the transfer up). Keeps the CBL2CAPChannel alive
/// until closed: CoreBluetooth closes a channel nobody holds.
@MainActor
final class L2capAssetChannel: NSObject, AssetChannel {
  let transport = "l2cap"
  let parameters: String
  let incoming: AsyncThrowingStream<[UInt8], Error>
  private let continuation: AsyncThrowingStream<[UInt8], Error>.Continuation
  private var channel: CBL2CAPChannel?
  private var outbox: [UInt8] = []
  private var buffer = [UInt8](repeating: 0, count: 4096)

  init(_ channel: CBL2CAPChannel, parameters: String) {
    self.channel = channel
    self.parameters = parameters
    var c: AsyncThrowingStream<[UInt8], Error>.Continuation!
    incoming = AsyncThrowingStream(bufferingPolicy: .unbounded) { c = $0 }
    continuation = c
    super.init()
    for s in [channel.inputStream as Stream, channel.outputStream as Stream] {
      s.delegate = self
      s.schedule(in: .main, forMode: .common)
      s.open()
    }
  }

  func write(_ bytes: [UInt8]) {
    outbox += bytes
    flush()
  }

  private func flush() {
    guard let out = channel?.outputStream, !outbox.isEmpty, out.hasSpaceAvailable else { return }
    let n = outbox.withUnsafeBufferPointer { out.write($0.baseAddress!, maxLength: $0.count) }
    if n < 0 {
      fail(AssetError.stream("write: \(out.streamError.map { "\($0)" } ?? "failed")"))
    } else {
      outbox.removeFirst(n)
    }
  }

  private func readAvailable() {
    guard let input = channel?.inputStream else { return }
    while input.hasBytesAvailable {
      let n = input.read(&buffer, maxLength: buffer.count)
      if n < 0 {
        fail(AssetError.stream("read: \(input.streamError.map { "\($0)" } ?? "failed")"))
        return
      }
      if n == 0 { break }
      continuation.yield(Array(buffer[0..<n]))
    }
  }

  func fail(_ error: Error) {
    continuation.finish(throwing: error)
    close()
  }

  func close() {
    guard let channel else { return }
    self.channel = nil
    for s in [channel.inputStream as Stream, channel.outputStream as Stream] {
      s.delegate = nil
      s.close()
      s.remove(from: .main, forMode: .common)
    }
    continuation.finish()
  }
}

extension L2capAssetChannel: @preconcurrency StreamDelegate {
  func stream(_ s: Stream, handle event: Stream.Event) {
    switch event {
    case .hasBytesAvailable:
      readAvailable()
    case .hasSpaceAvailable:
      flush()
    case .endEncountered:
      // The box closed it (or the link went): whatever came is all.
      readAvailable()
      continuation.finish()
      close()
    case .errorOccurred:
      fail(AssetError.stream(s.streamError.map { "\($0)" } ?? "stream error"))
    default:
      break
    }
  }
}
