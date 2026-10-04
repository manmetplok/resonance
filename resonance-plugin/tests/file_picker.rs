//! Tests for the non-blocking file/folder picker (code review PUX-07):
//! `start`/`poll` must behave like a one-shot background job (busy
//! while running, `None` until it resolves, the answer exactly once),
//! without ever touching an actual native dialog — there is no display
//! server in the test environment, and a real `rfd::FileDialog` call
//! would simply hang.
//!
//! Linux only: Cocoa's modal dialog runs synchronously on the guarded
//! AppKit main thread and has no `FilePicker` to test.
#![cfg(all(feature = "editor-widgets", not(target_os = "macos")))]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use resonance_plugin::file_picker::{FilePicker, PickerAnswer};

/// Block up to a couple of seconds for `picker.poll()` to resolve —
/// the background thread really does run concurrently, so a single
/// poll right after `start` can legitimately see "not finished yet".
fn wait_for(picker: &mut FilePicker) -> PickerAnswer {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(answer) = picker.poll() {
            return answer;
        }
        assert!(Instant::now() < deadline, "picker never resolved");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn idle_picker_is_not_busy_and_poll_is_none() {
    let mut picker = FilePicker::default();
    assert!(!picker.busy());
    assert!(picker.poll().is_none());
}

#[test]
fn start_with_makes_the_picker_busy_until_it_resolves() {
    let mut picker = FilePicker::default();
    assert!(picker.start_with(|| PickerAnswer::One(Some(PathBuf::from("/tmp/chosen.json")))));
    assert!(picker.busy());

    let answer = wait_for(&mut picker);
    match answer {
        PickerAnswer::One(Some(p)) => assert_eq!(p, PathBuf::from("/tmp/chosen.json")),
        other => panic!("unexpected answer: {other:?}"),
    }
    // The answer was collected: no longer busy, nothing left to poll.
    assert!(!picker.busy());
    assert!(picker.poll().is_none());
}

#[test]
fn a_second_start_is_dropped_while_one_is_already_running() {
    let mut picker = FilePicker::default();
    assert!(picker.start_with(|| {
        std::thread::sleep(Duration::from_millis(50));
        PickerAnswer::One(None)
    }));
    // Dropped, not queued — `start_with`'s own closure never runs.
    assert!(!picker.start_with(|| panic!("a queued second dialog must never run")));
    assert!(picker.busy());

    let _ = wait_for(&mut picker);
    assert!(!picker.busy());
}

#[test]
fn a_dismissed_dialog_answers_none() {
    let mut picker = FilePicker::default();
    picker.start_with(|| PickerAnswer::One(None));
    assert_eq!(wait_for(&mut picker).into_one(), None);
}

#[test]
fn many_answer_collapses_to_the_last_path_via_into_one() {
    let answer = PickerAnswer::Many(vec![PathBuf::from("/a"), PathBuf::from("/b")]);
    assert_eq!(answer.into_one(), Some(PathBuf::from("/b")));
}

#[test]
fn one_answer_expands_to_zero_or_one_paths_via_into_many() {
    assert_eq!(
        PickerAnswer::One(Some(PathBuf::from("/a"))).into_many(),
        vec![PathBuf::from("/a")]
    );
    assert_eq!(PickerAnswer::One(None).into_many(), Vec::<PathBuf>::new());
}
