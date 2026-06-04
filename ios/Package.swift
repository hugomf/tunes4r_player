// swift-tools-version: 5.9
import PackageDescription

let package = Package(
  name: "tunes4r_player",
  platforms: [
    .iOS("13.0"),
  ],
  products: [
    .library(
      name: "tunes4r_player",
      type: .static,
      targets: ["tunes4r_player"]
    ),
  ],
  targets: [
    .binaryTarget(
      name: "libtunes4r",
      path: "Frameworks/libtunes4r.xcframework"
    ),
    .target(
      name: "tunes4r_player",
      dependencies: ["libtunes4r"],
      resources: [],
      linkerSettings: [
        .linkedFramework("AVFoundation"),
        .linkedFramework("AudioToolbox"),
        .linkedFramework("CoreAudio"),
        .linkedFramework("Security"),
        .linkedFramework("CoreFoundation"),
      ]
    ),
  ]
)
