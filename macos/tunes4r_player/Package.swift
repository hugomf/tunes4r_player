// swift-tools-version: 5.9
//
// tunes4r macOS Package.swift
//
// The Rust XCFramework (libtunes4r.xcframework) must be built before publishing.
// Run: make build-macos
//
// Flutter's plugin discovery looks for this file at
// `<plugin>/macos/<plugin_name>/Package.swift` (see
// `plugin.pluginSwiftPackageManifestPath` in flutter_tools). The actual
// XCFramework lives in the sibling `../Frameworks/` directory, which the
// build script populates.
//
// `FlutterFramework` is the local SwiftPM package that Flutter's host-app
// SPM integration generates next to this one (it links the Flutter engine
// frameworks). Declaring it satisfies Flutter's plugin SPM-support check;
// the actual Swift code in `Classes/Tunes4rPlayerPlugin.swift` is compiled
// by the host app's Runner target, not by this package.
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
    dependencies: [
        .package(name: "FlutterFramework", path: "../FlutterFramework"),
    ],
    targets: [
        .binaryTarget(
            name: "tunes4r_player",
            path: "../Frameworks/libtunes4r.xcframework"
        )
    ]
)
