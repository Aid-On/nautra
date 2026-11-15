# SPDX-License-Identifier: Apache-2.0
"""Regenerate the Nautra brand images from source/nautra-symbol-source.png.

    python3 docs/assets/brand/build.py        # needs Pillow

Outputs, next to this file:
    nautra-symbol.png / -dark.png               transparent symbol
    nautra-logo-horizontal.png / -dark.png      symbol + wordmark, transparent
    nautra-cover.png / -dark.png                README header, 1440x606
    nautra-ogp.png                              social preview, 1280x640
"""
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont

HERE = Path(__file__).resolve().parent
SOURCE = HERE / "source" / "nautra-symbol-source.png"
FONT = HERE / "font" / "Outfit-wght.ttf"

WHITE = (255, 255, 255)
DEEP = (60, 26, 167)      # shell shadow, hood, pupil
VIOLET = (100, 71, 240)   # body
ORCHID = (198, 121, 252)  # shell highlight
PALETTE = (DEEP, VIOLET, ORCHID)

INK = (30, 22, 56)        # wordmark on light
PAPER = (245, 243, 252)   # wordmark and DEEP replacement on dark

# The eye's white and its highlight are part of the drawing, not background.
EYE_SEEDS = ((760, 800), (815, 806))
PUPIL = (803, 826, 62)    # centre x, y, radius: stays DEEP in the dark variant


def _sub(a, b):
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def _dot(a, b):
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


AXES = [(_sub(c, WHITE), _dot(_sub(c, WHITE), _sub(c, WHITE)), c) for c in PALETTE]
DEEP_TO_VIOLET = _dot(_sub(VIOLET, DEEP), _sub(VIOLET, DEEP)) ** 0.5


def cut_out(src, dark):
    """White background -> alpha. Returns an RGBA image of the symbol alone."""
    w, h = src.size
    r, g, b = src.split()
    lightest = ImageChops.darker(ImageChops.darker(r, g), b)
    whites = lightest.point(lambda v: 255 if v > 215 else 0)
    for seed in EYE_SEEDS:
        ImageDraw.floodfill(whites, seed, 128)
    keep = whites.point(lambda v: 255 if v == 128 else 0).filter(ImageFilter.MaxFilter(9))

    px = src.load()
    kp = keep.load()
    out = Image.new("RGBA", (w, h))
    op = out.load()
    cx, cy, cr = PUPIL
    for y in range(h):
        for x in range(w):
            p = px[x, y]
            if kp[x, y]:
                op[x, y] = (*p, 255)
                continue
            if p[0] > 250 and p[1] > 250 and p[2] > 250:
                continue
            # How far along white -> palette colour this pixel sits is its
            # coverage; pick the palette colour whose line it lies closest to.
            d = _sub(p, WHITE)
            best = None
            for axis, norm, colour in AXES:
                t = _dot(d, axis) / norm
                tc = min(max(t, 0.0), 1.0)
                off = _sub(d, (axis[0] * tc, axis[1] * tc, axis[2] * tc))
                err = _dot(off, off)
                if best is None or err < best[0]:
                    best = (err, tc, colour)
            _, t, colour = best
            rgb = colour if t < 0.97 else p
            if dark and (x - cx) ** 2 + (y - cy) ** 2 > cr * cr:
                dd = _sub(rgb, DEEP)
                wgt = max(0.0, 1.0 - (_dot(dd, dd) ** 0.5) / DEEP_TO_VIOLET)
                if wgt:
                    rgb = tuple(
                        min(255, round(rgb[i] + (PAPER[i] - DEEP[i]) * wgt)) for i in range(3)
                    )
            op[x, y] = (*rgb, round(255 * t))
    return out.crop(out.getbbox())


