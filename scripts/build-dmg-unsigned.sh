#!/usr/bin/env bash
# Builds OpCode in Release configuration and packages a distributable
# .dmg WITHOUT a Developer ID certificate or notarization - for shipping
# before (or without ever) enrolling in the Apple Developer Program.
#
# The app is still code-signed (ad-hoc/"Sign to Run Locally", the same
# default Xcode has used for every Debug build all along) - that's just
# local integrity signing, not a trust identity Gatekeeper recognizes
# from another machine, so it does not avoid the Gatekeeper warning a
# downloaded, unnotarized app shows on first launch. See README.md's
# "Installing on macOS" section for the one-time bypass steps end users
# need - this script does not (and, without Developer ID, cannot) avoid
# that warning; it only gets a real, installable app into their hands.
#
# Once Developer ID enrollment happens, scripts/release-dmg.sh replaces
# this one - same shape, but signed, notarized, and stapled, which is
# what actually removes the warning.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAC_DIR="$ROOT_DIR/x8086-mac"
BUILD_DIR="$ROOT_DIR/build/release-unsigned"
DERIVED_DATA="$BUILD_DIR/DerivedData"
STAGING_DIR="$BUILD_DIR/dmg-staging"

rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"

echo "build-dmg-unsigned.sh: building universal Rust core..."
"$ROOT_DIR/scripts/build-universal.sh"

echo "build-dmg-unsigned.sh: generating Xcode project..."
(cd "$MAC_DIR" && xcodegen generate)

echo "build-dmg-unsigned.sh: building Release (ad-hoc signed, no team)..."
xcodebuild build \
  -project "$MAC_DIR/x8086.xcodeproj" \
  -scheme x8086 \
  -configuration Release \
  -destination 'platform=macOS' \
  -derivedDataPath "$DERIVED_DATA"

APP_PATH="$DERIVED_DATA/Build/Products/Release/OpCode.app"
if [[ ! -d "$APP_PATH" ]]; then
  echo "build-dmg-unsigned.sh: expected build output at $APP_PATH but it's missing." >&2
  exit 1
fi

echo "build-dmg-unsigned.sh: verifying (ad-hoc) code signature..."
codesign --verify --verbose=2 "$APP_PATH"

VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP_PATH/Contents/Info.plist")"
DMG_NAME="OpCode-$VERSION-unsigned.dmg"
DMG_PATH="$BUILD_DIR/$DMG_NAME"

echo "build-dmg-unsigned.sh: building $DMG_NAME..."
rm -rf "$STAGING_DIR"
mkdir -p "$STAGING_DIR"
cp -R "$APP_PATH" "$STAGING_DIR/"
ln -s /Applications "$STAGING_DIR/Applications"

hdiutil create -volname "OpCode" -srcfolder "$STAGING_DIR" -ov -format UDZO "$DMG_PATH"

echo "build-dmg-unsigned.sh: done."
echo "  Unsigned build (no Developer ID, not notarized): $DMG_PATH"
echo "  Point installers at README.md's 'Installing on macOS' section - the"
echo "  first launch needs a one-time Gatekeeper bypass without a Developer ID."
