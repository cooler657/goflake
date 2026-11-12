#!/usr/bin/env python3
"""Рисует иконку приложения: assets/icon-1024.png.

Без внешних библиотек: сглаженные фигуры считаются по расстоянию до них, картинка
пишется в PNG через zlib. Запуск: python3 tools/make_icon.py
Остальные размеры, .icns и .ico делает tools/build_icons.sh из этого файла.
"""
import math
import struct
import sys
import zlib

W = 1024
N = W * W
R = [0.0] * N
G = [0.0] * N
B = [0.0] * N
A = [0.0] * N  # прозрачность слоёв; итоговая маска — скруглённый квадрат


def clamp(x, lo=0.0, hi=1.0):
    return lo if x < lo else hi if x > hi else x


def mix(c1, c2, t):
    return tuple(a + (b - a) * t for a, b in zip(c1, c2))


def blend(i, col, a):
    """Кладёт цвет с прозрачностью `a` поверх пикселя `i` (все слои непрозрачны под ним)."""
    if a <= 0.0:
        return
    R[i] += (col[0] - R[i]) * a
    G[i] += (col[1] - G[i]) * a
    B[i] += (col[2] - B[i]) * a


def disc(cx, cy, r, col, alpha=1.0, soft=False):
    """Круг; `soft` — мягкое свечение, яркость падает к краю по квадрату."""
    x0, x1 = max(0, int(cx - r - 2)), min(W, int(cx + r + 3))
    y0, y1 = max(0, int(cy - r - 2)), min(W, int(cy + r + 3))
    for y in range(y0, y1):
        for x in range(x0, x1):
            d = math.hypot(x + 0.5 - cx, y + 0.5 - cy)
            if soft:
                g = clamp(1.0 - d / r)
                a = alpha * g * g
            else:
                a = alpha * clamp(r - d + 0.5)
            blend(y * W + x, col, a)


def segment(p0, p1, width, col, alpha=1.0):
    """Отрезок с круглыми концами."""
    (ax, ay), (bx, by) = p0, p1
    pad = width / 2 + 2
    x0, x1 = max(0, int(min(ax, bx) - pad)), min(W, int(max(ax, bx) + pad + 1))
    y0, y1 = max(0, int(min(ay, by) - pad)), min(W, int(max(ay, by) + pad + 1))
    dx, dy = bx - ax, by - ay
    ll = dx * dx + dy * dy or 1.0
    for y in range(y0, y1):
        for x in range(x0, x1):
            px, py = x + 0.5 - ax, y + 0.5 - ay
            t = clamp((px * dx + py * dy) / ll)
            d = math.hypot(px - dx * t, py - dy * t)
            blend(y * W + x, col, alpha * clamp(width / 2 - d + 0.5))


def snowflake(cx, cy, radius, col, alpha, offset=(0.0, 0.0)):
    ox, oy = offset
    for k in range(6):
        ang = math.radians(60 * k - 90)
        ux, uy = math.cos(ang), math.sin(ang)
        tip = (cx + ox + ux * radius, cy + oy + uy * radius)
        segment((cx + ox, cy + oy), tip, 38, col, alpha)
        # боковые веточки на двух высотах: ближе к центру они длиннее
        for t, length, width in ((0.46, 0.36, 28), (0.72, 0.24, 24)):
            bx, by = cx + ox + ux * radius * t, cy + oy + uy * radius * t
            for side in (-1, 1):
                a2 = ang + side * math.radians(58)
                segment((bx, by), (bx + math.cos(a2) * radius * length, by + math.sin(a2) * radius * length), width, col, alpha)
        disc(tip[0], tip[1], 24, col, alpha)
    disc(cx + ox, cy + oy, 56, col, alpha)


def drift_edge(x):
    """Верхняя кромка сугроба: два холма и мелкая рябь."""
    return (
        812
        - 74 * math.exp(-(((x - 300) / 250) ** 2))
        - 48 * math.exp(-(((x - 770) / 210) ** 2))
        + 5 * math.sin(x / 31.0)
    )


def render():
    # 1. фон: вечернее небо и мягкое свечение за снежинкой
    top, bottom = (10, 22, 62), (44, 98, 168)
    for y in range(W):
        row = mix(top, bottom, (y / W) ** 1.1)
        for x in range(W):
            i = y * W + x
            R[i], G[i], B[i], A[i] = row[0], row[1], row[2], 1.0
    disc(512, 410, 470, (120, 175, 255), 0.55, soft=True)

    # 2. падающие снежинки-точки (псевдослучайно, но всегда одинаково)
    seed = 7
    for _ in range(26):
        seed = (seed * 1103515245 + 12345) & 0x7FFFFFFF
        x = 110 + (seed % 804)
        seed = (seed * 1103515245 + 12345) & 0x7FFFFFFF
        y = 110 + (seed % 560)
        seed = (seed * 1103515245 + 12345) & 0x7FFFFFFF
        r = 4 + seed % 8
        if math.hypot(x - 512, y - 420) < 320:  # центр оставляем снежинке
            continue
        disc(x, y, r, (235, 244, 255), 0.35 + 0.1 * (r % 5))

    # 3. снежинка: тень для объёма, затем сама
    snowflake(512, 420, 280, (14, 38, 96), 0.35, offset=(0, 13))
    snowflake(512, 420, 280, (247, 251, 255), 1.0)

    # 4. сугроб: белая гора с синеватой тенью к низу
    for x in range(W):
        e = drift_edge(x)
        for y in range(max(0, int(e) - 2), W):
            a = clamp(y + 0.5 - e + 0.5)
            t = clamp((y - e) / 220.0)
            blend(y * W + x, mix((255, 255, 255), (196, 218, 248), t), a)
        # лёгкая тень над кромкой, чтобы гора читалась на светлом небе
        for y in range(max(0, int(e) - 9), int(e)):
            blend(y * W + x, (20, 50, 110), 0.10 * clamp((y - (e - 9)) / 9.0))

    # 5. скруглённый квадрат (суперэллипс), как у значков macOS: тело 824 из 1024
    a, n = 412.0, 5.0
    out = bytearray(N * 4)
    for y in range(W):
        for x in range(W):
            dx, dy = abs(x + 0.5 - 512) / a, abs(y + 0.5 - 512) / a
            f = dx**n + dy**n
            cover = clamp((1.0 - f ** (1.0 / n)) * a + 0.5) if f < 1.6 else 0.0
            i = y * W + x
            j = i * 4
            out[j] = int(clamp(R[i] / 255.0) * 255 + 0.5)
            out[j + 1] = int(clamp(G[i] / 255.0) * 255 + 0.5)
            out[j + 2] = int(clamp(B[i] / 255.0) * 255 + 0.5)
            out[j + 3] = int(cover * 255 + 0.5)
    return bytes(out)


def write_png(path, rgba, size):
    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    raw = b"".join(b"\x00" + rgba[y * size * 4 : (y + 1) * size * 4] for y in range(size))
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )
    with open(path, "wb") as f:
        f.write(png)


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "assets/icon-1024.png"
    write_png(out, render(), W)
    print("готово:", out)
