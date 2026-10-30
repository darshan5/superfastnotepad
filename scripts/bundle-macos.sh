#!/bin/sh
# Builds superfastnotepad.app for macOS.
#
# Usage: scripts/bundle-macos.sh [target]   (default: aarch64-apple-darwin)
# Output: target/<target>/release/superfastnotepad.app
set -eu

TARGET="${1:-aarch64-apple-darwin}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

NAME=superfastnotepad
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
ICNS="assets/macos/AppIcon.icns"

cargo build --release --target "$TARGET"

# The icon is generated once and kept in the repo; rebuild it if missing.
if [ ! -f "$ICNS" ]; then
    TMP="$(mktemp -d)"
    python3 scripts/make-icon.py "$TMP/icon.png"
    SET="$TMP/AppIcon.iconset"
    mkdir "$SET"
    for s in 16 32 128 256 512; do
        sips -z $s $s "$TMP/icon.png" --out "$SET/icon_${s}x${s}.png" >/dev/null
        d=$((s * 2))
        sips -z $d $d "$TMP/icon.png" --out "$SET/icon_${s}x${s}@2x.png" >/dev/null
    done
    mkdir -p "$(dirname "$ICNS")"
    iconutil -c icns "$SET" -o "$ICNS"
    rm -rf "$TMP"
fi

APP="target/$TARGET/release/$NAME.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "target/$TARGET/release/$NAME" "$APP/Contents/MacOS/$NAME"
cp "$ICNS" "$APP/Contents/Resources/AppIcon.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>$NAME</string>
    <key>CFBundleDisplayName</key>
    <string>$NAME</string>
    <key>CFBundleIdentifier</key>
    <string>org.superfastnotepad.superfastnotepad</string>
    <key>CFBundleExecutable</key>
    <string>$NAME</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key>
            <string>Text and source code</string>
            <key>CFBundleTypeRole</key>
            <string>Editor</string>
            <key>LSHandlerRank</key>
            <string>Alternate</string>
            <key>LSItemContentTypes</key>
            <array>
                <string>public.text</string>
                <string>public.plain-text</string>
                <string>public.source-code</string>
                <string>public.script</string>
                <string>public.shell-script</string>
                <string>public.json</string>
                <string>public.xml</string>
                <string>public.html</string>
                <string>public.css</string>
                <string>public.log</string>
                <string>public.comma-separated-values-text</string>
                <string>net.daringfireball.markdown</string>
            </array>
        </dict>
    </array>
</dict>
</plist>
PLIST

# Ad-hoc signature so the bundle (binary + Info.plist) is sealed; this is
# not a Developer ID signature, so other Macs still show a first-run warning.
codesign --force --sign - "$APP"
echo "Built $APP"
