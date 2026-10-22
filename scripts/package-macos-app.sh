#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="BigHead"
PRODUCT_NAME="BigHeadMenuBar"
DIST_DIR="$ROOT/dist/macos"
APP_DIR="$DIST_DIR/$APP_NAME.app"
CONTENTS_DIR="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"

cd "$ROOT"

echo "==> Building Rust runtime"
cargo build --release --bin big-head

echo "==> Building Swift desktop app"
swift build --package-path external/big-head-menubar -c release

echo "==> Assembling $APP_NAME.app"
rm -rf "$APP_DIR"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

cp "$ROOT/external/big-head-menubar/.build/release/$PRODUCT_NAME" "$MACOS_DIR/$APP_NAME"
cp "$ROOT/target/release/big-head" "$RESOURCES_DIR/big-head-runtime"
chmod +x "$MACOS_DIR/$APP_NAME" "$RESOURCES_DIR/big-head-runtime"

cat > "$CONTENTS_DIR/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>BigHead</string>
  <key>CFBundleExecutable</key>
  <string>BigHead</string>
  <key>CFBundleIdentifier</key>
  <string>local.bighead.desktop</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>BigHead</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
PLIST

cat > "$RESOURCES_DIR/README-FIRST.txt" <<'TXT'
BigHead Desktop

This app includes the Big Head Rust runtime.

Runtime data directory:
  ~/Library/Application Support/BigHead

Create this file before first use:
  ~/Library/Application Support/BigHead/.env

Example:
  MiniMax_API_KEY=your-key-here

The desktop app starts the bundled runtime with:
  --view-server --headless
TXT

echo "==> Creating zip"
rm -f "$DIST_DIR/$APP_NAME-macOS.zip"
(
  cd "$DIST_DIR"
  ditto -c -k --sequesterRsrc --keepParent "$APP_NAME.app" "$APP_NAME-macOS.zip"
)

echo "==> Done"
echo "App: $APP_DIR"
echo "Zip: $DIST_DIR/$APP_NAME-macOS.zip"

