# Usage: python3 app/scripts/tray_icon.py app/src-tauri/icons/tray.png
# Draws a monochrome microphone glyph as a PNG (black on transparent), for
# use as a macOS template tray icon. Shapes are signed-distance functions,
# anti-aliased with 4x4 supersampling.
import math, struct, sys, zlib

def capsule(x, y, cx, top, bottom, r):
    yy = min(max(y, top + r), bottom - r)
    return math.hypot(x - cx, y - yy) - r

def arc(x, y, cx, cy, r, w):  # lower half ring
    d = abs(math.hypot(x - cx, y - cy) - r) - w / 2
    return d if y >= cy else max(d, cy - y)

def rect(x, y, x0, y0, x1, y1):
    dx, dy = max(x0 - x, 0, x - x1), max(y0 - y, 0, y - y1)
    inside = min(max(x0 - x, x - x1), max(y0 - y, y - y1))
    return math.hypot(dx, dy) if (dx or dy) else inside

def shape(x, y):  # coordinates in a 0..18 box
    return min(
        capsule(x, y, 9, 1.5, 11, 3.2),
        arc(x, y, 9, 8.5, 5.6, 1.5),
        rect(x, y, 8.25, 14, 9.75, 16.5),
        rect(x, y, 5.5, 15.75, 12.5, 17.0),
    )

def png(size, path):
    s = size / 18
    rows = []
    for py in range(size):
        row = bytearray([0])
        for px in range(size):
            cov = sum(shape((px + (i + .5) / 4) / s, (py + (j + .5) / 4) / s) <= 0
                      for i in range(4) for j in range(4)) / 16
            row += bytes([0, 0, 0, round(cov * 255)])
        rows.append(bytes(row))
    def chunk(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)) \
        + chunk(b"IDAT", zlib.compress(b"".join(rows), 9)) + chunk(b"IEND", b"")
    open(path, "wb").write(data)

png(44, sys.argv[1])
