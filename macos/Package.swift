// swift-tools-version: 5.9
//
// tunes4r macOS Package.swift
//
// The Rust XCFramework (libtunes4r.xcframework) must be built before publishing.
// Run: make build-macos
//
// This Package.swift is consumed by Flutter's macOS Swift Package Manager
// integration: it provides a `tunes4r-player` product that the host app
// (example/macos/Runner) links against. The underlying dylib lives inside
// the XCFramework's libtunes4r.framework bundle.
//

import PackageDescription

let package = Package(
    name: "tunes4r_player",
    platforms: [
        .macOS("10.15")
    ],
    products: [
        .library(
            name: "tunes4r-player",
            type: .dynamic,
            targets: ["tunes4r_player"]
        )
    ],
    targets: [
        .binaryTarget(
            name: "tunes4r_player",
            path: "Frameworks/libtunes4r.xcframework"
        )
    ]
)
