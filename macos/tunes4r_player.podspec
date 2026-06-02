#
# tunes4r macOS podspec
#
# The Rust dylib (libtunes4r.dylib) must be built before publishing.
# Run: make build-macos
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

  # Vendored Rust dynamic library
  s.vendored_libraries = 'Frameworks/libtunes4r.dylib'

  # Frameworks required by the Rust engine
  s.frameworks = 'AVFoundation', 'AudioToolbox', 'CoreAudio', 'Security', 'CoreFoundation'

  # Ensure the dylib is properly signed during code-signing phase
  s.script_phases = [
    {
      :name => 'Sign libtunes4r.dylib',
      :script => 'codesign --force --sign - "${PODS_TARGET_SRCROOT}/macos/Frameworks/libtunes4r.dylib" 2>/dev/null || true',
      :execution_position => :after_compile,
    }
  ]
end
