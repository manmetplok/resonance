//! CLAP note-event delivery contract (code review ENG-01).
//!
//! CLAP requires a `process()` call's input events to be sorted by
//! `header.time` and to lie inside the block (`time < frames_count`).
//! The host used to hand `pending_notes` over in insertion order and
//! unclamped, so:
//!
//! - a live note queued against the whole callback buffer (offset up to
//!   `frames - 1`) reached the *head* sub-block of a loop seam with
//!   `time >= frames_count`; a plugin that drains events up to the
//!   current sample never reached it, and since the host had already
//!   drained its queue the note-off was lost (stuck note);
//! - timeline notes appended after live ones arrived out of order.
//!
//! These tests drive `ClapInstance::process` against a hand-rolled fake
//! plugin (same harness as `tests/plugin_output_scrub.rs`) that records
//! every input event it is handed.

use std::ffi::c_void;
use std::ptr;

use clap_sys::events::{clap_event_note, CLAP_EVENT_NOTE_OFF, CLAP_EVENT_NOTE_ON};
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};

use resonance_audio::__test_support::{ClapInstance, __instance_from_raw_for_test};

// ---------------------------------------------------------------------------
// Fake plugin
// ---------------------------------------------------------------------------

/// One recorded note event: `(time, is_note_on, key)`.
type Seen = (u32, bool, i16);

#[derive(Default)]
struct FakeState {
    /// Events of each process call, in delivery order, plus the call's
    /// `frames_count`.
    calls: Vec<(u32, Vec<Seen>)>,
}

unsafe fn fake_state<'a>(plugin: *const clap_plugin) -> &'a mut FakeState {
    &mut *((*plugin).plugin_data as *mut FakeState)
}

unsafe extern "C" fn fake_init(_plugin: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn fake_destroy(_plugin: *const clap_plugin) {}

unsafe extern "C" fn fake_activate(
    _plugin: *const clap_plugin,
    _sample_rate: f64,
    _min_frames: u32,
    _max_frames: u32,
) -> bool {
    true
}

unsafe extern "C" fn fake_deactivate(_plugin: *const clap_plugin) {}

unsafe extern "C" fn fake_start_processing(_plugin: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn fake_stop_processing(_plugin: *const clap_plugin) {}

unsafe extern "C" fn fake_process(
    plugin: *const clap_plugin,
    process: *const clap_process,
) -> clap_process_status {
    let state = fake_state(plugin);
    let events = &*(*process).in_events;
    let size = (events.size.unwrap())(events);
    let mut seen = Vec::new();
    for i in 0..size {
        let header = (events.get.unwrap())(events, i);
        let type_ = (*header).type_;
        if type_ == CLAP_EVENT_NOTE_ON || type_ == CLAP_EVENT_NOTE_OFF {
            let note = &*(header as *const clap_event_note);
            seen.push(((*header).time, type_ == CLAP_EVENT_NOTE_ON, note.key));
        }
    }
    state.calls.push(((*process).frames_count, seen));
    CLAP_PROCESS_CONTINUE
}

/// Build an instance around a fresh fake plugin plus a raw pointer to
/// its recorder. Both are intentionally leaked — the instance's `Drop`
/// still dereferences them.
fn make_instance() -> (ClapInstance, *mut FakeState) {
    let mut state_ptr: *mut FakeState = ptr::null_mut();
    let instance = __instance_from_raw_for_test(
        |_host| {
            let state = Box::into_raw(Box::<FakeState>::default());
            state_ptr = state;
            let plugin = Box::new(clap_plugin {
                desc: ptr::null(),
                plugin_data: state as *mut c_void,
                init: Some(fake_init),
                destroy: Some(fake_destroy),
                activate: Some(fake_activate),
                deactivate: Some(fake_deactivate),
                start_processing: Some(fake_start_processing),
                stop_processing: Some(fake_stop_processing),
                reset: None,
                process: Some(fake_process),
                get_extension: None,
                on_main_thread: None,
            });
            Box::into_raw(plugin) as *const clap_plugin
        },
        48_000,
    )
    .expect("fake plugin instance");
    (instance, state_ptr)
}

fn run(instance: &mut ClapInstance, frames: usize) {
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    instance.process(&mut l, &mut r, frames);
}

fn assert_in_block_and_sorted(calls: &[(u32, Vec<Seen>)]) {
    for (frames, seen) in calls {
        for w in seen.windows(2) {
            assert!(w[0].0 <= w[1].0, "events not sorted by time: {seen:?}");
        }
        for e in seen {
            assert!(e.0 < *frames, "event {e:?} outside block of {frames} frames");
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn events_are_sorted_and_late_ones_carry_to_the_next_sub_block() {
    let (mut inst, state) = make_instance();
    let state = unsafe { &mut *state };

    // A live note queued against the whole 128-frame callback, then a
    // timeline note for the head sub-block.
    inst.queue_note_on(60, 0.8, 100);
    inst.queue_note_on(62, 0.8, 5);

    // Head sub-block of a loop seam: 20 frames.
    run(&mut inst, 20);
    // Tail sub-block: the rest of the callback.
    run(&mut inst, 108);

    assert_in_block_and_sorted(&state.calls);
    assert_eq!(state.calls[0].1, vec![(5, true, 62)]);
    assert_eq!(
        state.calls[1].1,
        vec![(80, true, 60)],
        "the out-of-range note must arrive in the next call, re-based"
    );
}

#[test]
fn carried_note_off_survives_a_seam_panic() {
    let (mut inst, state) = make_instance();
    let state = unsafe { &mut *state };

    inst.queue_note_off(60, 100);
    run(&mut inst, 20);
    // The seam flushes instrument voices between the sub-blocks.
    inst.all_notes_off();
    // Tail sub-block, with a timeline note early in it.
    inst.queue_note_on(64, 0.8, 3);
    run(&mut inst, 108);

    assert_in_block_and_sorted(&state.calls);
    assert!(state.calls[0].1.is_empty());
    let tail = &state.calls[1].1;
    assert_eq!(tail.len(), 128 + 2);
    assert!(tail.contains(&(80, false, 60)), "carried note-off lost: {tail:?}");
    assert!(tail.contains(&(3, true, 64)));
}

#[test]
fn equal_time_events_keep_insertion_order() {
    let (mut inst, state) = make_instance();
    let state = unsafe { &mut *state };

    // Retrigger (off then on, same key) and a zero-length note (on then
    // off) at the same offset: both orders are meaningful, so the host
    // sort must be stable rather than impose a type order.
    inst.queue_note_on(70, 0.8, 50);
    inst.queue_note_off(60, 10);
    inst.queue_note_on(60, 0.8, 10);
    inst.queue_note_on(72, 0.8, 30);
    inst.queue_note_off(72, 30);
    run(&mut inst, 64);

    assert_in_block_and_sorted(&state.calls);
    assert_eq!(
        state.calls[0].1,
        vec![
            (10, false, 60),
            (10, true, 60),
            (30, true, 72),
            (30, false, 72),
            (50, true, 70),
        ]
    );
}
