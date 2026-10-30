#!/usr/bin/env python3
"""Draws the app icon (1024x1024 PNG) with no image libraries.

A yellow legal pad with a brown binding and white ruled lines, on a
transparent background. Shapes are signed-distance functions, so edges are
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


def coverage(d):
    return max(0.0, min(1.0, 0.5 - d))


def blend(dst, src, a):
    return tuple(dst[i] * (1 - a) + src[i] * a for i in range(3))


PAD = (512, 512, 330, 412, 80)  # centre x, centre y, half width, half height, radius
BINDING_BOTTOM = 260
LINES = [(375 + i * 95) for i in range(6)]


def mix(a, b, t):
    return tuple(a[i] * (1 - t) + b[i] * t for i in range(3))


def pixel(x, y):
    px, py = x + 0.5, y + 0.5
    cx, cy, hw, hh, r = PAD
    # Pad: fills the macOS icon grid height (824 px, 100 px margin).
    a_pad = coverage(rounded_rect(px, py, cx, cy, hw, hh, r))
    if a_pad == 0.0:
        return (0, 0, 0, 0)
    top = cy - hh
    col = mix((0xFF, 0xE0, 0x5C), (0xF6, 0xC4, 0x3A), (py - top) / (2 * hh))
    # Brown binding across the top, with a darker edge below it.
    if py < BINDING_BOTTOM:
        col = mix((0x8A, 0x58, 0x2A), (0x5E, 0x3A, 0x1A), (py - top) / (BINDING_BOTTOM - top))
    elif py < BINDING_BOTTOM + 14:
        col = (0xD9, 0xA4, 0x2E)
    # White ruled lines.
    for ly in LINES:
        d_line = rounded_rect(px, py, cx, ly, hw - 70, 10, 10)
        col = blend(col, (0xFF, 0xFF, 0xFF), 0.95 * coverage(d_line))
    return (round(col[0]), round(col[1]), round(col[2]), round(255 * a_pad))


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
