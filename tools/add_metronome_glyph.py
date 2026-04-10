#!/usr/bin/env python3
"""
Extend the bundled Font Awesome Solid font with a custom metronome glyph.

The FA 7 Free tier does not include the metronome icon (it is a Pro-only
glyph), so the codepoint U+F8DB currently falls back to .notdef and shows as
an empty square in the app.  This script draws a clean silhouette metronome
and stores it in the font under codepoint U+F8DB so `theme::icon(fa::METRONOME)`
renders it.

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
GLYPH_NAME = "metronome"
GLYPH_UNICODE = 0xF8DB
GLYPH_ADVANCE_WIDTH = 512  # square — same as the other FA icons

# Renamed family so iced can uniquely resolve our bundled, extended font
# without clashing with any system-installed Font Awesome.
NEW_FAMILY = "Resonance Icons"
NEW_FULL_NAME = "Resonance Icons Solid"
NEW_POSTSCRIPT = "ResonanceIcons-Solid"


def draw_metronome(pen: T2CharStringPen) -> None:
    """
    Draw a metronome silhouette.

    Coordinate system: y-up, 512x512 em. All FA Solid icons are centered on
    y=192 (midpoint of ascent 448 / descent -64) and most span ~448 units
    vertically (e.g. play/stop go -32..416). We mirror that so the glyph
    renders at the same apparent size as neighbouring icons.

    Four filled sub-paths (all counter-clockwise so CFF nonzero winding
    fills them as one shape):
      1. Trapezoidal body
      2. Thin plate at the top of the body
      3. Pendulum mast on the plate
      4. Rectangular weight on the mast
    """
    # Vertical range: y = -32 .. 416  (span 448, center 192) — matches play/stop.
    # Horizontal range: x ≈ 48 .. 464 (span 416) — wider than the old version.

    # Body — trapezoidal silhouette. Wider base, narrower shoulders.
    pen.moveTo((48, -32))
    pen.lineTo((464, -32))
    pen.lineTo((340, 280))
    pen.lineTo((172, 280))
    pen.closePath()

    # Top plate of the body (where the pendulum attaches).
    pen.moveTo((160, 280))
    pen.lineTo((352, 280))
    pen.lineTo((342, 316))
    pen.lineTo((170, 316))
    pen.closePath()

    # Pendulum mast — thicker vertical bar centered on x=256.
    pen.moveTo((240, 316))
    pen.lineTo((272, 316))
    pen.lineTo((272, 416))
    pen.lineTo((240, 416))
    pen.closePath()

    # Weight — larger rectangle straddling the mast.
    pen.moveTo((200, 348))
    pen.lineTo((312, 348))
    pen.lineTo((312, 388))
    pen.lineTo((200, 388))
    pen.closePath()


def main() -> None:
    font = TTFont(FONT_PATH)
    cff = font["CFF "].cff
    top_dict = cff.topDictIndex[0]
    char_strings = top_dict.CharStrings

    # The Private dict and global subroutine index live on each existing
    # charstring; we borrow them from any glyph (here: 'circle').
    template = char_strings["circle"]
    private = template.private
    global_subrs = template.globalSubrs

    pen = T2CharStringPen(GLYPH_ADVANCE_WIDTH, glyphSet=None)
    draw_metronome(pen)
    charstring = pen.getCharString(private=private, globalSubrs=global_subrs)

    if GLYPH_NAME in char_strings:
        # Update in place via the existing name → index mapping.
        print(f"Glyph '{GLYPH_NAME}' already exists — replacing its outline.")
        char_strings[GLYPH_NAME] = charstring
    else:
        # Append a new charstring and register its name → index mapping.
        if char_strings.charStringsAreIndexed:
            idx = len(char_strings.charStringsIndex)
            char_strings.charStringsIndex.append(charstring)
            char_strings.charStrings[GLYPH_NAME] = idx
        else:
            char_strings.charStrings[GLYPH_NAME] = charstring

    # Make sure the font's glyph order includes our new name.
    glyph_order = font.getGlyphOrder()
    if GLYPH_NAME not in glyph_order:
        font.setGlyphOrder(list(glyph_order) + [GLYPH_NAME])

    # Add hmtx entry (advance width, left side bearing).
    font["hmtx"].metrics[GLYPH_NAME] = (GLYPH_ADVANCE_WIDTH, 78)

    # Wire up the cmap so U+F8DB → metronome in every unicode subtable.
    for table in font["cmap"].tables:
        if table.isUnicode():
            table.cmap[GLYPH_UNICODE] = GLYPH_NAME

    # Register the glyph in the CFF charset as well (belt-and-braces).
    if GLYPH_NAME not in top_dict.charset:
        top_dict.charset = list(top_dict.charset) + [GLYPH_NAME]

    # Rename the font family so our modified copy can be unambiguously
    # resolved by iced even when a vanilla Font Awesome is installed
    # system-wide with the same original family name.
    rename_font(font)

    font.save(FONT_PATH)
    print(f"Wrote {FONT_PATH}")


def rename_font(font: TTFont) -> None:
    """Rewrite the name table and CFF top dict so the font identifies
    itself as `Resonance Icons` rather than `Font Awesome 7 Free`."""
    name_table = font["name"]

    # nameID → new value. Covers the typical ID set used by font loaders.
    replacements = {
        1: NEW_FAMILY,            # Family
        3: NEW_POSTSCRIPT,        # Unique ID
        4: NEW_FULL_NAME,         # Full name
        6: NEW_POSTSCRIPT,        # PostScript name
        16: NEW_FAMILY,           # Preferred family
        21: NEW_FAMILY,           # WWS family
    }

    for record in list(name_table.names):
        if record.nameID in replacements:
            record.string = replacements[record.nameID].encode(
                "utf-16-be" if record.platformID == 3 else "ascii",
                errors="replace",
            )

    # CFF top dict also carries the PostScript name.
    cff = font["CFF "]
    cff.cff.fontNames = [NEW_POSTSCRIPT]
    top_dict = cff.cff.topDictIndex[0]
    if hasattr(top_dict, "FullName"):
        top_dict.FullName = NEW_FULL_NAME
    if hasattr(top_dict, "FamilyName"):
        top_dict.FamilyName = NEW_FAMILY


if __name__ == "__main__":
    main()
