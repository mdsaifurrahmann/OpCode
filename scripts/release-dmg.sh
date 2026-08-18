#!/usr/bin/env bash
# Builds OpCode in Release configuration, signs it with a Developer ID
# Application identity, packages a distributable .dmg, submits it for
# notarization, and staples the resulting ticket - the same pipeline a
# developer and CI both run, so there's exactly one place this logic lives.
#
# Requires, none of which this script can obtain on its own:
#   - A "Developer ID Application" certificate + private key in the login
#     keychain (Xcode > Settings > Accounts > Manage Certificates, or the
#     Apple Developer portal - either way needs an enrolled Apple Developer
#     Program membership and an interactive Apple ID sign-in).
#   - DEVELOPER_TEAM_ID: your Apple Developer Team ID (Xcode > Settings >
#     Accounts, or developer.apple.com/account > Membership).
#   - A notarytool credential profile stored once, interactively, via:
#       xcrun notarytool store-credentials "opcode-notary" \
#         --apple-id you@example.com --team-id TEAMID --password APP_SPECIFIC_PW
#     (an app-specific password from appleid.apple.com, not your Apple ID
#     password itself). This script only ever references the profile by
#     name - it never sees or handles the credential.
#
# Usage:
#   DEVELOPER_TEAM_ID=ABCDE12345 ./scripts/release-dmg.sh
# Optional:
#   NOTARY_PROFILE=opcode-notary   (default shown)
#   SKIP_NOTARIZE=1                (build + sign + package, skip submission -
#                                    useful for a dry run once a cert exists
#                                    but you're not ready to notarize yet)
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAC_DIR="$ROOT_DIR/x8086-mac"
BUILD_DIR="$ROOT_DIR/build/release"
ARCHIVE_PATH="$BUILD_DIR/OpCode.xcarchive"
EXPORT_DIR="$BUILD_DIR/export"
STAGING_DIR="$BUILD_DIR/dmg-staging"
NOTARY_PROFILE="${NOTARY_PROFILE:-opcode-notary}"

if [[ -z "${DEVELOPER_TEAM_ID:-}" ]]; then
  echo "release-dmg.sh: DEVELOPER_TEAM_ID is not set." >&2
  echo "  Find yours at developer.apple.com/account > Membership, or Xcode > Settings > Accounts." >&2
  echo "  Usage: DEVELOPER_TEAM_ID=ABCDE12345 ./scripts/release-dmg.sh" >&2
  exit 1
fi

if ! security find-identity -v -p codesigning | grep -q "Developer ID Application"; then
  echo "release-dmg.sh: no 'Developer ID Application' signing identity found in the keychain." >&2
  echo "  This requires an enrolled Apple Developer Program membership (developer.apple.com)." >&2
  echo "  Once enrolled: Xcode > Settings > Accounts > Manage Certificates > + > Developer ID Application." >&2
  exit 1
fi

rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"

# --- 1. Rust core + Xcode project -------------------------------------------
echo "release-dmg.sh: building universal Rust core..."
"$ROOT_DIR/scripts/build-universal.sh"

echo "release-dmg.sh: generating Xcode project..."
(cd "$MAC_DIR" && xcodegen generate)

# --- 2. archive ---------------------------------------------------------------
echo "release-dmg.sh: archiving (Release, Developer ID: team $DEVELOPER_TEAM_ID)..."
xcodebuild archive \
  -project "$MAC_DIR/x8086.xcodeproj" \
  -scheme x8086 \
  -configuration Release \
  -archivePath "$ARCHIVE_PATH" \
  -destination 'generic/platform=macOS' \
  DEVELOPMENT_TEAM="$DEVELOPER_TEAM_ID" \
  CODE_SIGN_IDENTITY="Developer ID Application"

# --- 3. export, signed for Developer ID distribution ------------------------
EXPORT_OPTIONS="$BUILD_DIR/exportOptions.plist"
cat > "$EXPORT_OPTIONS" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>method</key>
	<string>developer-id</string>
	<key>teamID</key>
	<string>$DEVELOPER_TEAM_ID</string>
	<key>signingStyle</key>
	<string>automatic</string>
</dict>
</plist>
PLIST

echo "release-dmg.sh: exporting signed .app..."
xcodebuild -exportArchive \
  -archivePath "$ARCHIVE_PATH" \
  -exportPath "$EXPORT_DIR" \
  -exportOptionsPlist "$EXPORT_OPTIONS"

APP_PATH="$EXPORT_DIR/OpCode.app"
if [[ ! -d "$APP_PATH" ]]; then
  echo "release-dmg.sh: expected export at $APP_PATH but it's missing." >&2
  exit 1
fi

echo "release-dmg.sh: verifying code signature..."
codesign --verify --deep --strict --verbose=2 "$APP_PATH"
spctl -a -t exec -vvv "$APP_PATH"

VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP_PATH/Contents/Info.plist")"
DMG_NAME="OpCode-$VERSION.dmg"
DMG_PATH="$BUILD_DIR/$DMG_NAME"

# --- 4. package the .dmg -----------------------------------------------------
echo "release-dmg.sh: building $DMG_NAME..."
rm -rf "$STAGING_DIR"
mkdir -p "$STAGING_DIR"
cp -R "$APP_PATH" "$STAGING_DIR/"
ln -s /Applications "$STAGING_DIR/Applications"

hdiutil create -volname "OpCode" -srcfolder "$STAGING_DIR" -ov -format UDZO "$DMG_PATH"

echo "release-dmg.sh: signing the .dmg..."
codesign --sign "Developer ID Application" --identifier "com.saifurrahmann.opcode.dmg" "$DMG_PATH"

# --- 5. notarize + staple -----------------------------------------------------
if [[ "${SKIP_NOTARIZE:-0}" == "1" ]]; then
  echo "release-dmg.sh: SKIP_NOTARIZE=1, stopping before notarization."
  echo "  Unnotarized build: $DMG_PATH"
  exit 0
fi

echo "release-dmg.sh: submitting for notarization (this can take a few minutes)..."
xcrun notarytool submit "$DMG_PATH" --keychain-profile "$NOTARY_PROFILE" --wait

echo "release-dmg.sh: stapling notarization ticket..."
xcrun stapler staple "$DMG_PATH"

echo "release-dmg.sh: validating..."
xcrun stapler validate "$DMG_PATH"
spctl -a -t open --context context:primary-signature -v "$DMG_PATH"

echo "release-dmg.sh: done."
echo "  Signed, notarized, stapled: $DMG_PATH"
