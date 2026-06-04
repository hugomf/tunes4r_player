#!/usr/bin/env bash
# Cross-compile the Rust native library for iOS, Android, and macOS.
#
# Usage:
#   ./scripts/build_rust.sh ios         # Build iOS static lib
#   ./scripts/build_rust.sh android     # Build Android .so libs
#   ./scripts/build_rust.sh macos       # Build macOS dylib
#   ./scripts/build_rust.sh all         # Build all platforms
#   ./scripts/build_rust.sh install     # Install cross-compilation targets
#
# After building, artifacts are copied into the plugin's platform directories:
#   ios/Frameworks/libtunes4r.a
#   ios/Frameworks/libtunes4r.xcframework
#   macos/Frameworks/libtunes4r.dylib
#   macos/Frameworks/libtunes4r.xcframework   (consumed by SPM and CocoaPods)
#   android/src/main/jniLibs/<abi>/libtunes4r.so
#
# Prerequisites:
#   - Rust toolchain (rustup)
#   - Xcode (for iOS / macOS)
#   - Android NDK 27+ (for Android)

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLUGIN_DIR="$(dirname "$SCRIPT_DIR")"
RUST_DIR="$PLUGIN_DIR/rust"

# If rust/ doesn't exist in the plugin, try the parent project
if [ ! -d "$RUST_DIR" ]; then
  RUST_DIR="$(dirname "$PLUGIN_DIR")/rust"
fi
if [ ! -d "$RUST_DIR" ]; then
  echo "ERROR: rust/ directory not found. Copy or symlink it into $PLUGIN_DIR/rust/"
  echo "  ln -s ../../rust $PLUGIN_DIR/rust"
  exit 1
fi

BUILD_TYPE="${2:-release}"
PLATFORM="${1:-all}"

echo "=== tunes4r Rust Build ==="
echo "  Plugin dir: $PLUGIN_DIR"
echo "  Rust dir:   $RUST_DIR"
echo "  Platform:   $PLATFORM"
echo "  Build:      $BUILD_TYPE"
echo ""

install_targets() {
  echo "[install] Adding rustup targets..."
  rustup target add aarch64-apple-ios aarch64-apple-ios-sim
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  rustup target add aarch64-linux-android armv7-linux-androideabi
  rustup target add x86_64-linux-android i686-linux-android
  echo "[install] Done."
}

build_ios() {
  echo "=== Building for iOS ==="
  cd "$RUST_DIR"

  local profile="release"
  [ "$BUILD_TYPE" = "debug" ] && profile="debug"

  cargo rustc --target aarch64-apple-ios --"$profile" --crate-type staticlib
  cargo rustc --target aarch64-apple-ios-sim --"$profile" --crate-type staticlib
  cargo rustc --target x86_64-apple-ios --"$profile" --crate-type staticlib

  cd "$PLUGIN_DIR"
  mkdir -p ios/Frameworks

  local device="$RUST_DIR/target/aarch64-apple-ios/$profile/libtunes4r.a"
  local sim_arm="$RUST_DIR/target/aarch64-apple-ios-sim/$profile/libtunes4r.a"
  local sim_x86="$RUST_DIR/target/x86_64-apple-ios/$profile/libtunes4r.a"

  # Combine simulator archs into one fat lib, then create XCFramework with
  # device + simulator slices so the pod works on all iOS targets.
  local sim_fat="$(mktemp -u)_libtunes4r_sim.a"
  lipo -create "$sim_arm" "$sim_x86" -output "$sim_fat"

  rm -rf ios/Frameworks/libtunes4r.xcframework
  xcodebuild -create-xcframework \
    -library "$device" \
    -library "$sim_fat" \
    -output ios/Frameworks/libtunes4r.xcframework 2>/dev/null

  rm -f "$sim_fat"

  # Keep the raw .a as a convenience fallback (device only)
  cp "$device" ios/Frameworks/libtunes4r.a

  echo "[iOS] XCFramework created at ios/Frameworks/libtunes4r.xcframework"
  echo "[iOS] Device-only .a at ios/Frameworks/libtunes4r.a"
}

