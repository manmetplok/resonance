//! Melisma hold editing and origins (spec
//! `vocal-mcp-control-and-quality.md` §6): in-word `·_·` holds, the
//! draft rewrite behind the vocal roll's `s` key, and telling a draft
//! `_` hold from a per-note generator hold.

use resonance_music_theory::g2p::{
    assign_syllables_to_notes, hold_origins, line_first_syllables, note_unit_count, resolve_draft,
    set_holds_after, strip_syllable_marks, HoldOrigin,
};
use resonance_music_theory::{count_syllables, LyricLine};

fn line(text: &str) -> LyricLine {
    LyricLine {
        n: 1,
        rhyme: 'A',
        syllables: 0,
        text: text.into(),
        locked: false,
    }
}

fn annotations(marks: &[&str]) -> Vec<String> {
    marks.iter().map(|m| (*m).to_owned()).collect()
}

#[test]
fn in_word_hold_pieces_hold_that_syllable() {
    let draft = [line("a·_·ma·zing grace")];
    let syllables = resolve_draft(&draft);
    let labels: Vec<&str> = syllables.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, ["a", "ma", "zing", "grace"]);
    assert_eq!(syllables[0].slurs_after, 1);
    assert_eq!(note_unit_count(&syllables), 5);
    assert_eq!(count_syllables(&draft[0].text), 5, "generator count agrees");
    // The hold is a slur note between `a` and `ma`.
    let assigned = assign_syllables_to_notes(&syllables, &[], 5);
    let slurs: Vec<bool> = assigned.iter().map(|a| a.is_slur).collect();
    assert_eq!(slurs, [false, true, false, false, false]);
    assert_eq!(assigned[2].label, "ma");
}

#[test]
fn trailing_in_word_hold_equals_a_standalone_one() {
    let a = resolve_draft(&[line("love·_ me")]);
    let b = resolve_draft(&[line("love _ me")]);
    assert_eq!(a, b);
}

#[test]
fn set_holds_after_writes_and_removes_standalone_holds() {
    let mut draft = vec![line("love me tender")];
    assert!(set_holds_after(&mut draft, 0, 2));
    assert_eq!(draft[0].text, "love _ _ me tender");
    assert_eq!(draft[0].syllables, 5, "the line's count follows the text");
    assert!(set_holds_after(&mut draft, 0, 1));
    assert_eq!(draft[0].text, "love _ me tender");
    assert!(set_holds_after(&mut draft, 0, 0));
    assert_eq!(draft[0].text, "love me tender");
    // The last syllable of a line, and the second line.
    let mut draft = vec![line("hold on"), line("to me")];
    assert!(set_holds_after(&mut draft, 1, 1));
    assert_eq!(draft[0].text, "hold on _");
    assert!(set_holds_after(&mut draft, 3, 1));
    assert_eq!(draft[1].text, "to me _");
}

#[test]
fn set_holds_after_inside_a_word_uses_hold_pieces() {
    let mut draft = vec![line("for·get me")];
    assert!(set_holds_after(&mut draft, 0, 1));
    assert_eq!(draft[0].text, "for·_·get me");
    let syllables = resolve_draft(&draft);
    assert_eq!(syllables[0].slurs_after, 1);
    assert!(set_holds_after(&mut draft, 0, 0));
    assert_eq!(draft[0].text, "for·get me");
    // Hyphen breaks are normalised on the way.
    let mut draft = vec![line("a-ma-zing")];
    assert!(set_holds_after(&mut draft, 1, 1));
    assert_eq!(draft[0].text, "a·ma·_·zing");
}

#[test]
fn set_holds_after_refuses_what_it_cannot_write() {
    let mut draft = vec![line("[l ih · l iy] la")];
    let before = draft.clone();
    assert!(!set_holds_after(&mut draft, 0, 1), "mid-block");
    assert!(!set_holds_after(&mut draft, 9, 1), "no such syllable");
    assert_eq!(draft, before);
    // A block's last group takes a standalone hold.
    assert!(set_holds_after(&mut draft, 1, 1));
    assert_eq!(draft[0].text, "[l ih · l iy] _ la");
}

#[test]
fn hold_origins_tell_draft_holds_from_marked_ones() {
    let syllables = resolve_draft(&[line("love _ me")]);
    // Note 1 is the draft hold, note 3 is a generator `+` hold of `me`.
    let marks = annotations(&["", "", "", "+"]);
    let origins = hold_origins(&syllables, &marks, 4);
    assert_eq!(
        origins,
        [None, Some(HoldOrigin::Draft), None, Some(HoldOrigin::Annotation)]
    );
    // A marker on a note the draft already holds is that same hold.
    let marks = annotations(&["", "+", ""]);
    assert_eq!(
        hold_origins(&syllables, &marks, 3),
        [None, Some(HoldOrigin::Draft), None]
    );
}

#[test]
fn line_first_syllables_index_each_lines_start() {
    let draft = [line("hel·lo there"), line("&"), line("love _ me")];
    assert_eq!(line_first_syllables(&draft), [0, 3]);
}

#[test]
fn strip_syllable_marks_keeps_breaks_around_holds() {
    assert_eq!(strip_syllable_marks("a·_·ma·zing grace"), "a·_·mazing grace");
    assert_eq!(strip_syllable_marks("hel·lo there _"), "hello there _");
}
