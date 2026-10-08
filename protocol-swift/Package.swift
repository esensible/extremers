// swift-tools-version:5.9
// The host protocol in Swift (tools/bindgen generates Sources/), for the
// watch app. `swift test` checks it against the Rust encoder's golden
// vectors (protocol-js/test/vectors.json).
import PackageDescription

let package = Package(
    name: "HostProtocol",
    platforms: [.watchOS(.v10), .iOS(.v17), .macOS(.v14)],
    products: [.library(name: "HostProtocol", targets: ["HostProtocol"])],
    targets: [
        .target(name: "HostProtocol"),
        .testTarget(name: "HostProtocolTests", dependencies: ["HostProtocol"]),
    ]
)