build_macos() {
  echo "=== Building for macOS ==="

  local profile="release"
  [ "$BUILD_TYPE" = "debug" ] && profile="debug"

  # Ensure both Apple targets are available so the XCFramework is universal
  # (arm64 for Apple Silicon, x86_64 for Intel).
  rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null 2>&1 || true

  cd "$RUST_DIR"
  cargo build --target aarch64-apple-darwin --"$profile"
  cargo build --target x86_64-apple-darwin  --"$profile"
  cd "$PLUGIN_DIR"

  local arm_lib="$RUST_DIR/target/aarch64-apple-darwin/$profile/libtunes4r.dylib"
  local x86_lib="$RUST_DIR/target/x86_64-apple-darwin/$profile/libtunes4r.dylib"
  local out_dir="macos/Frameworks"

  if [ ! -f "$arm_lib" ] || [ ! -f "$x86_lib" ]; then
    echo "ERROR: missing built dylib(s) for macOS XCFramework"
    echo "  arm64: $arm_lib"
    echo "  x86_64: $x86_lib"
    exit 1
  fi

  mkdir -p "$out_dir"

  # 1) Build a universal XCFramework that Swift Package Manager and CocoaPods
  #    can both consume as a binaryTarget / vendored_framework.
  rm -rf "$out_dir/libtunes4r.xcframework"

  # xcodebuild can take a flat dylib directly via -library; the iOS build
  # already uses this for the .a staticlib slices, so do the same for macOS.
  if ! xcodebuild -create-xcframework \
        -library "$arm_lib" \
        -library "$x86_lib" \
        -output "$out_dir/libtunes4r.xcframework" >/dev/null 2>&1; then
    echo "ERROR: xcodebuild -create-xcframework failed; re-running with output"
    xcodebuild -create-xcframework \
        -library "$arm_lib" \
        -library "$x86_lib" \
        -output "$out_dir/libtunes4r.xcframework"
    exit 1
  fi

  # 2) Also drop a flat dylib for any tooling that loads it without an
  #    .framework wrapper (raw `DynamicLibrary.open('libtunes4r.dylib')`).
  cp "$arm_lib" "$out_dir/libtunes4r.dylib"
  install_name_tool -id "@rpath/libtunes4r.dylib" "$out_dir/libtunes4r.dylib"

  echo "[macOS] XCFramework at $out_dir/libtunes4r.xcframework (arm64 + x86_64)"
  echo "[macOS] Flat dylib at $out_dir/libtunes4r.dylib"
}

# Build a minimal macOS .framework bundle from a dylib. xcodebuild
# -create-xcframework requires Resources/Info.plist + the standard
# Versions/{A,Current} symlink layout.
#
# $1 = source dylib
# $2 = target framework directory
# $3 = arch tag used in CFBundleIdentifier (so xcodebuild treats the
#      arm64 and x86_64 wrappers as distinct input libraries)
_make_framework() {
  local dylib="$1"
  local fw="$2"
  local arch_tag="$3"

  mkdir -p "$fw/Versions/A/Resources"

  cat > "$fw/Versions/A/Resources/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>libtunes4r</string>
    <key>CFBundleIdentifier</key>
    <string>com.tunes4r.libtunes4r.${arch_tag}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>libtunes4r</string>
    <key>CFBundlePackageType</key>
    <string>FMWK</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
</dict>
</plist>
PLIST

  cp "$dylib" "$fw/Versions/A/libtunes4r"
  install_name_tool -id "@rpath/libtunes4r.framework/libtunes4r" \
    "$fw/Versions/A/libtunes4r"

  ln -snf "A" "$fw/Versions/Current"
  ln -snf "Versions/Current/libtunes4r" "$fw/libtunes4r"
  ln -snf "Versions/Current/Resources"  "$fw/Resources"
}

