// swift-tools-version: 5.9
//
// tunes4r macOS Package.swift
//
// The Rust dylib (libtunes4r.dylib) must be built before publishing.
// Run: make build-macos
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
            targets: ["tunes4r_player"]
        )
    ],
    targets: [
        .binaryTarget(
            name: "tunes4r_player",
            path: "XCFrameworks/libtunes4r.xcframework"
        )
    ]
)