def squared(im, pad=0.06):
    side = round(max(im.size) * (1 + 2 * pad))
    sq = Image.new("RGBA", (side, side))
    sq.alpha_composite(im, ((side - im.width) // 2, (side - im.height) // 2))
    return sq


def wordmark(height, colour):
    """'Nautra' set in Outfit, cap height = `height` px, tightly cropped."""
    font = ImageFont.truetype(str(FONT), 100)
    font.set_variation_by_axes([700])
    cap = font.getbbox("N")
    size = round(100 * height / (cap[3] - cap[1]))
    font = ImageFont.truetype(str(FONT), size)
    font.set_variation_by_axes([700])
    box = font.getbbox("Nautra")
    im = Image.new("RGBA", (box[2] - box[0] + 8, box[3] - box[1] + 8))
    ImageDraw.Draw(im).text((4 - box[0], 4 - box[1]), "Nautra", font=font, fill=colour)
    return im


def lockup(symbol, dark, sym_h):
    """Symbol and wordmark side by side on a transparent canvas."""
    sym = symbol.resize((round(symbol.width * sym_h / symbol.height), sym_h), Image.LANCZOS)
    word = wordmark(round(sym_h * 0.46), PAPER if dark else INK)
    gap = round(sym_h * 0.2)
    im = Image.new("RGBA", (sym.width + gap + word.width, sym_h))
    im.alpha_composite(sym, (0, 0))
    im.alpha_composite(word, (sym.width + gap, (sym_h - word.height) // 2))
    return im


def backdrop(size, dark):
    """Flat ground with three slanted bands and a hairline frame."""
    w, h = size
    if dark:
        base, left, left2, right, frame = (
            (15, 11, 31), (27, 19, 58), (34, 24, 74), (26, 16, 48), (46, 36, 84))
    else:
        base, left, left2, right, frame = (
            (249, 248, 254), (237, 233, 254), (228, 222, 253), (245, 235, 254), (226, 221, 244))
    im = Image.new("RGB", size, base)
    d = ImageDraw.Draw(im)
    lean = h * 0.46
    d.polygon([(0, 0), (w * 0.33, 0), (w * 0.33 + lean, h), (0, h)], fill=left)
    d.polygon([(w * 0.02, 0), (w * 0.085, 0), (w * 0.085 + lean, h), (w * 0.02 + lean, h)], fill=left2)
    d.polygon([(w * 0.68, 0), (w, 0), (w, h), (w * 0.68 + lean, h)], fill=right)
    m = round(h * 0.04)
    d.rectangle([m, m, w - m - 1, h - m - 1], outline=frame, width=max(1, h // 300))
    return im


def card(symbol, size, dark, tagline=None, fill=0.8):
    """`fill` is the share of the card's width the logo occupies."""
    scale = 2
    w, h = size[0] * scale, size[1] * scale
    im = backdrop((w, h), dark).convert("RGBA")
    probe = lockup(symbol, dark, 400)
    logo = lockup(symbol, dark, round(400 * w * fill / probe.width))
    y = (h - logo.height) // 2 - (round(h * 0.05) if tagline else 0)
    im.alpha_composite(logo, ((w - logo.width) // 2, y))
    if tagline:
        font = ImageFont.truetype(str(FONT), round(h * 0.052))
        font.set_variation_by_axes([400])
        colour = (196, 190, 224) if dark else (88, 78, 128)
        ImageDraw.Draw(im).text(
            (w // 2, y + logo.height + round(h * 0.075)), tagline, font=font, fill=colour, anchor="mm")
    return im.convert("RGB").resize(size, Image.LANCZOS)


def main():
    src = Image.open(SOURCE).convert("RGB")
    for dark in (False, True):
        suffix = "-dark" if dark else ""
        symbol = cut_out(src, dark)
        squared(symbol).resize((1024, 1024), Image.LANCZOS).save(
            HERE / f"nautra-symbol{suffix}.png", optimize=True)
        lockup(symbol, dark, 512).save(HERE / f"nautra-logo-horizontal{suffix}.png", optimize=True)
        card(symbol, (1440, 606), dark).save(HERE / f"nautra-cover{suffix}.png", optimize=True)
        if not dark:
            card(symbol, (1280, 640), False,
                 "Build, ship and run WebAssembly workloads", fill=0.78).save(
                HERE / "nautra-ogp.png", optimize=True)


if __name__ == "__main__":
    main()
