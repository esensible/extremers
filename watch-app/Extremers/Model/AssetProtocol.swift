import CryptoKit
import Foundation
import HostProtocol

// The box's asset channel (BLE.md, "Bundles"): the watch asks for a
// package's watch bundle by its hash and the box streams it back from flash.
// Pure Swift, no CoreBluetooth: the tests feed it bytes in any chunking.

/// Watch -> box, on the asset channel: postcard `GetAsset { package:
/// PackageId, hash: [u8; 8] }` (the generated binding), `hash` being the
/// bundle's as the envelope's `ui.watch` names it.
typealias AssetRequest = GetAsset

extension GetAsset {
  var bytes: [UInt8] { encodeGetAsset(self) }
}

/// Box -> watch, before the asset's bytes: postcard `AssetHeader { status:
/// u8, len: u32 }` (the generated binding).
extension AssetHeader {
  /// `len` bytes follow.
  static let ok: UInt8 = 0
  /// The box has no such package.
  static let unknownPackage: UInt8 = 1
  /// That hash is not the package's current bundle.
  static let notCurrent: UInt8 = 2

  var bytes: [UInt8] { encodeAssetHeader(self) }
}

/// Why a bundle could not be fetched.
enum AssetError: Error, Equatable {
  /// The link is not up (or went down): fetched again when it is back.
  case linkDown
  /// The box has no `ui-psm` (firmware without the asset channel), or its
  /// value is not a PSM.
  case noChannel(String)
  /// The L2CAP channel did not open.
  case openFailed(String)
  /// Nothing arrived for too long.
  case timeout(received: Int, of: Int?)
  /// The channel closed before the whole asset arrived.
  case shortRead(received: Int, of: Int?)
  /// The box answered with a non-zero status.
  case refused(status: UInt8)
  /// Not the protocol (a bad header, bytes past the asset).
  case malformed(String)
  /// More than the watch keeps.
  case tooLarge(Int)
  /// The bytes are not the bundle asked for.
  case hashMismatch(got: [UInt8])
  case stream(String)

  /// One line for the log and the error screen.
  var detail: String {
    switch self {
    case .linkDown: return "the link is down"
    case .noChannel(let why): return "the box has no bundle channel (\(why))"
    case .openFailed(let why): return "the bundle channel did not open (\(why))"
    case .timeout(let n, let total): return "timed out after \(n)\(total.map { " of \($0)" } ?? "") B"
    case .shortRead(let n, let total): return "the channel closed after \(n)\(total.map { " of \($0)" } ?? "") B"
    case .refused(let s):
      switch s {
      case AssetHeader.unknownPackage: return "the box has no such package"
      case AssetHeader.notCurrent: return "the box has another version of it now"
      default: return "the box refused it (status \(s))"
      }
    case .malformed(let why): return "bad answer (\(why))"
    case .tooLarge(let n): return "too large (\(n) B)"
    case .hashMismatch(let got): return "wrong bytes (hash \(ShellLog.hex(got)))"
    case .stream(let why): return "channel error (\(why))"
    }
  }
}

enum AssetHash {
  /// `ui.watch` of a package without a watch bundle (`NO_WATCH_BUNDLE`,
  /// all zeros; never an actual bundle's hash): nothing to fetch.
  static let none = [UInt8](repeating: 0, count: 8)

  /// The first 8 bytes of SHA-256: what `ui.watch` names a bundle by.
  static func prefix(_ data: Data) -> [UInt8] {
    Array(SHA256.hash(data: data).prefix(8))
  }

  /// Full SHA-256, hex (what solid-watch checks a bundle against on load).
  static func hex(_ data: Data) -> String {
    SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
  }
}

/// Reads one response off the channel's byte stream, whatever its chunking:
/// the header (a status byte and a varint length, so 2 to 6 bytes, possibly
/// split across chunks), then exactly `len` bytes. Bytes past those are a
/// protocol error (one request is outstanding at a time).
struct AssetResponseParser {
  /// The most a response may carry.
  let limit: Int
  private(set) var header: AssetHeader?
  private var pending: [UInt8] = []
  private(set) var body = Data()

  init(limit: Int) {
    self.limit = limit
  }

  /// Bytes of the asset received so far.
  var received: Int { body.count }
  /// The asset's length, once the header is in.
  var total: Int? { header.map { Int($0.len) } }
  /// The whole asset has arrived.
  var isComplete: Bool { header.map { body.count == Int($0.len) } ?? false }

  /// Takes the next chunk. Throws on a refusal, a malformed header, an
  /// asset over the limit, or bytes past the asset.
  mutating func feed(_ chunk: [UInt8]) throws {
    var rest = chunk[...]
    if header == nil {
      pending += rest
      rest = []
      var r = PostcardReader(pending)
      let h: AssetHeader
      do {
        h = try AssetHeader(from: &r)
      } catch PostcardError.endOfData {
        // A varint's bytes are at most 5: a longer run is no header.
        if pending.count >= 6 { throw AssetError.malformed("header \(ShellLog.hex(pending))") }
        return
      } catch {
        throw AssetError.malformed("header \(ShellLog.hex(pending)): \(error)")
      }
      header = h
      rest = pending[r.index...]
      pending = []
      if h.status != AssetHeader.ok { throw AssetError.refused(status: h.status) }
      if Int(h.len) > limit { throw AssetError.tooLarge(Int(h.len)) }
      body.reserveCapacity(Int(h.len))
    }
    guard let header else { return }
    let room = Int(header.len) - body.count
    if rest.count > room {
      throw AssetError.malformed("\(rest.count - room) B past the \(header.len) B asset")
    }
    body.append(contentsOf: rest)
  }
}
