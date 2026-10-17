# Usage: python3 app/scripts/tray_icon.py app/src-tauri/icons/tray.png app/src-tauri/icons/tray-windows.png
# Draws the tray icons from Quill's logo (the same paths as `Logo` in
# app/src/components/Icon.tsx):
# - tray.png: the feather and pulse line alone, black on transparent, for a
#   macOS template icon (macOS tints it to suit the menu bar). 44 px: 22 pt @2x.
# - tray-windows.png: the full-colour logo, which stays visible on light and
#   dark taskbars. 32 px: 16 px @2x.
# No dependencies: paths are flattened to polygons and strokes to segments,
# then anti-aliased with 4x4 supersampling.
import math, re, struct, sys, zlib

FEATHER = ("M742 236c-170 40-310 170-370 340l-58 172 40 14 52-120c92-6 196-60 258-150-70 20-130 "
           "14-172 2 88-24 176-90 222-168-40 6-82 4-112-6 70-30 120-56 140-84z")
PULSE = "M250 800h60l30-50 40 100 40-150 40 120 30-20h284"
PULSE_WIDTH = 34
BACKGROUND = (64, 64, 960, 960, 190)  # x0, y0, x1, y1, corner radius
PINK = (0xFF, 0x10, 0x53)

def points(d):
    """The subset of SVG path data the logo uses (M, L, H, C, Z, absolute or
    relative), as a list of subpaths of points, curves flattened."""
    tokens = re.findall(r"[MmLlHhVvCcZz]|-?\d*\.?\d+", d)
    paths, cur, x, y, cmd, i = [], [], 0.0, 0.0, None, 0
    while i < len(tokens):
        if tokens[i].isalpha():
            cmd = tokens[i]; i += 1
            if cmd in "Zz":
                if cur: paths.append(cur); cur = []
                continue
        n = lambda k: [float(t) for t in tokens[i:i + k]]
        rel = cmd.islower()
        if cmd in "Mm":
            dx, dy = n(2); i += 2
            x, y = (x + dx, y + dy) if rel else (dx, dy)
            if cur: paths.append(cur)
            cur = [(x, y)]
            cmd = "l" if rel else "L"  # later pairs are lines
        elif cmd in "Ll":
            dx, dy = n(2); i += 2
            x, y = (x + dx, y + dy) if rel else (dx, dy)
            cur.append((x, y))
        elif cmd in "Hh":
            (dx,) = n(1); i += 1
            x = x + dx if rel else dx
            cur.append((x, y))
        elif cmd in "Vv":
            (dy,) = n(1); i += 1
            y = y + dy if rel else dy
            cur.append((x, y))
        elif cmd in "Cc":
            x1, y1, x2, y2, x3, y3 = n(6); i += 6
            if rel:
                x1, y1, x2, y2, x3, y3 = x + x1, y + y1, x + x2, y + y2, x + x3, y + y3
            for k in range(1, 25):
                t = k / 24; u = 1 - t
                cur.append((u**3 * x + 3 * u * u * t * x1 + 3 * u * t * t * x2 + t**3 * x3,
                            u**3 * y + 3 * u * u * t * y1 + 3 * u * t * t * y2 + t**3 * y3))
            x, y = x3, y3
    if cur: paths.append(cur)
    return paths

def inside(px, py, polygon):
    """Even-odd point-in-polygon."""
    hit = False
    for (x0, y0), (x1, y1) in zip(polygon, polygon[1:] + polygon[:1]):
        if (y0 > py) != (y1 > py) and px < x0 + (py - y0) * (x1 - x0) / (y1 - y0):
            hit = not hit
    return hit

def near(px, py, line, half):
    """Within `half` of the polyline: a stroke with round caps and joins."""
    for (x0, y0), (x1, y1) in zip(line, line[1:]):
        dx, dy = x1 - x0, y1 - y0
        if dx == dy == 0:
            continue
        t = max(0.0, min(1.0, ((px - x0) * dx + (py - y0) * dy) / (dx * dx + dy * dy)))
        if math.hypot(px - x0 - t * dx, py - y0 - t * dy) <= half:
            return True
    return False

