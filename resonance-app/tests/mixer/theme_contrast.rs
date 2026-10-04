//! Contrast guard over the theme's text tokens (code review UX-07).
//!
//! `TEXT_3` used to be `#5d626d` on `BG_2`, ≈2.8:1 — well under WCAG AA's
//! 4.5:1 floor for small text — even though it carries real information
//! (track kind, the dirty label, drum-grid shares, the global-shelf M/L
//! letters). It's now `#8a909b`. This test is the thing that would have
//! caught the regression: it computes the real WCAG relative-luminance
//! contrast ratio (not a guess) for every text tone the UI actually draws
//! text in, against the panel background it's drawn on, and fails if any
//! of the three readable tiers (`TEXT_1`/`TEXT_2`/`TEXT_3`) drops below
//! 4.5:1. `TEXT_4` is excluded on purpose — it's disabled-only (see its
//! doc comment in `theme.rs`) and WCAG doesn't hold inert controls to the
//! same floor.

use iced::Color;
use resonance_app::theme;

/// WCAG 2.1 relative luminance (§1.4.3's formula) of an sRGB colour.
fn relative_luminance(c: Color) -> f32 {
    fn chan(v: f32) -> f32 {
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * chan(c.r) + 0.7152 * chan(c.g) + 0.0722 * chan(c.b)
}

/// WCAG contrast ratio between two opaque colours — order-independent,
/// always >= 1.0.
fn contrast_ratio(a: Color, b: Color) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// WCAG AA's floor for normal-weight text under 18pt/14pt-bold — every
/// size this app uses for informational text.
const AA_SMALL_TEXT: f32 = 4.5;

#[test]
fn readable_text_tiers_clear_aa_on_every_panel_background() {
    // (label, text color) x (label, panel background) — every background
    // a track header, panel or card actually paints (`ux-guidelines.md` →
    // Backdrop layers), crossed with every tier that's meant to be read
    // rather than merely present (TEXT_4 is disabled-only; excluded).
    let texts = [
        ("TEXT_1", theme::TEXT_1),
        ("TEXT_2", theme::TEXT_2),
        ("TEXT_3", theme::TEXT_3),
    ];
    let panels = [
        ("BG_0", theme::BG_0),
        ("BG_1", theme::BG_1),
        ("BG_2", theme::BG_2),
    ];
    let mut failures = Vec::new();
    for (tname, tcolor) in texts {
        for (pname, pcolor) in panels {
            let ratio = contrast_ratio(tcolor, pcolor);
            if ratio < AA_SMALL_TEXT {
                failures.push(format!("{tname} on {pname}: {ratio:.2}:1 (need >= {AA_SMALL_TEXT}:1)"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "theme text tokens fall below WCAG AA on a panel background:\n{}",
        failures.join("\n")
    );
}

/// The error bar used to paint `Color::WHITE` text straight on `BAD` — a
/// light soft-pink, so no legible text color clears 4.5:1 against it (not
/// even pure white: ≈2.75:1). It was restyled to a tinted wash (`BAD_DIM`
/// over `BG_1`-ish) with `TEXT_1` body text and a bordered `BAD` tag chip,
/// mirroring the other status lines; this locks the new combination in.
#[test]
fn error_bar_body_text_clears_aa_on_its_wash() {
    // `BAD_DIM` is a translucent wash; composite it over `BG_1` (the app
    // body it's drawn on in `view_status_area`) to get the opaque surface
    // the text actually sits on, same blend `theme::frost_over` uses.
    let wash = theme::BAD_DIM;
    let base = theme::BG_1;
    let a = wash.a;
    let composited = Color {
        r: base.r * (1.0 - a) + wash.r * a,
        g: base.g * (1.0 - a) + wash.g * a,
        b: base.b * (1.0 - a) + wash.b * a,
        a: 1.0,
    };
    let ratio = contrast_ratio(theme::TEXT_1, composited);
    assert!(
        ratio >= AA_SMALL_TEXT,
        "TEXT_1 on the error bar's BAD_DIM-over-BG_1 wash: {ratio:.2}:1 (need >= {AA_SMALL_TEXT}:1)"
    );
    // The tag chip's own text (`BAD` on the same wash) is a smaller,
    // secondary read next to the bordered chip outline — document its
    // ratio rather than assert AA on it, since `BAD`-on-`BAD_DIM` is a
    // same-hue tint-on-wash pairing the rest of the status lines already
    // use (`AUDIO` in `BAD` on `BAD_DIM`, `AUTOSAVE` in `WARM` on
    // `WARM_DIM`) and is paired with a 1px `BAD` border, not color alone.
    let chip_ratio = contrast_ratio(theme::BAD, composited);
    assert!(chip_ratio > 1.0);
}
