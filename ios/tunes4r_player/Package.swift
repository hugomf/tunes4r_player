// swift-tools-version: 5.9
//
// tunes4r iOS Package.swift
//
// The Rust XCFramework (libtunes4r.xcframework) must be built before publishing.
// Run: make build-ios
//

import PackageDescription

let package = Package(
    name: "tunes4r_player",
    platforms: [
        .iOS("13.0")
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
            path: "libtunes4r.xcframework"
        )
    ]
)
