use resonance_common::drum_map::{self as gm, GM_PADS, NUM_PADS};

#[test]
fn gm_standard_note_numbers_are_pinned() {
    assert_eq!(gm::KICK, 36);
    assert_eq!(gm::SNARE, 38);
    assert_eq!(gm::RIMSHOT, 37);
    assert_eq!(gm::HIHAT_CLOSED, 42);
    assert_eq!(gm::HIHAT_OPEN, 46);
    assert_eq!(gm::HIHAT_PEDAL, 44);
    assert_eq!(gm::TOM_LOW, 45);
    assert_eq!(gm::TOM_MID, 47);
    assert_eq!(gm::TOM_HIGH, 50);
    assert_eq!(gm::CRASH_16_EDGE, 49);
    assert_eq!(gm::CRASH_18_EDGE, 57);
    assert_eq!(gm::RIDE_EDGE, 51);
    assert_eq!(gm::RIDE_BELL, 53);
    assert_eq!(gm::CHINA_EDGE, 52);
    assert_eq!(gm::COWBELL, 56);
}

#[test]
fn extended_note_numbers_are_pinned() {
    assert_eq!(gm::SNARE_SIDESTICK, 39);
    assert_eq!(gm::SNARE_FLAM, 21);
    assert_eq!(gm::SNARE_ROLL, 22);
    assert_eq!(gm::SNARE_HANDTUCH, 23);
    assert_eq!(gm::HIHAT_HALF_OPEN, 24);
    assert_eq!(gm::HIHAT_LOOSE, 25);
    assert_eq!(gm::HIHAT_PRESSED, 26);
    assert_eq!(gm::HIHAT_TRASH_OPEN, 27);
    assert_eq!(gm::CRASH_16_BELL, 28);
    assert_eq!(gm::CRASH_16_TIP, 29);
    assert_eq!(gm::CRASH_18_BELL, 55);
    assert_eq!(gm::CRASH_18_TIP, 58);
    assert_eq!(gm::RIDE_TIP, 59);
    assert_eq!(gm::CHINA_BELL, 60);
    assert_eq!(gm::CHINA_TIP, 61);
    assert_eq!(gm::COUNT_STICK, 31);
}

#[test]
fn pad_table_notes_are_unique() {
    for (i, a) in GM_PADS.iter().enumerate() {
        for b in GM_PADS.iter().skip(i + 1) {
            assert_ne!(a.note, b.note, "duplicate note {} in GM_PADS", a.note);
        }
    }
}

#[test]
fn pad_index_for_note_round_trips() {
    for (i, pad) in GM_PADS.iter().enumerate() {
        assert_eq!(gm::pad_index_for_note(pad.note), Some(i));
    }
    assert_eq!(gm::pad_index_for_note(0), None);
    // GM Cowbell is used by the drumroll but is not a resonance-drums pad.
    assert_eq!(gm::pad_index_for_note(gm::COWBELL), None);
}

#[test]
fn pad_table_anchors_are_pinned() {
    assert_eq!(NUM_PADS, 30);
    assert_eq!(GM_PADS[0].note, gm::KICK);
    assert_eq!(GM_PADS[0].name, "Kick");
    assert_eq!(GM_PADS[1].note, gm::SNARE);
    assert_eq!(GM_PADS[1].name, "Snare");
    assert_eq!(GM_PADS[29].note, gm::COUNT_STICK);
    assert_eq!(GM_PADS[29].name, "Count Stick");
}
