// SPDX-License-Identifier: MIT

/// Every refusal, with a `code` from SPEC §7.
public struct WireError: Error, Equatable, CustomStringConvertible, Sendable {
    public enum Code: String, Sendable {
        /// Load: the descriptor fails SPEC §6.3.
        case descriptor
        /// Decode: the message ends before the value does.
        case truncated
        /// Decode: bytes after the end of the value.
        case trailing
        /// Decode: malformed wire data.
        case invalid
        /// Encode: wrong type, missing required property or union tag.
        case shape
        /// Encode: right type but cannot be written.
        case range
    }

    public let code: Code
    public let message: String
    /// Encode only: JSON pointer to the offending value ("" for the root).
    public internal(set) var path: String

    init(_ code: Code, _ message: String, path: String = "") {
        self.code = code
        self.message = message
        self.path = path
    }

    public var description: String {
        switch code {
        case .shape, .range:
            return "\(code.rawValue): \(path.isEmpty ? "the root" : path): \(message)"
        default:
            return "\(code.rawValue): \(message)"
        }
    }

    /// Adds a parent key (object field or array index) in front of `path`.
    mutating func prepend(_ key: String) {
        var k = ""
        for u in key.unicodeScalars {
            switch u {
            case "~": k += "~0"
            case "/": k += "~1"
            default: k.unicodeScalars.append(u)
            }
        }
        path = "/" + k + path
    }
}
