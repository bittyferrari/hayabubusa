#!/bin/bash
# Build Hayabubusa.app on macOS
# Usage: ./build-mac.sh            (build for current arch)
#        ./build-mac.sh install    (build + copy to /Applications + register file types)
#        ./build-mac.sh universal  (build arm64 + x86_64 universal binary)
set -euo pipefail
cd "$(dirname "$0")"

if [[ "$(uname)" != "Darwin" ]]; then
  echo "This script must be run on macOS." >&2
  exit 1
fi

BIN_NAME="hayabubusa"
APP_NAME="Hayabubusa"
BUNDLE_ID="com.bittyferrari.hayabubusa"
ICON_SRC="assets/icon.png"
VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"(.*)".*/\1/')"

OUT_DIR="target/release"
if [[ " $* " == *" universal "* ]]; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  mkdir -p "$OUT_DIR"
  lipo -create \
    "target/aarch64-apple-darwin/release/$BIN_NAME" \
    "target/x86_64-apple-darwin/release/$BIN_NAME" \
    -output "$OUT_DIR/$BIN_NAME-universal"
  BIN_PATH="$OUT_DIR/$BIN_NAME-universal"
else
  cargo build --release
  BIN_PATH="$OUT_DIR/$BIN_NAME"
fi

APP="$OUT_DIR/$APP_NAME.app"
echo "==> Creating $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN_PATH" "$APP/Contents/MacOS/$BIN_NAME"
chmod +x "$APP/Contents/MacOS/$BIN_NAME"

# Icon: png -> icns
if [[ -f "$ICON_SRC" ]]; then
  echo "==> Generating AppIcon.icns"
  ICONSET="target/AppIcon.iconset"
  rm -rf "$ICONSET"; mkdir -p "$ICONSET"
  for s in 16 32 128 256 512; do
    sips -z $s $s "$ICON_SRC" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    sips -z $((s*2)) $((s*2)) "$ICON_SRC" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
  done
  iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
  rm -rf "$ICONSET"
else
  echo "warning: $ICON_SRC not found, skipping icon" >&2
fi

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$APP_NAME</string>
  <key>CFBundleDisplayName</key><string>$APP_NAME</string>
  <key>CFBundleExecutable</key><string>$BIN_NAME</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.photography</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>Image</string>
      <key>CFBundleTypeRole</key><string>Viewer</string>
      <key>LSHandlerRank</key><string>Alternate</string>
      <key>LSItemContentTypes</key>
      <array>
        <string>public.jpeg</string>
        <string>public.png</string>
        <string>org.webmproject.webp</string>
        <string>com.compuserve.gif</string>
        <string>com.microsoft.bmp</string>
        <string>public.tiff</string>
        <string>com.microsoft.ico</string>
        <string>com.truevision.tga-image</string>
        <string>public.image</string>
      </array>
    </dict>
  </array>
</dict>
</plist>
PLIST

echo "==> Ad-hoc code signing"
codesign --force --deep -s - "$APP"

LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"

# ./build-mac.sh install  (or: ./build-mac.sh universal install)
if [[ " $* " == *" install "* ]]; then
  echo "==> Installing to /Applications"
  rm -rf "/Applications/$APP_NAME.app"
  cp -R "$APP" /Applications/
  "$LSREGISTER" -f "/Applications/$APP_NAME.app"
  APP="/Applications/$APP_NAME.app"
else
  "$LSREGISTER" -f "$APP"
fi

echo ""
echo "Done: $APP"
echo "Run:   open \"$APP\""
echo "Install: cp -R \"$APP\" /Applications/"
