#
# tunes4r iOS podspec
#
# The Rust static library (libtunes4r.a) must be built before publishing.
# Run: make build-ios
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
  s.platform         = :ios, '13.0'
  s.static_framework = true

  # Vendored Rust static library (device arch only; use an XCFramework for
  # simulator+device in release builds).
  s.vendored_libraries = 'Frameworks/libtunes4r.a'

  # Frameworks required by the Rust engine
  s.frameworks = 'AVFoundation', 'AudioToolbox', 'CoreAudio', 'Security', 'CoreFoundation'

  # Tell the linker to force-load the static lib so all extern "C" symbols
  # are visible to DynamicLibrary.process() from Dart.
  s.xcconfig = {
    'OTHER_LDFLAGS' => '-force_load $(PODS_TARGET_SRCROOT)/ios/Frameworks/libtunes4r.a',
  }
end