build_android() {
  echo "=== Building for Android ==="

  # Ensure NDK is configured
  if [ -z "${ANDROID_NDK_HOME:-}" ]; then
    if [ -d "$HOME/Library/Android/sdk/ndk" ]; then
      ANDROID_NDK_HOME=$(ls -d "$HOME/Library/Android/sdk/ndk"/*/ | sort -V | tail -1)
      ANDROID_NDK_HOME="${ANDROID_NDK_HOME%/}"
    else
      echo "ERROR: ANDROID_NDK_HOME not set. Set it or install Android NDK."
      exit 1
    fi
  fi

  export ANDROID_NDK_HOME

  NDK_TOOLCHAIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64/bin"
  export PATH="$NDK_TOOLCHAIN:$PATH"

  # NDK 28+ dropped unversioned clang symlinks that cc-rs/cmake expect.
  # Create them on demand: e.g. aarch64-linux-android-clang -> aarch64-linux-android21-clang
  for f in "$NDK_TOOLCHAIN"/*-linux-android*-clang "$NDK_TOOLCHAIN"/*-linux-android*-clang++; do
    [ -f "$f" ] || continue
    base="${f%-clang*}"           # strip -clang or -clang++
    base="${base%[0-9][0-9]}"    # strip trailing version suffix (e.g. 21)
    if [ "$base" != "${f%-clang*}" ] && [ ! -f "$base-clang" ] && [ ! -f "$base-clang++" ]; then
      ln -sf "$(basename "$f")" "$base-${f##*-}" 2>/dev/null || true
    fi
  done

  # Set CC/CXX for cross-compilation
  export CC_aarch64_linux_android="aarch64-linux-android21-clang"
  export CC_armv7_linux_androideabi="armv7a-linux-androideabi21-clang"
  export CC_x86_64_linux_android="x86_64-linux-android21-clang"
  export CC_i686_linux_android="i686-linux-android21-clang"

  cd "$RUST_DIR"
  for target in aarch64-linux-android armv7-linux-androideabi \
                x86_64-linux-android i686-linux-android; do
    echo "  Building for $target..."
    cargo build --target "$target" --release || echo "  [WARN] $target failed"
  done
  cd "$PLUGIN_DIR"

  declare -A ABI_MAP
  ABI_MAP[aarch64-linux-android]="arm64-v8a"
  ABI_MAP[armv7-linux-androideabi]="armeabi-v7a"
  ABI_MAP[x86_64-linux-android]="x86_64"
  ABI_MAP[i686-linux-android]="x86"

  for target in "${!ABI_MAP[@]}"; do
    abi="${ABI_MAP[$target]}"
    mkdir -p "android/src/main/jniLibs/$abi"
    src="$RUST_DIR/target/$target/release/libtunes4r.so"
    if [ -f "$src" ]; then
      cp "$src" "android/src/main/jniLibs/$abi/"
      echo "[Android] Copied to android/src/main/jniLibs/$abi/libtunes4r.so"
    fi
  done

  # Also bundle libc++_shared.so from NDK
  NDK_SYSROOT="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64/sysroot/usr/lib"
  for target in aarch64-linux-android arm-linux-androideabi \
                x86_64-linux-android i686-linux-android; do
    local_dir=""
    case "$target" in
      aarch64-linux-android)  local_dir="arm64-v8a" ;;
      arm-linux-androideabi)  local_dir="armeabi-v7a" ;;
      x86_64-linux-android)  local_dir="x86_64" ;;
      i686-linux-android)    local_dir="x86" ;;
    esac
    cxx="$NDK_SYSROOT/$target/libc++_shared.so"
    if [ -f "$cxx" ]; then
      cp "$cxx" "android/src/main/jniLibs/$local_dir/" 2>/dev/null || true
    fi
  done
  echo "[Android] Done."
}

case "$PLATFORM" in
  install)
    install_targets
    ;;
  ios)
    build_ios
    ;;
  macos)
    build_macos
    ;;
  android)
    build_android
    ;;
  all)
    build_macos
    build_ios
    build_android
    echo ""
    echo "=== All platform builds complete ==="
    ;;
  *)
    echo "Usage: $0 [ios|android|macos|all|install] [debug|release]"
    exit 1
    ;;
esac
