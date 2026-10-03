// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "ShouciCore",
    platforms: [.macOS(.v14)],
    products: [.library(name: "ShouciCore", targets: ["ShouciCore"])],
    targets: [
        // The Rust core: static library, C header, and module map, made by
        // scripts/build-core.sh.
        .binaryTarget(name: "ShouciFFI", path: "ShouciFFI.xcframework"),
        // The core as Swift: generated bindings plus Swift conveniences.
        .target(name: "ShouciCore", dependencies: ["ShouciFFI"]),
        .testTarget(name: "ShouciCoreTests", dependencies: ["ShouciCore"]),
    ]
)
