#!/bin/sh
# Сборка Goflake.app рядом с исходником.
set -e
cd "$(dirname "$0")"
APP=Goflake.app
mkdir -p "$APP/Contents/MacOS"
cat > "$APP/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>local.goflake</string>
<key>CFBundleName</key><string>Сугроб</string>
<key>CFBundleExecutable</key><string>Goflake</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>1.0</string>
<key>LSMinimumSystemVersion</key><string>14.0</string>
<key>LSUIElement</key><true/>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
EOF
swiftc -O main.swift -o "$APP/Contents/MacOS/Goflake"
codesign --force -s - "$APP"
echo "Готово: $(pwd)/$APP"
