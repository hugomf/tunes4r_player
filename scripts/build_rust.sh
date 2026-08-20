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
RUST_DIR="$(dirname "$PLUGIN_DIR")/tunes4r-core/crates/ffi"

if [ ! -d "$RUST_DIR" ]; then
  echo "ERROR: $RUST_DIR not found."
  echo "Expected tunes4r-core at: $(dirname "$PLUGIN_DIR")/tunes4r-core"
  exit 1
fi

BUILD_TYPE="${2:-release}"
PLATFORM="${1:-all}"
FEATURES="${FEATURES:-}"

echo "=== tunes4r Rust Build ==="
echo "  Plugin dir: $PLUGIN_DIR"
echo "  Rust dir:   $RUST_DIR"
echo "  Platform:   $PLATFORM"
echo "  Build:      $BUILD_TYPE"
echo "  Features:   $FEATURES"
echo ""

features_flag() {
  if [ -n "$FEATURES" ]; then echo "--features $FEATURES"; else echo ""; fi
}

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
  local profile_flag=""
  local feat_flag
  feat_flag=$(features_flag)
  [ "$profile" = "release" ] && profile_flag="--release"

  cargo rustc --lib --target aarch64-apple-ios $profile_flag $feat_flag --crate-type staticlib
  cargo rustc --lib --target aarch64-apple-ios-sim $profile_flag $feat_flag --crate-type staticlib
  cargo rustc --lib --target x86_64-apple-ios $profile_flag $feat_flag --crate-type staticlib

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

  # Copy to the SPM package's Frameworks directory (consumed by SPM plugin
  # integration — Flutter resolves relative paths from the package symlink).
  local spm_dir="ios/tunes4r_player/Frameworks"
  mkdir -p "$spm_dir"
  tar -C "ios/Frameworks" -cf - libtunes4r.xcframework \
    | tar -C "$spm_dir" -xf -

  # Keep the raw .a as a convenience fallback (device only)
  cp "$device" ios/Frameworks/libtunes4r.a

  echo "[iOS] XCFramework created at ios/Frameworks/libtunes4r.xcframework"
  echo "[iOS] SPM copy at $spm_dir/libtunes4r.xcframework"
  echo "[iOS] Device-only .a at ios/Frameworks/libtunes4r.a"
}

