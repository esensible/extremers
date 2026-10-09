// swift-tools-version:5.9
//
// bitwire (working name): the Swift interpreter for the schema-driven wire
// codec. See SPEC.md. MIT licensed; no dependencies beyond Foundation.

import PackageDescription

let package = Package(
    name: "Wire",
    platforms: [.watchOS(.v9), .iOS(.v16), .macOS(.v13)],
    products: [
        .library(name: "Wire", targets: ["Wire"]),
    ],
    targets: [
        .target(name: "Wire", path: "swift/Sources/Wire"),
        .testTarget(name: "WireTests", dependencies: ["Wire"], path: "swift/Tests/WireTests"),
    ]
)
