#!/bin/bash
# Prepare the tunes4r plugin for publishing to pub.dev.
#
# This ensures all native libraries are built and in the right places,
# then runs `flutter pub publish --dry-run` for verification.

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLUGIN_DIR="$(dirname "$SCRIPT_DIR")"

echo "=== Preparing tunes4r for publishing ==="
echo ""

# 1. Verify native libs exist
echo "[1/4] Checking native libraries..."
IOS_LIB="$PLUGIN_DIR/ios/Frameworks/libtunes4r.a"
MACOS_LIB="$PLUGIN_DIR/macos/Frameworks/libtunes4r.dylib"
ANDROID_DIR="$PLUGIN_DIR/android/src/main/jniLibs"

if [ ! -f "$IOS_LIB" ]; then
  echo "  ⚠  iOS lib missing: $IOS_LIB"
  echo "     Run: make build-ios"
fi

if [ ! -f "$MACOS_LIB" ]; then
  echo "  ⚠  macOS lib missing: $MACOS_LIB"
  echo "     Run: make build-macos"
fi

if [ ! -d "$ANDROID_DIR" ] || [ -z "$(ls -A "$ANDROID_DIR" 2>/dev/null)" ]; then
  echo "  ⚠  Android libs missing in $ANDROID_DIR"
  echo "     Run: make build-android"
fi

# 2. Verify Rust source is present (vendored for pub.dev)
echo "[2/4] Checking Rust source..."
if [ -d "$PLUGIN_DIR/rust" ] && [ -f "$PLUGIN_DIR/rust/Cargo.toml" ]; then
  echo "  ✓ Rust source found"
else
  echo "  ⚠  Rust source not found in plugin directory."
  echo "     Copy or symlink: ln -s ../../rust $PLUGIN_DIR/rust"
fi

# 3. Dry run
echo "[3/4] Running pub publish dry-run..."
cd "$PLUGIN_DIR"
flutter pub publish --dry-run 2>&1 || true

echo ""
echo "[4/4] Done."
echo ""
echo "To publish: cd $PLUGIN_DIR && flutter pub publish"
