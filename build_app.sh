#!/usr/bin/env bash
# Собирает Desktop Cleaner.app и устанавливает его в /Applications.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

APP_NAME="Desktop Cleaner"
BUNDLE_ID="com.local.desktopcleaner"
APP_DIR="$ROOT_DIR/build/$APP_NAME.app"
CONTENTS_DIR="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"
ICONSET_DIR="$ROOT_DIR/build/AppIcon.iconset"
ICON_PNG="$ROOT_DIR/build/AppIcon-1024.png"
ZIP_PATH="$ROOT_DIR/DesktopCleaner-macOS-AppleSilicon.zip"
DMG_PATH="$ROOT_DIR/DesktopCleaner-macOS-AppleSilicon.dmg"
DMG_STAGING_DIR="$ROOT_DIR/build/dmg"

echo "==> cargo build --release"
cargo build --release

echo "==> Сборка бандла $APP_NAME.app"
rm -rf "$APP_DIR" "$ICONSET_DIR"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR" "$ICONSET_DIR"

cp "$ROOT_DIR/target/release/desktop_cleaner" "$MACOS_DIR/desktop_cleaner"
chmod +x "$MACOS_DIR/desktop_cleaner"

echo "==> Генерация иконки"
python3 - "$ICON_PNG" <<'PY'
import struct
import sys
import zlib

SIZE = 1024
OUT = sys.argv[1]
BG = (31, 33, 38)
ACCENT = (182, 240, 90)
SAMPLES = 2


def rounded_rect(x, y, left, top, right, bottom, radius):
    cx = min(max(x, left + radius), right - radius)
    cy = min(max(y, top + radius), bottom - radius)
    dx = x - cx
    dy = y - cy
    return dx * dx + dy * dy <= radius * radius


def glyph(x, y):
    outer = rounded_rect(x, y, 270, 310, 754, 714, 80)
    inner = rounded_rect(x, y, 314, 354, 710, 670, 46)
    frame = outer and not inner
    shaft = 488 <= x <= 536 and 380 <= y <= 540
    head = 520 <= y <= 650 and abs(x - 512) <= (650 - y) * 0.62
    return frame or shaft or head


raw = bytearray()
total_samples = SAMPLES * SAMPLES
step = 1.0 / SAMPLES

for y in range(SIZE):
    raw.append(0)  # PNG filter: none
    for x in range(SIZE):
        bg_hits = 0
        glyph_hits = 0
        for sy in range(SAMPLES):
            py = y + (sy + 0.5) * step
            for sx in range(SAMPLES):
                px = x + (sx + 0.5) * step
                if rounded_rect(px, py, 64, 64, 960, 960, 210):
                    bg_hits += 1
                    if glyph(px, py):
                        glyph_hits += 1

        if bg_hits == 0:
            raw.extend((0, 0, 0, 0))
            continue

        bg_alpha = bg_hits / total_samples
        accent = glyph_hits / total_samples
        r = round(BG[0] * (1 - accent) + ACCENT[0] * accent)
        g = round(BG[1] * (1 - accent) + ACCENT[1] * accent)
        b = round(BG[2] * (1 - accent) + ACCENT[2] * accent)
        raw.extend((r, g, b, round(bg_alpha * 255)))


def chunk(tag, data):
    return (
        struct.pack(">I", len(data))
        + tag
        + data
        + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    )


png = b"\x89PNG\r\n\x1a\n"
png += chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
png += chunk(b"IEND", b"")

with open(OUT, "wb") as handle:
    handle.write(png)
PY

make_icon() {
    sips -z "$1" "$1" "$ICON_PNG" --out "$ICONSET_DIR/$2.png" >/dev/null
}

make_icon 16   "icon_16x16"
make_icon 32   "icon_16x16@2x"
make_icon 32   "icon_32x32"
make_icon 64   "icon_32x32@2x"
make_icon 128  "icon_128x128"
make_icon 256  "icon_128x128@2x"
make_icon 256  "icon_256x256"
make_icon 512  "icon_256x256@2x"
make_icon 512  "icon_512x512"
make_icon 1024 "icon_512x512@2x"

iconutil -c icns "$ICONSET_DIR" -o "$RESOURCES_DIR/AppIcon.icns"

cat > "$CONTENTS_DIR/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>$APP_NAME</string>
    <key>CFBundleDisplayName</key>
    <string>$APP_NAME</string>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleExecutable</key>
    <string>desktop_cleaner</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleVersion</key>
    <string>1.1.0</string>
    <key>CFBundleShortVersionString</key>
    <string>1.1.0</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSHumanReadableCopyright</key>
    <string>Desktop Cleaner</string>
</dict>
</plist>
PLIST

echo "==> Установка в /Applications"
rm -rf "/Applications/$APP_NAME.app"
ditto "$APP_DIR" "/Applications/$APP_NAME.app"

echo "==> Упаковка релизного архива"
rm -f "$ZIP_PATH"
ditto -c -k --sequesterRsrc --keepParent "$APP_DIR" "$ZIP_PATH"

echo "==> Сборка установочного образа"
rm -rf "$DMG_STAGING_DIR"
mkdir -p "$DMG_STAGING_DIR"
ditto "$APP_DIR" "$DMG_STAGING_DIR/$APP_NAME.app"
ln -s /Applications "$DMG_STAGING_DIR/Applications"
rm -f "$DMG_PATH"
hdiutil create -volname "$APP_NAME" -srcfolder "$DMG_STAGING_DIR" -ov -format UDZO "$DMG_PATH" >/dev/null

echo "Готово: /Applications/$APP_NAME.app"
echo "Архив для GitHub Releases: $ZIP_PATH"
echo "Установочный образ: $DMG_PATH"
