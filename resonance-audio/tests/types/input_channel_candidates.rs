//! Channel counts the cpal input fallback tries, in order (FU-M3a): a
//! device wider than `MAX_INPUT_CHANNELS` whose capped request is
//! rejected must still open at a supported count that fits, and only
//! fail — clearly — when none does.

use resonance_audio::test_support::input_channel_candidates;

const MAX: u16 = 32;

#[test]
fn capped_request_first_then_supported_widest_first_then_default() {
    // A 64-ch MADI device that also offers 16 and 2.
    assert_eq!(
        input_channel_candidates(32, 64, &[64, 2, 16], MAX),
        vec![32, 16, 2]
    );
    // A default that fits comes last, after the supported counts.
    assert_eq!(input_channel_candidates(8, 4, &[2], MAX), vec![8, 2, 4]);
}

#[test]
fn duplicates_are_tried_once() {
    assert_eq!(input_channel_candidates(2, 2, &[2, 2, 1], MAX), vec![2, 1]);
}

#[test]
fn never_above_the_cap_and_empty_when_nothing_fits() {
    assert!(input_channel_candidates(64, 64, &[64, 128], MAX)
        .iter()
        .all(|&c| c <= MAX));
    assert_eq!(
        input_channel_candidates(0, 64, &[64, 128], MAX),
        Vec::<u16>::new()
    );
}
