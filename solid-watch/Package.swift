// swift-tools-version:5.9
// solid-watch: a native host for Solid (universal renderer) UIs on watchOS.
// QuickJS-ng runs the JS bundle, Yoga lays out the node tree, SwiftUI draws it.
import PackageDescription

let package = Package(
    name: "SolidWatch",
    platforms: [.watchOS(.v10), .iOS(.v17), .macOS(.v14)],
    products: [
        .library(name: "SolidWatch", targets: ["SolidWatch"]),
    ],
    targets: [
        // quickjs-ng core only: no qjs/qjsc CLI and no quickjs-libc (fork,
        // system, signals, worker threads are not available on watchOS).
        .target(
            name: "CQuickJS",
            exclude: ["LICENSE"],
            cSettings: [
                .define("_GNU_SOURCE"),
                .define("NDEBUG", .when(configuration: .release)),
            ]
        ),
        // facebook/yoga C++ sources; the C API headers are public under
        // include/yoga, the internal C++ headers stay private under yoga/.
        .target(
            name: "CYoga",
            exclude: ["LICENSE"],
            cxxSettings: [
                .headerSearchPath("."),
                .define("NDEBUG", .when(configuration: .release)),
            ]
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
            dependencies: ["CSolidHost", "CYoga"]
        ),
        // Reads ../../fixtures by path (tests run on the Mac or the simulator).
        .testTarget(
            name: "SolidWatchTests",
            dependencies: ["SolidWatch"]
        ),
    ],
    cxxLanguageStandard: .cxx20
)
