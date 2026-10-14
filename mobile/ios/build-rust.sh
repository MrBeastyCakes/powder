#!/bin/sh
# Called by Xcode's "Build Rust" phase. Builds the app's static library for
# the device or simulator Xcode is targeting.
set -e
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:$PATH"
TARGET="${RUST_TARGET:-aarch64-apple-ios}"
cargo build --release -p pt-app --lib --target "$TARGET"
