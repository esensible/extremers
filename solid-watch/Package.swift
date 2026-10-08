// swift-tools-version:5.9
// solid-watch: a native host for Solid (universal renderer) UIs on watchOS.
// QuickJS-ng runs the JS bundle, Yoga lays out the node tree, SwiftUI draws it.
import Foundation
import PackageDescription

// CQuickJS is a build output, not in the repository: stop with what to run
// rather than SwiftPM's "does not contain a binary artifact".
let quickJS = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
    .appendingPathComponent("Frameworks/CQuickJS.xcframework/Info.plist")
if !FileManager.default.fileExists(atPath: quickJS.path) {
    fatalError("""
        solid-watch: Frameworks/CQuickJS.xcframework is missing. It is built, not \
        committed: in solid-watch/, run `git submodule update --init quickjs-ng` (from \
        the repository root: `git submodule update --init`), then \
        `sh scripts/build-quickjs-xcframework.sh`. See README.md, "Why QuickJS is prebuilt".
        """)
}

let package = Package(
    name: "SolidWatch",
    platforms: [.watchOS(.v10), .iOS(.v17), .macOS(.v14)],
    products: [
        .library(name: "SolidWatch", targets: ["SolidWatch"]),
    ],
    dependencies: [
        // facebook/yoga's own package: the C API (module `yoga`) over its
        // C++ sources, built with the app.
        .package(url: "https://github.com/facebook/yoga.git", exact: "3.2.1"),
    ],
    targets: [
        // quickjs-ng core only: no qjs/qjsc CLI and no quickjs-libc (fork,
        // system, signals, worker threads are not available on watchOS).
        // Prebuilt and always optimised, whatever the app's configuration:
        // at -O0 the interpreter's frames overflow the engine's stack limit.
        // scripts/build-quickjs-xcframework.sh builds it from the
        // quickjs-ng submodule.
        .binaryTarget(
            name: "CQuickJS",
            path: "Frameworks/CQuickJS.xcframework"
        ),
        // C glue between QuickJS and Swift: the JSValue representation
        // differs between arm64 (struct) and arm64_32 (NaN-boxed uint64), so
        // Swift never touches JSValue directly.
        .target(
            name: "CSolidHost",
            dependencies: ["CQuickJS"]
        ),
        .target(
            name: "SolidWatch",
            dependencies: ["CSolidHost", .product(name: "yoga", package: "yoga")]
        ),
        // Reads the package's fixtures/ by path (tests run on the Mac or the simulator).
        .testTarget(
            name: "SolidWatchTests",
            dependencies: ["SolidWatch"]
        ),
    ]
)
