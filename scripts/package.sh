#!/usr/bin/env bash
# Build DiskLens.app and a drag-to-install DiskLens-<version>.dmg in ./dist.
#
# Usage: scripts/package.sh [--universal]
#   --universal  build for Apple silicon and Intel and merge with lipo
#                (needs: rustup target add x86_64-apple-darwin)
#
# The app is ad-hoc signed, not Developer ID signed or notarized, so
# Gatekeeper asks users to approve it once (see README).
set -euo pipefail

cd "$(dirname "$0")/.."

NAME="DiskLens"
BIN="disklens"
BUNDLE_ID="com.devground-labs.disklens"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
DIST="dist"
APP="$DIST/$NAME.app"
DMG="$DIST/$NAME-$VERSION.dmg"

rm -rf "$DIST"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# --- binary -----------------------------------------------------------------
if [[ "${1:-}" == "--universal" ]]; then
  cargo build --release --locked --target aarch64-apple-darwin
  cargo build --release --locked --target x86_64-apple-darwin
  lipo -create \
    "target/aarch64-apple-darwin/release/$BIN" \
    "target/x86_64-apple-darwin/release/$BIN" \
    -output "$APP/Contents/MacOS/$BIN"
else
  cargo build --release --locked
  cp "target/release/$BIN" "$APP/Contents/MacOS/$BIN"
fi

# --- icon -------------------------------------------------------------------
ICONSET="$(mktemp -d)/$NAME.iconset"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" assets/icon.png --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  sips -z "$((size * 2))" "$((size * 2))" assets/icon.png --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/$NAME.icns"

# --- Info.plist -------------------------------------------------------------
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$NAME</string>
  <key>CFBundleDisplayName</key><string>$NAME</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key><string>$BIN</string>
  <key>CFBundleIconFile</key><string>$NAME</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

# --- ad-hoc sign (gives the bundle a stable identity; not notarization) ------
codesign --force --deep --sign - "$APP"

# --- dmg --------------------------------------------------------------------
STAGE="$(mktemp -d)"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "$NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null

# Unversioned copy, so releases/latest/download/DiskLens.dmg always resolves.
cp "$DMG" "$DIST/$NAME.dmg"

echo "Built $DMG and $DIST/$NAME.dmg"
shasum -a 256 "$DMG"
