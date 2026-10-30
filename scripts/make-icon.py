#!/usr/bin/env python3
"""Draws the app icon (1024x1024 PNG) with no image libraries.

A green rounded square holding a notepad page with text lines and a
lightning bolt. Shapes are signed-distance functions, so edges are
anti-aliased without supersampling.

Usage: make-icon.py OUT.png
"""

import math
import struct
import sys
import zlib

SIZE = 1024


def rounded_rect(px, py, cx, cy, hw, hh, r):
    """Signed distance to a rounded rectangle (negative inside)."""
    qx = abs(px - cx) - hw + r
    qy = abs(py - cy) - hh + r
    outside = math.hypot(max(qx, 0.0), max(qy, 0.0))
    return outside + min(max(qx, qy), 0.0) - r


def polygon(px, py, pts):
    """Signed distance to a simple polygon (negative inside)."""
    d = float("inf")
    inside = False
    n = len(pts)
    for i in range(n):
        ax, ay = pts[i]
        bx, by = pts[(i + 1) % n]
        ex, ey = bx - ax, by - ay
        wx, wy = px - ax, py - ay
        t = max(0.0, min(1.0, (wx * ex + wy * ey) / (ex * ex + ey * ey)))
        d = min(d, math.hypot(wx - ex * t, wy - ey * t))
        if (ay > py) != (by > py) and px < ax + (py - ay) * ex / ey:
            inside = not inside
    return -d if inside else d


def coverage(d):
    return max(0.0, min(1.0, 0.5 - d))


def blend(dst, src, a):
    return tuple(dst[i] * (1 - a) + src[i] * a for i in range(3))


BOLT = [(640, 250), (470, 540), (575, 540), (500, 790), (700, 470), (590, 470), (660, 250)]
LINES = [(395 + i * 82) for i in range(5)]


def pixel(x, y):
    px, py = x + 0.5, y + 0.5
    # Background tile: macOS icon grid (824 px square, 100 px margin).
    d_tile = rounded_rect(px, py, 512, 512, 412, 412, 185)
    a_tile = coverage(d_tile)
    if a_tile == 0.0:
        return (0, 0, 0, 0)
    t = py / SIZE
    col = (0x43 * (1 - t) + 0x2A * t, 0xA0 * (1 - t) + 0x75 * t, 0x47 * (1 - t) + 0x33 * t)
    # Soft shadow under the page.
    d_shadow = rounded_rect(px, py + 14, 512, 530, 270, 330, 36)
    col = blend(col, (0x10, 0x30, 0x18), 0.35 * max(0.0, min(1.0, 0.5 - d_shadow / 24)))
    # Page.
    d_page = rounded_rect(px, py, 512, 516, 270, 330, 36)
    col = blend(col, (0xF7, 0xF8, 0xF2), coverage(d_page))
    if d_page < 0:
        # Header band.
        if py < 290:
            col = blend(col, (0xE0, 0xE6, 0xDC), 1.0)
        # Text lines (shorter on the right where the bolt is).
        for i, ly in enumerate(LINES):
            right = 700 if i % 2 == 0 else 620
            d_line = rounded_rect(px, py, (300 + right) / 2, ly, (right - 300) / 2, 11, 11)
            col = blend(col, (0x8A, 0xA2, 0xBE), coverage(d_line))
    # Lightning bolt with a dark outline.
    d_bolt = polygon(px, py, BOLT)
    col = blend(col, (0x6B, 0x4A, 0x00), coverage(d_bolt - 9))
    col = blend(col, (0xFF, 0xC8, 0x2E), coverage(d_bolt))
    return (round(col[0]), round(col[1]), round(col[2]), round(255 * a_tile))


def write_png(path, w, h, rows):
    def chunk(kind, data):
        c = struct.pack(">I", len(data)) + kind + data
        return c + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    raw = b"".join(b"\x00" + bytes(v for p in row for v in p) for row in rows)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(raw, 9)))
        f.write(chunk(b"IEND", b""))


if __name__ == "__main__":
    rows = [[pixel(x, y) for x in range(SIZE)] for y in range(SIZE)]
    write_png(sys.argv[1], SIZE, SIZE, rows)
