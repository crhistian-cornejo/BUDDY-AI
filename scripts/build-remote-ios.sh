#!/bin/bash
# Builds buddy-remote (the phone's end of the link) as a static library for the iPhone app and generates its Swift
# bindings (UniFFI). Runs as an Xcode pre-build phase and by hand. Output: apps/ios/Generated/ (git-ignored).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/apps/ios/Generated"
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

if ! command -v cargo >/dev/null; then
  echo "error: cargo no está instalado (https://rustup.rs)" >&2
  exit 1
fi

# Xcode passes CONFIGURATION and PLATFORM_NAME; by hand it is a debug build for the simulator.
PROFILE="debug"
CARGO_PROFILE=()
if [[ "${CONFIGURATION:-Debug}" == "Release" ]]; then
  PROFILE="release"
  CARGO_PROFILE=(--release)
fi
case "${PLATFORM_NAME:-iphonesimulator}" in
  iphoneos) TARGET="aarch64-apple-ios" ;;
  *) TARGET="aarch64-apple-ios-sim" ;;
esac
if ! rustup target list --installed | grep -qx "$TARGET"; then
  echo "error: falta el destino de Rust $TARGET (rustup target add aarch64-apple-ios aarch64-apple-ios-sim)" >&2
  exit 1
fi

# Xcode's SDK is the phone's: the build's own tools (proc macros, the bindings generator) are for this Mac.
unset SDKROOT LIBRARY_PATH

cd "$ROOT"
IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-26.0}" \
  cargo rustc -p buddy-remote --lib --features ffi --crate-type staticlib --target "$TARGET" ${CARGO_PROFILE[@]+"${CARGO_PROFILE[@]}"}
mkdir -p "$OUT/include"
cp "$ROOT/target/$TARGET/$PROFILE/libbuddy_remote.a" "$OUT/libbuddy_remote.a"

# The bindings are read from a build for this Mac (the same interface, whatever the target).
cargo rustc -q -p buddy-remote --lib --features ffi --crate-type staticlib
cargo run -q -p uniffi-bindgen -- generate --library "$ROOT/target/debug/libbuddy_remote.a" --language swift --out-dir "$OUT/swift"
cp "$OUT/swift/buddy_remoteFFI.h" "$OUT/include/"
cp "$OUT/swift/buddy_remoteFFI.modulemap" "$OUT/include/module.modulemap"
cp "$OUT/swift/buddy_remote.swift" "$OUT/BuddyRemote.swift"
