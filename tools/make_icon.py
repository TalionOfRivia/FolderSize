"""Draws the FolderSize icon: a donut chart on a dark rounded tile.

Writes assets/icon.png (256 px, used for the window) and assets/icon.ico
(16-256 px, embedded in the exe). Run: python tools/make_icon.py
"""

import math
from pathlib import Path

from PIL import Image, ImageDraw

S = 1024  # drawing size; downscaled for each icon size
OUT = Path(__file__).resolve().parent.parent / "assets"

# (share, color) - same hues as the app's charts, biggest first.
SLICES = [
    (0.46, (0x39, 0x87, 0xE5)),
    (0.24, (0xE8, 0x6A, 0x33)),
    (0.17, (0x1B, 0xB0, 0x7D)),
    (0.13, (0xE5, 0xA4, 0x12)),
]


def tile() -> Image.Image:
    """Rounded square with a soft vertical gradient."""
    grad = Image.new("RGBA", (S, S))
    top, bottom = (0x2C, 0x37, 0x48), (0x12, 0x17, 0x20)
    px = grad.load()
    for y in range(S):
        t = y / (S - 1)
        c = tuple(round(a + (b - a) * t) for a, b in zip(top, bottom)) + (255,)
        for x in range(S):
            px[x, y] = c
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle((36, 36, S - 36, S - 36), radius=210, fill=255)
    out = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    out.paste(grad, (0, 0), mask)
    return out


def donut() -> Image.Image:
    layer = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    cx = cy = S / 2
    r_out, r_in, gap_deg = 330, 168, 3.0
    start = -90.0
    for i, (share, color) in enumerate(SLICES):
        sweep = share * 360
        a0, a1 = start + gap_deg / 2, start + sweep - gap_deg / 2
        # Pull the biggest slice out a little so the shape reads at small sizes.
        off = 26 if i == 0 else 0
        mid = math.radians((a0 + a1) / 2)
        ox, oy = math.cos(mid) * off, math.sin(mid) * off
        box = (cx - r_out + ox, cy - r_out + oy, cx + r_out + ox, cy + r_out + oy)
        slice_layer = Image.new("RGBA", (S, S), (0, 0, 0, 0))
        sd = ImageDraw.Draw(slice_layer)
        sd.pieslice(box, a0, a1, fill=color + (255,))
        hole = (cx - r_in + ox, cy - r_in + oy, cx + r_in + ox, cy + r_in + oy)
        sd.ellipse(hole, fill=(0, 0, 0, 0))
        layer = Image.alpha_composite(layer, slice_layer)
        start += sweep
    return layer


def main() -> None:
    OUT.mkdir(exist_ok=True)
    img = Image.alpha_composite(tile(), donut())
    img.resize((256, 256), Image.LANCZOS).save(OUT / "icon.png")
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    img.resize((256, 256), Image.LANCZOS).save(OUT / "icon.ico", sizes=[(s, s) for s in sizes])
    print("wrote", OUT / "icon.png", "and", OUT / "icon.ico")


if __name__ == "__main__":
    main()
