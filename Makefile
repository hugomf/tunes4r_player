# tunes4r plugin Makefile
#
# Targets:
#   install         - Install Rust cross-compilation targets
#   build-macos     - Build macOS dylib
#   build-ios       - Build iOS static lib
#   build-android   - Build Android .so libs (requires NDK)
#   build-all       - Build for all platforms
#   prepare         - Build all + copy artifacts into plugin dirs
#
# Use BUILD=debug for debug builds (default: release).

BUILD ?= release

.PHONY: install build-macos build-ios build-android build-all prepare

install:
	./scripts/build_rust.sh install

build-macos:
	./scripts/build_rust.sh macos $(BUILD)

build-ios:
	./scripts/build_rust.sh ios $(BUILD)

build-android:
	./scripts/build_rust.sh android $(BUILD)

build-all:
	./scripts/build_rust.sh all $(BUILD)

# Full prepare: compile Rust and copy artifacts into plugin directories
prepare: build-all
	@echo "✅ All artifacts prepared for publishing"
	@echo "   ios/Frameworks/libtunes4r.a"
	@echo "   macos/Frameworks/libtunes4r.dylib"
	@echo "   android/src/main/jniLibs/{arm64-v8a,armeabi-v7a,x86_64,x86}/libtunes4r.so"
