// SPDX-License-Identifier: MIT

import Foundation

extension Descriptor {
    /// Validates a descriptor held in `Data`.
    public init(_ data: Data) throws {
        try self.init(bytes: [UInt8](data))
    }

    /// Same as the free function `decode(_:_:)`.
    public func decode(_ data: Data) throws -> WireValue { try Wire.decode(self, data) }
}

/// Decodes a message held in `Data` (no copy).
public func decode(_ d: Descriptor, _ data: Data) throws -> WireValue {
    try data.withUnsafeBytes { try decode(d, $0) }
}
