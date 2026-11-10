#!/usr/bin/env bash
# Builds the Rust core for iOS: ios/build/OmgCoreFFI.xcframework (device and simulator, arm64)
# and ios/build/generated/OmgCore.swift. Run it before xcodegen / xcodebuild. Needs cargo,
# `rustup target add aarch64-apple-ios aarch64-apple-ios-sim` and Xcode.
set -euo pipefail

IOS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="$IOS_DIR/../rust"
OUT="$IOS_DIR/build"
GEN="$OUT/generated"
WORK="$OUT/work"
TARGETS=(aarch64-apple-ios aarch64-apple-ios-sim)

cd "$RUST_DIR"
TARGET_DIR="${CARGO_TARGET_DIR:-$RUST_DIR/target}"

rm -rf "$OUT/OmgCoreFFI.xcframework" "$GEN" "$WORK"
mkdir -p "$GEN" "$WORK/headers"

# The Swift bindings are generated from a host build of the same crate.
cargo build -p omg-ffi --release
cargo run -p omg-ffi --release --features cli --bin uniffi-bindgen -- \
    generate --library "$TARGET_DIR/release/libomg_ffi.dylib" --language swift --out-dir "$WORK/bindings"

cp "$WORK/bindings/OmgCore.swift" "$GEN/OmgCore.swift"
cp "$WORK/bindings/OmgCoreFFI.h" "$WORK/headers/"
cp "$WORK/bindings/OmgCoreFFI.modulemap" "$WORK/headers/module.modulemap"

args=()
for target in "${TARGETS[@]}"; do
    cargo build -p omg-ffi --release --lib --target "$target"
    args+=(-library "$TARGET_DIR/$target/release/libomg_ffi.a" -headers "$WORK/headers")
done

xcodebuild -create-xcframework "${args[@]}" -output "$OUT/OmgCoreFFI.xcframework"
rm -rf "$WORK"
echo "Built $OUT/OmgCoreFFI.xcframework and $GEN/OmgCore.swift"
