#!/bin/sh
# Собирает релизный бинарник и упаковывает его в dist/Goflake.app.
set -e
cd "$(dirname "$0")/.."
cargo build --release
APP=dist/Goflake.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp target/release/goflake "$APP/Contents/MacOS/Goflake"
mkdir -p "$APP/Contents/Resources"
cp assets/Goflake.icns "$APP/Contents/Resources/Goflake.icns"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>local.goflake</string>
<key>CFBundleName</key><string>Сугроб</string>
<key>CFBundleExecutable</key><string>Goflake</string>
<key>CFBundleIconFile</key><string>Goflake</string>
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
if [ -n "$SIGN_ID" ]; then
  # Релиз: подпись Developer ID с «усиленной средой выполнения» и меткой времени — без них
  # нотаризацию не пройти. Требование по идентификатору здесь не нужно: у такой подписи оно
  # и так привязано к команде разработчика и не меняется от сборки к сборке.
  codesign --force --options runtime --timestamp -s "$SIGN_ID" "$APP"
else
  codesign --force -s - --requirements '=designated => identifier "local.goflake"' "$APP"
fi
echo "Готово: $(pwd)/$APP"
