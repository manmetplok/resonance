//! The tiling ribbon renders through a fingerprinted cache (review
//! VIEW-32): the fingerprint must hold for identical inputs (so the 16 ms
//! tick reuses the geometry) and move on anything it paints.

use resonance_app::view::compose::drumroll::ribbon::{RibbonCanvas, RibbonSpan, RibbonSpanKind};

fn ribbon() -> RibbonCanvas {
    RibbonCanvas {
        spans: vec![RibbonSpan {
            bar_start: 0,
            bar_end: 4,
            kind: RibbonSpanKind::Normal,
            color: [10, 20, 30],
            label: "Groove ×4".into(),
            entry_index: Some(0),
        }],
        length_bars: 8,
        selected_entry_index: None,
        section_name: "Verse".into(),
        content_width: 800.0,
    }
}

#[test]
fn ribbon_fingerprint_stable_for_identical_inputs() {
    assert_eq!(ribbon().fingerprint(), ribbon().fingerprint());
}

#[test]
fn ribbon_fingerprint_moves_on_painted_changes() {
    let base = ribbon().fingerprint();
    let mut r = ribbon();
    r.selected_entry_index = Some(0);
    assert_ne!(base, r.fingerprint(), "selection highlight");
    let mut r = ribbon();
    r.spans[0].bar_end = 3;
    assert_ne!(base, r.fingerprint(), "span extent");
    let mut r = ribbon();
    r.section_name = "Chorus".into();
    assert_ne!(base, r.fingerprint(), "side tag");
    let mut r = ribbon();
    r.content_width = 900.0;
    assert_ne!(base, r.fingerprint(), "bar grid width");
}
