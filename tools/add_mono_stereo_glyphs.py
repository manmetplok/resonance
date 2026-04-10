#!/usr/bin/env python3
"""
Extend the bundled Font Awesome Solid font with two custom glyphs for the
track-header mono/stereo toggle:

  * `circle-hollow`         at U+F8DC — a single hollow circle (mono)
  * `circle-hollow-double`  at U+F8DD — two overlapping hollow circles (stereo)

FA Free Solid doesn't ship any hollow-circle glyph and we don't want to
bundle a second font. Both codepoints live well outside FA Free's range so
they don't clash with anything FA already defines.

The font is modified in place at
`resonance-app/assets/fonts/fa-solid-900.otf`.
"""
from pathlib import Path

from fontTools.ttLib import TTFont
from fontTools.pens.t2CharStringPen import T2CharStringPen

FONT_PATH = (
    Path(__file__).resolve().parents[1]
    / "resonance-app/assets/fonts/fa-solid-900.otf"
)

# Glyph em square matches the other FA icons (512x512 with y-center = 192).
GLYPH_ADVANCE_WIDTH = 512

# Magic constant to approximate a quarter circle with a cubic bezier.
KAPPA = 0.552284749831


def draw_circle(
    pen: T2CharStringPen,
    cx: float,
    cy: float,
    r: float,
    counter_clockwise: bool,
) -> None:
    """Draw a full circle as four cubic Beziers.

    Use `counter_clockwise=True` for an outer contour (fills inside) and
    `counter_clockwise=False` for a hole under CFF nonzero winding.
    """
    c = r * KAPPA
    if counter_clockwise:
        pen.moveTo((cx + r, cy))
        pen.curveTo((cx + r, cy + c), (cx + c, cy + r), (cx, cy + r))
        pen.curveTo((cx - c, cy + r), (cx - r, cy + c), (cx - r, cy))
        pen.curveTo((cx - r, cy - c), (cx - c, cy - r), (cx, cy - r))
        pen.curveTo((cx + c, cy - r), (cx + r, cy - c), (cx + r, cy))
    else:
        pen.moveTo((cx + r, cy))
        pen.curveTo((cx + r, cy - c), (cx + c, cy - r), (cx, cy - r))
        pen.curveTo((cx - c, cy - r), (cx - r, cy - c), (cx - r, cy))
        pen.curveTo((cx - r, cy + c), (cx - c, cy + r), (cx, cy + r))
        pen.curveTo((cx + c, cy + r), (cx + r, cy + c), (cx + r, cy))
    pen.closePath()


def draw_ring(
    pen: T2CharStringPen,
    cx: float,
    cy: float,
    outer_r: float,
    inner_r: float,
) -> None:
    """Draw a ring (a donut) by combining an outer CCW circle with an
    inner CW circle. Nonzero winding carves the hole.
    """
    draw_circle(pen, cx, cy, outer_r, counter_clockwise=True)
    draw_circle(pen, cx, cy, inner_r, counter_clockwise=False)


def draw_circle_hollow(pen: T2CharStringPen) -> None:
    """One centered ring — used as the 'mono' glyph."""
    draw_ring(pen, cx=256, cy=192, outer_r=180, inner_r=132)


def draw_circle_hollow_double(pen: T2CharStringPen) -> None:
    """Two overlapping rings — the classic stereo symbol.

    Circle centers are separated by 160 units (=2*80); each ring has an
    outer radius of 140, so the two circles overlap by 120 units in the
    middle region. Nonzero winding handles the overlap correctly: the
    ring boundaries remain hollow even where they cross.
    """
    outer_r = 140
    inner_r = 96
    left_cx = 256 - 80
    right_cx = 256 + 80
    cy = 192
    draw_ring(pen, cx=left_cx, cy=cy, outer_r=outer_r, inner_r=inner_r)
    draw_ring(pen, cx=right_cx, cy=cy, outer_r=outer_r, inner_r=inner_r)


GLYPHS = [
    # (glyph_name, unicode, drawer, left_side_bearing)
    ("circle-hollow", 0xF8DC, draw_circle_hollow, 76),
    ("circle-hollow-double", 0xF8DD, draw_circle_hollow_double, 36),
]


def install_glyph(font: TTFont, name: str, codepoint: int, drawer, lsb: int) -> None:
    cff = font["CFF "].cff
    top_dict = cff.topDictIndex[0]
    char_strings = top_dict.CharStrings

    # Borrow private dict + global subrs from any existing glyph.
    template = char_strings["circle"]
    private = template.private
    global_subrs = template.globalSubrs

    pen = T2CharStringPen(GLYPH_ADVANCE_WIDTH, glyphSet=None)
    drawer(pen)
    charstring = pen.getCharString(private=private, globalSubrs=global_subrs)

    if name in char_strings:
        print(f"Glyph '{name}' already exists — replacing its outline.")
        char_strings[name] = charstring
    else:
        if char_strings.charStringsAreIndexed:
            idx = len(char_strings.charStringsIndex)
            char_strings.charStringsIndex.append(charstring)
            char_strings.charStrings[name] = idx
        else:
            char_strings.charStrings[name] = charstring

    glyph_order = font.getGlyphOrder()
    if name not in glyph_order:
        font.setGlyphOrder(list(glyph_order) + [name])

    font["hmtx"].metrics[name] = (GLYPH_ADVANCE_WIDTH, lsb)

    for table in font["cmap"].tables:
        if table.isUnicode():
            table.cmap[codepoint] = name

    if name not in top_dict.charset:
        top_dict.charset = list(top_dict.charset) + [name]


def main() -> None:
    font = TTFont(FONT_PATH)
    for name, codepoint, drawer, lsb in GLYPHS:
        install_glyph(font, name, codepoint, drawer, lsb)
        print(f"Installed glyph '{name}' at U+{codepoint:04X}")
    font.save(FONT_PATH)
    print(f"Wrote {FONT_PATH}")


if __name__ == "__main__":
    main()
