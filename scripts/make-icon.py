#!/usr/bin/env python3
"""Draws the app icon with no image libraries.

A yellow legal pad with a brown binding and white ruled lines, on a
transparent background. Shapes are signed-distance functions, so edges are
anti-aliased without supersampling, and each size is drawn directly (small
sizes get fewer, bolder lines instead of a blurry downscale).

Usage:
  make-icon.py OUT.png        1024x1024 PNG (macOS icon grid, 100 px margin)
  make-icon.py --ico OUT.ico  Windows icon, 16 to 256 px, filling more of
                              each square as Windows icons do; also writes
                              its 256 px image as OUT-256.png (the window
                              and taskbar icon)
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


def pixel(px, py, scale=1.0, geo=None):
    """Colour at design point (px, py) of the 1024-unit icon, where one
    design unit is `scale` output pixels. `geo` is the shapes' geometry
    (from `snapped`), or the full-size design."""
    pad, binding_bottom, edge, lines, line_half = geo or (PAD, BINDING_BOTTOM, 14, LINES, 10)
    cx, cy, hw, hh, r = pad

    def cov(d):
        return coverage(d * scale)

    # Pad: fills the macOS icon grid height (824 px, 100 px margin).
    a_pad = cov(rounded_rect(px, py, cx, cy, hw, hh, r))
    if a_pad == 0.0:
        return (0, 0, 0, 0)
    top = cy - hh
    col = mix((0xFF, 0xE0, 0x5C), (0xF6, 0xC4, 0x3A), (py - top) / (2 * hh))
    # Brown binding across the top, with a darker edge below it.
    if py < binding_bottom:
        col = mix((0x8A, 0x58, 0x2A), (0x5E, 0x3A, 0x1A), (py - top) / (binding_bottom - top))
    elif py < binding_bottom + edge:
        col = (0xD9, 0xA4, 0x2E)
    # White ruled lines.
    for ly in lines:
        d_line = rounded_rect(px, py, cx, ly, hw - 70, line_half, line_half)
        col = blend(col, (0xFF, 0xFF, 0xFF), 0.95 * cov(d_line))
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


def snapped(n, scale):
    """The design's geometry for an n-pixel icon, with the pad's edges, the
    binding and the ruled lines on whole pixels so small sizes stay sharp.
    Small icons get fewer lines, each at least a pixel thick."""

    def to_px(u):
        return n / 2 + (u - SIZE / 2) * scale

    def to_units(p):
        return SIZE / 2 + (p - n / 2) / scale

    cx, cy, hw, hh, r = PAD
    left, top = round(to_px(cx - hw)), round(to_px(cy - hh))
    right, bottom = n - left, n - top
    pad = (SIZE / 2, SIZE / 2, (right - left) / 2 / scale, (bottom - top) / 2 / scale,
           max(r, 1.5 / scale) if n <= 24 else r)
    bind = max(top + 2, round(to_px(BINDING_BOTTOM)))
    edge_px = round(14 * scale) if n >= 32 else 0
    count = 6 if n >= 64 else 5 if n >= 48 else 4 if n >= 32 else 3
    thick = max(1, round(20 * scale))
    # Evenly spaced (a whole number of pixels apart) and centred between
    # the binding and the bottom.
    first, last = bind + edge_px + max(2, round(n / 10)), bottom - max(2, round(n / 8)) - thick
    step = (last - first) // (count - 1)
    first += (last - first - step * (count - 1)) // 2
    rows = [first + i * step for i in range(count)]
    lines = [to_units(y + thick / 2) for y in rows]
    return (pad, to_units(bind), edge_px / scale, lines, thick / 2 / scale)


def render(n, zoom=1.0):
    """Rows of an n x n image; `zoom` enlarges the pad about the centre."""
    scale = n * zoom / SIZE
    geo = snapped(n, scale)
    rows = []
    for y in range(n):
        row = []
        for x in range(n):
            px = SIZE / 2 + ((x + 0.5) / scale - n / scale / 2)
            py = SIZE / 2 + ((y + 0.5) / scale - n / scale / 2)
            row.append(pixel(px, py, scale, geo))
        rows.append(row)
    return rows


def png_bytes(w, h, rows):
    def chunk(kind, data):
        c = struct.pack(">I", len(data)) + kind + data
        return c + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    raw = b"".join(b"\x00" + bytes(v for p in row for v in p) for row in rows)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]
# The pad is 824 units tall; Windows icons leave a few percent of margin.
ICO_ZOOM = 1.15


def write_ico(path):
    """An .ico of PNG images (supported since Windows Vista)."""
    images = [png_bytes(n, n, render(n, ICO_ZOOM)) for n in ICO_SIZES]
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries = b""
    for n, data in zip(ICO_SIZES, images):
        # Width/height 0 means 256.
        entries += struct.pack("<BBBBHHII", n % 256, n % 256, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    with open(path, "wb") as f:
        f.write(header + entries + b"".join(images))
    with open(path.rsplit(".", 1)[0] + "-256.png", "wb") as f:
        f.write(images[ICO_SIZES.index(256)])


if __name__ == "__main__":
    if sys.argv[1] == "--ico":
        write_ico(sys.argv[2])
    else:
        rows = [[pixel(x + 0.5, y + 0.5) for x in range(SIZE)] for y in range(SIZE)]
        write_png(sys.argv[1], SIZE, SIZE, rows)
