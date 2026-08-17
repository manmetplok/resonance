//! Tiling ribbon span builder (design doc #170, surface #2 — todo #489).
//!
//! [`build_ribbon_spans`] is the pure layer between the #483 resolver and
//! the ribbon canvas: it turns the resolver's clipped pattern/fill spans
//! plus a coverage status into the four renderable span *types* (normal /
//! fill / gap / overflow), re-deriving each span's owning arrangement entry
//! so a click can select it. These cases drive it directly with stub
//! lookups — no `ComposeState`, no Canvas.

use resonance_app::compose::{resolve_arrangement, EntryLength, PatternEntry};
use resonance_app::view::{build_ribbon_spans, RibbonSpan, RibbonSpanKind};

const P_A: u64 = 1; // 2-bar pattern "Groove A"
const P_B: u64 = 2; // 1-bar pattern "Bass B"
const FILL: u64 = 9; // 1-bar fill "Fill B"

fn pattern_len(id: u64) -> u32 {
    match id {
        P_A => 2,
        _ => 1,
    }
}

fn pattern_name(id: u64) -> String {
    match id {
        P_A => "Groove A",
        P_B => "Bass B",
        FILL => "Fill B",
        _ => "?",
    }
    .to_string()
}

fn pattern_color(id: u64) -> [u8; 3] {
    match id {
        P_A => [10, 20, 30],
        P_B => [40, 50, 60],
        _ => [0, 0, 0],
    }
}

fn entry(pattern_id: u64, length: EntryLength, fill: Option<u64>) -> PatternEntry {
    PatternEntry {
        pattern_id,
        length,
        fill,
    }
}

fn build(entries: &[PatternEntry], section_bars: u32) -> Vec<RibbonSpan> {
    let resolved = resolve_arrangement(entries, section_bars, pattern_len);
    build_ribbon_spans(
        entries,
        section_bars,
        &resolved,
        pattern_len,
        pattern_name,
        pattern_color,
    )
}

#[test]
fn chained_with_fill_labels_and_attributes_spans() {
    // Groove A ×3 (a 2-bar pattern → 6 bars) with a fill on the last bar,
    // over a 6-bar section: normal [0,5) + fill [5,6), both entry 0.
    let entries = vec![entry(P_A, EntryLength::RepeatN(3), Some(FILL))];
    let spans = build(&entries, 6);

    assert_eq!(spans.len(), 2, "one normal span + one fill span");

    let normal = &spans[0];
    assert_eq!(normal.kind, RibbonSpanKind::Normal);
    assert_eq!((normal.bar_start, normal.bar_end), (0, 5));
    assert_eq!(normal.label, "Groove A \u{00d7}3");
    assert_eq!(normal.entry_index, Some(0));
    assert_eq!(normal.color, [10, 20, 30]);

    let fill = &spans[1];
    assert_eq!(fill.kind, RibbonSpanKind::Fill);
    assert_eq!((fill.bar_start, fill.bar_end), (5, 6));
    assert_eq!(fill.label, "FILL");
    // The fill bar still belongs to its entry so clicking it selects it.
    assert_eq!(fill.entry_index, Some(0));
}

#[test]
fn under_fill_appends_a_trailing_gap_span() {
    // Groove A ×1 = 2 bars in a 4-bar section → 2-bar tail gap.
    let entries = vec![entry(P_A, EntryLength::RepeatN(1), None)];
    let spans = build(&entries, 4);

    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0].kind, RibbonSpanKind::Normal);
    assert_eq!((spans[0].bar_start, spans[0].bar_end), (0, 2));

    let gap = &spans[1];
    assert_eq!(gap.kind, RibbonSpanKind::Gap);
    assert_eq!((gap.bar_start, gap.bar_end), (2, 4));
    assert_eq!(gap.label, "GAP");
    assert_eq!(gap.entry_index, None, "a gap belongs to no entry");
}

#[test]
fn over_fill_appends_an_overflow_cap_past_the_section_end() {
    // Groove A ×3 = 6 bars in a 4-bar section → 2 bars overflow. The normal
    // span is clipped to the boundary; the cap sits past it.
    let entries = vec![entry(P_A, EntryLength::RepeatN(3), None)];
    let spans = build(&entries, 4);

    assert_eq!(spans.len(), 2);
    let normal = &spans[0];
    assert_eq!(normal.kind, RibbonSpanKind::Normal);
    assert_eq!((normal.bar_start, normal.bar_end), (0, 4), "clipped to section");

    let cap = &spans[1];
    assert_eq!(cap.kind, RibbonSpanKind::Overflow);
    assert_eq!(cap.bar_start, 4, "cap begins at the section end");
    assert_eq!(cap.bar_end, 6, "cap extends past the end by the overflow");
    assert_eq!(cap.label, "OVERFLOW (clipped)");
    assert_eq!(cap.entry_index, None);
}

#[test]
fn fixed_bar_entries_attribute_to_the_right_entry() {
    // Two fixed-bar entries tile a 4-bar section exactly. Attribution must
    // map the second span back to entry 1 (not entry 0) so a click on the
    // right half selects the right entry.
    let entries = vec![
        entry(P_A, EntryLength::Bars(2), None),
        entry(P_B, EntryLength::Bars(2), None),
    ];
    let spans = build(&entries, 4);

    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0].entry_index, Some(0));
    assert_eq!(spans[0].label, "Groove A", "fixed-bar entries carry no ×N");
    assert_eq!(spans[1].entry_index, Some(1));
    assert_eq!(spans[1].label, "Bass B");
    assert_eq!((spans[1].bar_start, spans[1].bar_end), (2, 4));
}

#[test]
fn empty_arrangement_yields_a_single_full_width_gap() {
    let spans = build(&[], 8);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].kind, RibbonSpanKind::Gap);
    assert_eq!((spans[0].bar_start, spans[0].bar_end), (0, 8));
}
