//! Regression test for `ClapInstance::all_notes_off` dropping high
//! keys when `pending_notes` was near `MAX_PENDING_NOTES`: only keys
//! `0..remaining` got NoteOffs queued, so high MIDI keys kept
//! sounding after a panic and the dropped offs were never re-queued.
//!
//! Fixed by clearing the queue first — queued-but-undelivered events
//! are superseded by the panic (note-ons must not fire after it,
//! note-offs are redundant with the full sweep) — which also
//! guarantees all 128 offs fit without reallocating on the audio
//! thread.
//!
//! Needs a real plugin binary to instantiate `ClapInstance`; see
//! `plugin_binaries` for where it is found and why a missing one fails.

use std::path::PathBuf;

use resonance_audio::test_support::{ClapBundle, SyncClapInstance};

use crate::plugin_binaries::plugin_binary;

fn find_bundled_clap() -> Option<PathBuf> {
    plugin_binary("resonance-amp")
}

fn make_instance(bundle: &ClapBundle) -> SyncClapInstance {
    let descriptor_id = bundle
        .descriptors()
        .first()
        .map(|d| d.id.clone())
        .expect("bundle exposes at least one plugin");
    SyncClapInstance(
        bundle
            .create_instance(&descriptor_id, 48_000)
            .expect("create_instance"),
    )
}

#[test]
fn all_notes_off_covers_every_key_even_when_queue_is_full() {
    let Some(bundle_path) = find_bundled_clap() else {
        return;
    };
    let bundle = ClapBundle::load(&bundle_path).expect("bundle load");
    let mut inst = make_instance(&bundle);

    // Fill the queue to capacity with note-ons (a live MIDI burst).
    for i in 0..512u32 {
        inst.0.queue_note_on((i % 128) as u8, 0.8, i);
    }
    let queued = inst.0.__pending_notes_for_test().len();
    assert!(queued >= 128, "queue should be at capacity, got {queued}");

    inst.0.all_notes_off();

    let pending = inst.0.__pending_notes_for_test();
    // Exactly one NoteOff per key, no surviving note-ons.
    assert_eq!(pending.len(), 128);
    for (key, &(is_on, k, _vel, offset)) in pending.iter().enumerate() {
        assert!(!is_on, "note-on survived all_notes_off at key {k}");
        assert_eq!(k as usize, key);
        assert_eq!(offset, 0);
    }
}

#[test]
fn all_notes_off_on_empty_queue_covers_every_key() {
    let Some(bundle_path) = find_bundled_clap() else {
        return;
    };
    let bundle = ClapBundle::load(&bundle_path).expect("bundle load");
    let mut inst = make_instance(&bundle);

    inst.0.all_notes_off();

    let pending = inst.0.__pending_notes_for_test();
    assert_eq!(pending.len(), 128);
    assert!(pending.iter().all(|&(is_on, _, _, _)| !is_on));
}
