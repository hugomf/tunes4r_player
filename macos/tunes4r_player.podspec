#
# tunes4r macOS podspec
#
# The Rust XCFramework (libtunes4r.xcframework) must be built before publishing.
# Run: make build-macos
#
# This podspec is a CocoaPods fallback for hosts that haven't opted into
# Swift Package Manager. The SPM path is defined in macos/Package.swift
# and activated by `swift_package_manager: true` in the plugin's pubspec.
#

Pod::Spec.new do |s|
  s.name             = 'tunes4r_player'
  s.version          = '0.1.0'
  s.summary          = 'Rust-powered audio playback engine for Flutter'
  s.description      = 'Tunes4R is a cross-platform audio engine with MP3/FLAC/AAC/Opus decoding, HTTP streaming, YouTube audio extraction, and real-time FFT spectrum analysis.'
  s.homepage         = 'https://github.com/hugomf/tunes4r_player'
  s.license          = { :type => 'MIT' }
  s.author           = { 'hugomf' => 'hugo@example.com' }
  s.source           = { :path => '.' }
  s.platform         = :macos, '10.15'
  s.static_framework = false

  # Vendored Rust dynamic library, packaged as a universal XCFramework
  # (arm64 + x86_64). The same artifact is consumed by Swift Package
  # Manager via macos/Package.swift.
  s.vendored_frameworks = 'Frameworks/libtunes4r.xcframework'

  # Frameworks required by the Rust engine
  s.frameworks = 'AVFoundation', 'AudioToolbox', 'CoreAudio', 'Security', 'CoreFoundation'
end