def distance(px, py, polygon):
    """Distance to the polygon's outline."""
    best = math.inf
    for (x0, y0), (x1, y1) in zip(polygon, polygon[1:] + polygon[:1]):
        dx, dy = x1 - x0, y1 - y0
        if dx == dy == 0:
            continue
        t = max(0.0, min(1.0, ((px - x0) * dx + (py - y0) * dy) / (dx * dx + dy * dy)))
        best = min(best, math.hypot(px - x0 - t * dx, py - y0 - t * dy))
    return best

def in_rounded_rect(px, py, x0, y0, x1, y1, r):
    cx, cy = min(max(px, x0 + r), x1 - r), min(max(py, y0 + r), y1 - r)
    return x0 <= px <= x1 and y0 <= py <= y1 and math.hypot(px - cx, py - cy) <= r

FEATHER_POLY = points(FEATHER)[0]
PULSE_LINE = points(PULSE)[0]

def marks(x, y, stroke, gap=0):
    """The feather, and the pulse line kept `gap` clear of it so the two
    don't merge at small sizes."""
    if inside(x, y, FEATHER_POLY):
        return True
    return near(x, y, PULSE_LINE, stroke / 2) and (gap == 0 or distance(x, y, FEATHER_POLY) > gap)

def render(size, view, pixel):
    """`pixel(x, y)` gives RGBA coverage per sample in logo coordinates;
    `view` is the (x0, y0, side) square of logo coordinates to draw."""
    vx, vy, side = view
    rows = []
    for py in range(size):
        row = bytearray([0])
        for px in range(size):
            acc = [0, 0, 0, 0]
            for i in range(4):
                for j in range(4):
                    r, g, b, a = pixel(vx + (px + (i + .5) / 4) / size * side, vy + (py + (j + .5) / 4) / size * side)
                    acc = [acc[0] + r * a, acc[1] + g * a, acc[2] + b * a, acc[3] + a]
            a = acc[3] / 16
            row += bytes([round(acc[0] / acc[3]) if acc[3] else 0, round(acc[1] / acc[3]) if acc[3] else 0,
                          round(acc[2] / acc[3]) if acc[3] else 0, round(a * 255)])
        rows.append(bytes(row))
    return rows

def write_png(path, size, rows):
    def chunk(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)) \
        + chunk(b"IDAT", zlib.compress(b"".join(rows), 9)) + chunk(b"IEND", b"")
    open(path, "wb").write(data)

def macos(path):
    # Just the marks, a little bolder than in the logo so the pulse line
    # holds up at menu-bar size, framed to fill most of the 22 pt height.
    stroke = PULSE_WIDTH * 1.6
    xs = [p[0] for p in FEATHER_POLY + PULSE_LINE]
    ys = [p[1] for p in FEATHER_POLY + PULSE_LINE]
    x0, x1 = min(xs) - stroke / 2, max(xs) + stroke / 2
    y0, y1 = min(ys) - stroke / 2, max(ys) + stroke / 2
    side = max(x1 - x0, y1 - y0) / 0.86
    view = ((x0 + x1) / 2 - side / 2, (y0 + y1) / 2 - side / 2, side)
    gap = stroke * 0.6
    write_png(path, 44, render(44, view, lambda x, y: (0, 0, 0, 1) if marks(x, y, stroke, gap) else (0, 0, 0, 0)))

def windows(path):
    def pixel(x, y):
        if not in_rounded_rect(x, y, *BACKGROUND):
            return (0, 0, 0, 0)
        return (255, 255, 255, 1) if marks(x, y, PULSE_WIDTH * 1.8) else (*PINK, 1)
    write_png(path, 32, render(32, (64, 64, 896), pixel))

macos(sys.argv[1])
windows(sys.argv[2])
