#!/bin/sh
# Собирает релизный бинарник и упаковывает его в dist/Snowdrift.app.
set -e
cd "$(dirname "$0")/.."
cargo build --release
APP=dist/Snowdrift.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp target/release/snowdrift "$APP/Contents/MacOS/Snowdrift"
mkdir -p "$APP/Contents/Resources"
cp assets/Snowdrift.icns "$APP/Contents/Resources/Snowdrift.icns"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>local.snowdrift</string>
<key>CFBundleName</key><string>Сугроб</string>
<key>CFBundleExecutable</key><string>Snowdrift</string>
<key>CFBundleIconFile</key><string>Snowdrift</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$VERSION</string>
<key>LSMinimumSystemVersion</key><string>11.0</string>
<key>LSUIElement</key><true/>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
# Подпись «ad hoc» по умолчанию привязывает приложение к хэшу сборки, и разрешение в
# Универсальном доступе слетает после каждой пересборки. Явное требование «по
# идентификатору» этого не допускает: система узнаёт приложение по bundle id.
codesign --force -s - --requirements '=designated => identifier "local.snowdrift"' "$APP"
echo "Готово: $(pwd)/$APP"
