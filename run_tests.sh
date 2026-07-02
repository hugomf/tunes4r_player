#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
RUST_DIR="$ROOT/rust"

echo "============================================"
echo " tune4r_player — Run All Tests"
echo "============================================"

# ── 1. Rust unit tests (core + youtube crates) ──
echo ""
echo "═══ Rust unit tests (crates) ═══"
(cd "$RUST_DIR" && cargo test -p tunes4r-core --lib 2>&1)

echo ""
echo "═══ Rust integration tests ═══"
(cd "$RUST_DIR" && cargo test --test ffi_contract 2>&1)
(cd "$RUST_DIR" && cargo test --test seek_streaming 2>&1)
(cd "$RUST_DIR" && cargo test --test mock_youtube_stream 2>&1)
(cd "$RUST_DIR" && cargo test --test yt_stream_seek 2>&1)
(cd "$RUST_DIR" && cargo test --test streaming_download 2>&1)
(cd "$RUST_DIR" && cargo test --test decode_queue_backpressure 2>&1)

# ── 2. YouTube-dependent tests (require YT_TEST=1) ──
echo ""
echo "═══ Rust YouTube integration tests (YT_TEST=1) ═══"
(cd "$RUST_DIR" && YT_TEST=1 cargo test --test test_real_youtube 2>&1)
(cd "$RUST_DIR" && YT_TEST=1 cargo test --test real_youtube_stream 2>&1)

# ── 3. Flutter example tests ──
echo ""
echo "═══ Flutter widget tests ═══"
(cd "$ROOT/example" && flutter test test/widget_test.dart 2>&1)

echo ""
echo "═══ Flutter FFI seek test ═══"
(cd "$ROOT/example" && flutter test test/ffi_seek_test.dart 2>&1)

echo ""
echo "═══ Flutter YouTube seek test (YT_TEST=true) ═══"
(cd "$ROOT/example" && flutter test --dart-define=YT_TEST=true test/yt_stream_seek_test.dart 2>&1)

echo ""
echo "============================================"
echo " All tests complete."
echo "============================================"
