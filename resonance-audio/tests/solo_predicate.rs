//! `any_top_level_solo` is the single solo predicate shared by the
//! live mixer and the bounce renderer — soloing a sub-track must not
//! flip the global solo state in either path.

use resonance_audio::types::{any_top_level_solo, Track};

#[test]
fn empty_track_list_is_not_soloed() {
    assert!(!any_top_level_solo([]));
}

#[test]
fn no_solo_anywhere_is_false() {
    let a = Track::new(1, "A".to_string());
    let b = Track::new(2, "B".to_string());
    assert!(!any_top_level_solo([&a, &b]));
}

#[test]
fn top_level_solo_is_true() {
    let a = Track::new(1, "A".to_string());
    let b = Track::new(2, "B".to_string());
    b.set_soloed(true);
    assert!(any_top_level_solo([&a, &b]));
}

#[test]
fn sub_track_solo_alone_is_false() {
    // Sub-tracks follow their parent's solo state; a soloed sub-track
    // must not silence unrelated top-level tracks.
    let parent = Track::new(1, "Kit".to_string());
    let sub = Track::new_sub_track(2, "Snare".to_string(), 1, 1);
    sub.set_soloed(true);
    assert!(!any_top_level_solo([&parent, &sub]));
}

#[test]
fn mixed_solo_counts_only_top_level() {
    let parent = Track::new(1, "Kit".to_string());
    let sub = Track::new_sub_track(2, "Snare".to_string(), 1, 1);
    sub.set_soloed(true);
    parent.set_soloed(true);
    assert!(any_top_level_solo([&parent, &sub]));
}
