"""Generates assets/icon.ico (16-256 px) and assets/icon_64.rgba for the window icon.

Pure Python, no dependencies:  python assets/make_icon.py
"""
import math
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
YELLOW_TOP = (255, 220, 40)
YELLOW_BOTTOM = (255, 196, 0)
INK = (20, 20, 22)
TIP = (88, 88, 92)


def rounded_rect_cover(x, y, inset, radius):
    """Coverage test for a rounded square spanning [inset, 1-inset]."""
    lo, hi = inset + radius, 1 - inset - radius
    cx = min(max(x, lo), hi)
    cy = min(max(y, lo), hi)
    return (x - cx) ** 2 + (y - cy) ** 2 <= radius ** 2 and inset <= x <= 1 - inset and inset <= y <= 1 - inset


def capsule(x, y, ax, ay, bx, by, r):
    """Point-in-capsule: distance to segment AB <= r."""
    dx, dy = bx - ax, by - ay
    t = max(0.0, min(1.0, ((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)))
    px, py = ax + t * dx, ay + t * dy
    return (x - px) ** 2 + (y - py) ** 2 <= r * r


def sample(x, y):
    """RGBA at normalized coordinates (y down)."""
    if not rounded_rect_cover(x, y, 0.04, 0.22):
        return None
    # stem earbud seen from the side: silicone tip on the left, body, stem, sound waves on the right
    hx, hy = 0.42, 0.40
    body = (x - hx) ** 2 + (y - hy) ** 2 <= 0.165 ** 2
    stem = capsule(x, y, 0.47, 0.44, 0.50, 0.80, 0.07)
    tip = ((x - 0.25) / 0.085) ** 2 + ((y - 0.38) / 0.115) ** 2 <= 1.0
    d = math.hypot(x - hx, y - hy)
    ang = math.degrees(math.atan2(y - hy, x - hx))
    wave = -48 <= ang <= 30 and (abs(d - 0.27) <= 0.022 or abs(d - 0.355) <= 0.022)
    if body or stem:
        return INK
    if tip:
        return TIP
    if wave:
        return INK
    t = y
    return tuple(round(a + (b - a) * t) for a, b in zip(YELLOW_TOP, YELLOW_BOTTOM))


def render(size, ss=4):
    px = bytearray()
    for j in range(size):
        for i in range(size):
            r = g = b = a = 0
            for sj in range(ss):
                for si in range(ss):
                    c = sample((i + (si + 0.5) / ss) / size, (j + (sj + 0.5) / ss) / size)
                    if c:
                        r += c[0]
                        g += c[1]
                        b += c[2]
                        a += 1
            n = ss * ss
            if a:
                px += bytes((r // a, g // a, b // a, round(255 * a / n)))
            else:
                px += bytes(4)
    return bytes(px)


def png(size, rgba):
    raw = b"".join(b"\x00" + rgba[y * size * 4:(y + 1) * size * 4] for y in range(size))

    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def main():
    sizes = [16, 24, 32, 48, 64, 128, 256]
    images = {s: render(s, 8 if s <= 32 else 4) for s in sizes}
    pngs = [png(s, images[s]) for s in sizes]
    header = struct.pack("<HHH", 0, 1, len(sizes))
    offset = 6 + 16 * len(sizes)
    entries = b""
    for s, data in zip(sizes, pngs):
        entries += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    with open(os.path.join(HERE, "icon.ico"), "wb") as f:
        f.write(header + entries + b"".join(pngs))
    with open(os.path.join(HERE, "icon_64.rgba"), "wb") as f:
        f.write(images[64])
    with open(os.path.join(HERE, "icon_256.png"), "wb") as f:
        f.write(pngs[-1])
    print("wrote icon.ico, icon_64.rgba, icon_256.png")


if __name__ == "__main__":
    main()
