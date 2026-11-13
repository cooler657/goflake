#!/bin/sh
# Делает из assets/icon-1024.png все остальные форматы иконки:
#   assets/Goflake.icns     — macOS (значок приложения)
#   assets/icon.ico           — Windows (встраивается в goflake.exe)
#   assets/goflake-256.png  — Linux (рядом лежит goflake.desktop)
# Нужны macOS-инструменты sips и iconutil. Сама картинка — tools/make_icon.py.
set -e
cd "$(dirname "$0")/.."
python3 tools/make_icon.py assets/icon-1024.png

SET=$(mktemp -d)/Goflake.iconset
mkdir -p "$SET"
resize() { sips -z "$1" "$1" assets/icon-1024.png --out "$2" >/dev/null; }
for s in 16 32 128 256 512; do
  resize "$s" "$SET/icon_${s}x${s}.png"
  resize "$((s * 2))" "$SET/icon_${s}x${s}@2x.png"
done
iconutil -c icns "$SET" -o assets/Goflake.icns

resize 256 assets/goflake-256.png

# .ico: PNG внутри, как понимают Windows Vista и новее
TMP=$(mktemp -d)
for s in 16 24 32 48 64 128 256; do resize "$s" "$TMP/$s.png"; done
python3 - "$TMP" <<'PY'
import struct, sys, os
tmp = sys.argv[1]
sizes = [16, 24, 32, 48, 64, 128, 256]
pngs = [open(os.path.join(tmp, f"{s}.png"), "rb").read() for s in sizes]
head = struct.pack("<HHH", 0, 1, len(sizes))
offset = 6 + 16 * len(sizes)
entries = b""
for s, data in zip(sizes, pngs):
    entries += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
open("assets/icon.ico", "wb").write(head + entries + b"".join(pngs))
PY
ls -l assets
