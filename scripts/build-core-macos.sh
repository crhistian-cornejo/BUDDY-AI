#!/bin/bash
# Builds buddy-core as a static library for the Mac app and generates its Swift bindings (UniFFI).
# Runs as an Xcode pre-build phase and by hand. Output: apps/macos/Generated/ (git-ignored).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/apps/macos/Generated"
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

if ! command -v cargo >/dev/null; then
  echo "error: cargo no está instalado (https://rustup.rs)" >&2
  exit 1
fi

# Xcode passes CONFIGURATION and ARCHS; by hand it is a debug build for this Mac.
PROFILE="debug"
CARGO_PROFILE=()
if [[ "${CONFIGURATION:-Debug}" == "Release" ]]; then
  PROFILE="release"
  CARGO_PROFILE=(--release)
fi
ARCHS="${ARCHS:-$(uname -m)}"

cd "$ROOT"
LIBS=()
for arch in $ARCHS; do
  case "$arch" in
    arm64) target="aarch64-apple-darwin" ;;
    x86_64) target="x86_64-apple-darwin" ;;
    *) echo "error: arquitectura no soportada: $arch" >&2; exit 1 ;;
  esac
  MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-15.0}" \
    cargo rustc -p buddy-core --lib --features ffi --crate-type staticlib --target "$target" ${CARGO_PROFILE[@]+"${CARGO_PROFILE[@]}"}
  LIBS+=("$ROOT/target/$target/$PROFILE/libbuddy_core.a")
done

mkdir -p "$OUT"
if [[ ${#LIBS[@]} -gt 1 ]]; then
  lipo -create "${LIBS[@]}" -output "$OUT/libbuddy_core.a"
else
  cp "${LIBS[0]}" "$OUT/libbuddy_core.a"
fi

cargo run -q -p uniffi-bindgen -- generate --library "${LIBS[0]}" --language swift --out-dir "$OUT/swift"
mkdir -p "$OUT/include"
cp "$OUT/swift/buddy_coreFFI.h" "$OUT/include/"
cp "$OUT/swift/buddy_coreFFI.modulemap" "$OUT/include/module.modulemap"
cp "$OUT/swift/buddy_core.swift" "$OUT/BuddyCore.swift"
