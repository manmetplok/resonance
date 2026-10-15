//! `any_track_soloed` is the single solo predicate shared by the
//! live mixer and the bounce renderer. A soloed sub-track counts: soloing
//! one tap of a kit is "something is soloed" (BUG-02), so every unsoloed
//! track drops out. What it does to its parent and siblings is the render
//! path's business — see `sub_track_solo.rs`.

use resonance_audio::types::{any_track_soloed, Track};

#[test]
fn empty_track_list_is_not_soloed() {
    assert!(!any_track_soloed([]));
}

#[test]
fn no_solo_anywhere_is_false() {
    let a = Track::new(1, "A".to_string());
    let b = Track::new(2, "B".to_string());
    assert!(!any_track_soloed([&a, &b]));
}

#[test]
fn top_level_solo_is_true() {
    let a = Track::new(1, "A".to_string());
    let b = Track::new(2, "B".to_string());
    b.set_soloed(true);
    assert!(any_track_soloed([&a, &b]));
}

/// Deliberately flipped for BUG-02: this used to pin `false` ("a soloed
/// sub-track must not silence unrelated top-level tracks"), which is
/// exactly why the S button on a kit tap did nothing. The intent it
/// guarded — the rest of the soloed tap's own kit must not vanish with
/// it — is kept by the render path, which runs the parent as a fan-out
/// driver (`sub_track_solo.rs`).
#[test]
fn sub_track_solo_alone_is_true() {
    let parent = Track::new(1, "Kit".to_string());
    let sub = Track::new_sub_track(2, "Snare".to_string(), 1, 1);
    sub.set_soloed(true);
    assert!(any_track_soloed([&parent, &sub]));
}

#[test]
fn parent_and_sub_track_solo_is_true() {
    let parent = Track::new(1, "Kit".to_string());
    let sub = Track::new_sub_track(2, "Snare".to_string(), 1, 1);
    sub.set_soloed(true);
    parent.set_soloed(true);
    assert!(any_track_soloed([&parent, &sub]));
}