build_macos() {
  echo "=== Building for macOS ==="

  local profile="release"
  [ "$BUILD_TYPE" = "debug" ] && profile="debug"
  local profile_flag=""
  [ "$profile" = "release" ] && profile_flag="--release"

  # Ensure both Apple targets are available so the XCFramework is universal
  # (arm64 for Apple Silicon, x86_64 for Intel).
  rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null 2>&1 || true

  local feat_flag
  feat_flag=$(features_flag)

  cd "$RUST_DIR"
  cargo build --target aarch64-apple-darwin $profile_flag $feat_flag
  cargo build --target x86_64-apple-darwin  $profile_flag $feat_flag
  cd "$PLUGIN_DIR"

  # crates/ffi is a workspace member, so cargo outputs to the workspace
  # target dir ($RUST_DIR/../../target), not $RUST_DIR/target.
  local ws_target="$RUST_DIR/../../target"
  local arm_lib="$ws_target/aarch64-apple-darwin/$profile/libtunes4r.dylib"
  local x86_lib="$ws_target/x86_64-apple-darwin/$profile/libtunes4r.dylib"
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

  # xcodebuild -create-xcframework rejects two dylibs that share the same
  # install_name (which they do, since cargo writes both with the path under
  # target/<arch>/release/deps/). Make a universal (fat) dylib with lipo
  # first, then wrap it in a single .framework and pass that to xcodebuild.
  #
  # IMPORTANT: Output the xcframework to a temp dir *outside* the project
  # directory and then copy it in. Writing directly into macos/Frameworks/
  # triggers a deletion race (likely Flutter's Xcode project watcher).
  local tmpdir=""
  tmpdir="$(mktemp -d)"
  # shellcheck disable=SC2064
  trap "rm -rf '$tmpdir'" EXIT

  local fat_dylib="$tmpdir/libtunes4r.dylib"
  lipo -create "$arm_lib" "$x86_lib" -output "$fat_dylib"
  install_name_tool -id "@rpath/libtunes4r.framework/libtunes4r" "$fat_dylib"

  local fw="$tmpdir/libtunes4r.framework"
  _make_framework "$fat_dylib" "$fw"

  local xc_out="$tmpdir/libtunes4r.xcframework"
  if ! xcodebuild -create-xcframework \
        -framework "$fw" \
        -output "$xc_out" >/dev/null 2>&1; then
    echo "ERROR: xcodebuild -create-xcframework failed; re-running with output"
    xcodebuild -create-xcframework \
        -framework "$fw" \
        -output "$xc_out"
    exit 1
  fi

  # Copy from temp to both destinations in one shot. Use a tar pipe to
  # bypass APFS clone behavior (which can get corrupted when rm -rf of
  # the clone source triggers a race with install_name_tool).
  local spm_dir="$out_dir/../tunes4r_player/Frameworks"
  mkdir -p "$spm_dir"
  tar -C "$tmpdir" -cf - libtunes4r.xcframework \
    | tar -C "$out_dir" -xf -
  tar -C "$tmpdir" -cf - libtunes4r.xcframework \
    | tar -C "$spm_dir" -xf -

  trap - EXIT
  rm -rf "$tmpdir"

  # Verify both copies exist
  if [ ! -d "$out_dir/libtunes4r.xcframework" ]; then
    echo "ERROR: xcframework not found at $out_dir/libtunes4r.xcframework"
    exit 1
  fi
  if [ ! -d "$spm_dir/libtunes4r.xcframework" ]; then
    echo "ERROR: xcframework not found at $spm_dir/libtunes4r.xcframework"
    exit 1
  fi

  # 2) Also drop a flat dylib for any tooling that loads it without an
  #    .framework wrapper (raw `DynamicLibrary.open('libtunes4r.dylib')`).
  cp "$arm_lib" "$out_dir/libtunes4r.dylib"
  install_name_tool -id "@rpath/libtunes4r.dylib" "$out_dir/libtunes4r.dylib"
  cp "$arm_lib" "$spm_dir/libtunes4r.dylib"
  install_name_tool -id "@rpath/libtunes4r.dylib" "$spm_dir/libtunes4r.dylib"

  echo "[macOS] XCFramework at $out_dir/libtunes4r.xcframework (arm64 + x86_64)"
  echo "[macOS] SPM copy at $spm_dir/libtunes4r.xcframework"
  echo "[macOS] Flat dylib at $out_dir/libtunes4r.dylib"
}

# Build a minimal macOS .framework bundle from a dylib. xcodebuild
# -create-xcframework requires Resources/Info.plist + the standard
# Versions/{A,Current} symlink layout.
#
# $1 = source dylib
# $2 = target framework directory
_make_framework() {
  local dylib="$1"
  local fw="$2"

  mkdir -p "$fw/Versions/A/Resources"

  cat > "$fw/Versions/A/Resources/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>libtunes4r</string>
    <key>CFBundleIdentifier</key>
    <string>com.tunes4r.libtunes4r</string>
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

  local profile="release"
  [ "$BUILD_TYPE" = "debug" ] && profile="debug"
  local profile_flag=""
  [ "$profile" = "release" ] && profile_flag="--release"

  local feat_flag
  feat_flag=$(features_flag)

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

  cd "$RUST_DIR"
  local abi_list="${ABI:-arm64-v8a}"
  local ndk_targets=""
  for t in $abi_list; do ndk_targets="$ndk_targets -t $t"; done
  cargo ndk \
    $ndk_targets \
    -o "$PLUGIN_DIR/android/src/main/jniLibs" \
    build --lib \
    $profile_flag $feat_flag
  cd "$PLUGIN_DIR"

  # Copy libc++_shared.so from the NDK into each ABI directory.
  local ndk_cxx="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64/sysroot/usr/lib"
  local jni="$PLUGIN_DIR/android/src/main/jniLibs"
  for target in $abi_list; do
    case "$target" in
      arm64-v8a)       abi_dir="arm64-v8a";   ndk_abi="aarch64-linux-android" ;;
      armeabi-v7a)     abi_dir="armeabi-v7a"; ndk_abi="arm-linux-androideabi" ;;
      x86_64)          abi_dir="x86_64";      ndk_abi="x86_64-linux-android" ;;
      x86)             abi_dir="x86";         ndk_abi="i686-linux-android" ;;
      *)               echo "Unknown ABI target: $target"; exit 1 ;;
    esac
    cp "$ndk_cxx/$ndk_abi/libc++_shared.so" "$jni/$abi_dir/libc++_shared.so"
  done
  echo "[Android] libc++_shared.so copied."

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
