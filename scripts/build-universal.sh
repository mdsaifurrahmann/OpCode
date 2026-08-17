#!/usr/bin/env bash
# Cross-compiles x8086-ffi for Apple Silicon + Intel, combines the two
# static libraries into one universal binary via lipo, generates Swift
# bindings from it with our own uniffi-bindgen binary, and packages the
# result into an XCFramework the Xcode project links against.
#
# Single source of truth for this pipeline: run identically by developers
# and CI, invoked from Xcode as a Run Script build phase. Idempotent -
# skips work when the XCFramework is newer than every Rust source file
# that produced it, so it's cheap to run on every Xcode build.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
PROFILE=release
LIB_NAME=libx8086_ffi.a
BUILD_DIR="$ROOT_DIR/build"
BINDINGS_DIR="$BUILD_DIR/swift-bindings"
HEADERS_DIR="$BUILD_DIR/headers"
XCFRAMEWORK="$BUILD_DIR/x8086FFI.xcframework"
UNIVERSAL_LIB="$BUILD_DIR/$LIB_NAME"
GENERATED_SWIFT="$ROOT_DIR/x8086-mac/Generated/x8086_ffi.swift"

mkdir -p "$BUILD_DIR"

# --- idempotency check ------------------------------------------------------
# Skip the whole pipeline only if every output it produces already exists
# and nothing it was built from has changed since. (All outputs, not just
# the XCFramework - a partial previous run, e.g. from before Xcode.app was
# installed, must not look "done".)
if [[ -d "$XCFRAMEWORK" ]] && [[ -f "$GENERATED_SWIFT" ]]; then
  newer_source=$(find "$ROOT_DIR/Cargo.toml" "$ROOT_DIR/Cargo.lock" "$ROOT_DIR/crates" \
    -type f \( -name '*.rs' -o -name 'Cargo.toml' -o -name 'Cargo.lock' \) \
    -newer "$XCFRAMEWORK" -print -quit 2>/dev/null || true)
  if [[ -z "$newer_source" ]]; then
    echo "build-universal.sh: outputs are up to date, skipping rebuild."
    exit 0
  fi
fi

echo "build-universal.sh: (re)building universal x8086-ffi..."

# --- 1. cross-compile for each target ---------------------------------------
for target in "${TARGETS[@]}"; do
  echo "-- building x8086-ffi for $target ($PROFILE)"
  rustup target add "$target" >/dev/null
  cargo build --profile "$PROFILE" --target "$target" -p x8086-ffi
done

# --- 2. lipo the two static libs into one universal static lib -------------
rm -f "$UNIVERSAL_LIB"
lipo -create -output "$UNIVERSAL_LIB" \
  "$ROOT_DIR/target/${TARGETS[0]}/$PROFILE/$LIB_NAME" \
  "$ROOT_DIR/target/${TARGETS[1]}/$PROFILE/$LIB_NAME"
echo "-- universal static lib: $UNIVERSAL_LIB"
lipo -info "$UNIVERSAL_LIB"

# --- 3. generate Swift bindings ---------------------------------------------
# uniffi's metadata is architecture-independent, so any single-arch build
# works as the bindgen source; using the host build avoids needing the
# lipo'd binary just to introspect it.
rm -rf "$BINDINGS_DIR"
mkdir -p "$BINDINGS_DIR"
HOST_TARGET="$(rustc -vV | sed -n 's/host: //p')"
cargo run --profile "$PROFILE" -p x8086-ffi --bin uniffi-bindgen -- generate \
  --library "$ROOT_DIR/target/$HOST_TARGET/$PROFILE/$LIB_NAME" \
  --language swift \
  --out-dir "$BINDINGS_DIR"

rm -rf "$HEADERS_DIR"
mkdir -p "$HEADERS_DIR"
cp "$BINDINGS_DIR"/*.h "$HEADERS_DIR/"
cp "$BINDINGS_DIR"/*.modulemap "$HEADERS_DIR/module.modulemap"

# --- 4. package the XCFramework ---------------------------------------------
# Requires full Xcode.app (not just Command Line Tools). Degrade gracefully
# if it isn't installed: the static lib + Swift bindings are still usable
# on their own, and this step re-runs automatically next time the script
# runs once Xcode is available.
if ! xcodebuild -version >/dev/null 2>&1; then
  echo "build-universal.sh: WARNING - full Xcode.app is required for 'xcodebuild -create-xcframework' but is not installed (only Command Line Tools were found)."
  echo "  Universal static lib and Swift bindings were still produced:"
  echo "    $UNIVERSAL_LIB"
  echo "    $BINDINGS_DIR/x8086_ffi.swift"
  echo "  Install Xcode from the App Store, then re-run this script to produce $XCFRAMEWORK."
  exit 0
fi

rm -rf "$XCFRAMEWORK"
xcodebuild -create-xcframework \
  -library "$UNIVERSAL_LIB" -headers "$HEADERS_DIR" \
  -output "$XCFRAMEWORK"

# --- 5. hand the generated Swift source to the Xcode project ----------------
# x8086-mac/Generated/ is gitignored - the Xcode project references it at a
# stable path, and this step keeps it in sync with what was just generated.
GENERATED_DIR="$ROOT_DIR/x8086-mac/Generated"
mkdir -p "$GENERATED_DIR"
cp "$BINDINGS_DIR/x8086_ffi.swift" "$GENERATED_DIR/x8086_ffi.swift"

echo "build-universal.sh: done."
echo "  XCFramework:  $XCFRAMEWORK"
echo "  Swift source: $GENERATED_DIR/x8086_ffi.swift"
