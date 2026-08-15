//! The editor's per-pad "take N of M" readout (ba todo #1329).
//!
//! The audio thread publishes both the round-robin index and the layer's
//! take count into one atomic per pad; these tests cover the packing, the
//! unpacking, and that a real `note_on` cycles the published value so the
//! readout moves as takes advance.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use resonance_drums::drum_map::{self, NUM_PADS, PAD_MAPPINGS};
use resonance_drums::dsp::DrumSampler;
use resonance_drums::kit::{LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer};
use resonance_drums::rr_display::{self, RoundRobin};

fn pad_with_takes(mapping_index: usize, layers: usize, takes: usize) -> LoadedPad {
    let m = &PAD_MAPPINGS[mapping_index];
    LoadedPad {
        name: m.name.to_string(),
        choke_group: m.choke_group,
        output_group: m.output_group,
        close_mics: vec![LoadedMicBank {
            position: "close".to_string(),
            setup_key: String::new(),
            layers: (0..layers)
                .map(|_| VelocityLayer {
                    round_robins: (0..takes)
                        .map(|_| LoadedSample {
                            data: vec![0.5; 2],
                            frames: 1,
                        })
                        .collect(),
                })
                .collect(),
        }],
        overhead: None,
    }
}

fn sampler_with(takes: usize) -> (DrumSampler, Arc<[AtomicU32; NUM_PADS]>) {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut sampler = DrumSampler::new(rx);
    let last_rr: Arc<[AtomicU32; NUM_PADS]> = Arc::new(std::array::from_fn(|_| AtomicU32::new(0)));
    sampler.set_last_rr(last_rr.clone());
    sampler.pads = (0..PAD_MAPPINGS.len())
        .map(|i| pad_with_takes(i, 1, takes))
        .collect();
    (sampler, last_rr)
}

fn published(last_rr: &Arc<[AtomicU32; NUM_PADS]>, pad: usize) -> Option<RoundRobin> {
    rr_display::unpack(last_rr[pad].load(Ordering::Relaxed))
}

#[test]
fn pack_and_unpack_round_trip() {
    for count in [1usize, 2, 3, 12, 64] {
        for index in 0..count {
            let rr = rr_display::unpack(rr_display::pack(index, count))
                .expect("a triggered pad always unpacks");
            assert_eq!(rr.take_index, index);
            assert_eq!(rr.take_count, count);
        }
    }
}

/// Take 1 of 1 must not collide with the "never triggered" sentinel: the
/// count lives in the high half precisely so index 0 stays distinguishable.
#[test]
fn first_take_of_a_single_take_layer_is_not_the_sentinel() {
    let packed = rr_display::pack(0, 1);
    assert_ne!(packed, 0);
    let rr = rr_display::unpack(packed).expect("take 1 of 1 is a real reading");
    assert_eq!(rr.label(), "take 1 of 1");
    assert!(!rr.cycles());
}

#[test]
fn untriggered_pads_have_no_reading() {
    assert_eq!(rr_display::unpack(0), None);
    // A count of zero is not a valid publication either.
    assert_eq!(rr_display::unpack(5), None);
}

#[test]
fn labels_are_one_based() {
    let rr = rr_display::unpack(rr_display::pack(1, 3)).unwrap();
    assert_eq!(rr.label(), "take 2 of 3");
    assert_eq!(rr.compact(), "2/3");
    assert!(rr.cycles());
}

/// An out-of-range index (a torn or stale publication) is clamped rather
/// than shown as "take 9 of 3".
#[test]
fn index_is_clamped_to_the_count() {
    let rr = rr_display::unpack(8 | (3 << 16)).unwrap();
    assert_eq!(rr.take_count, 3);
    assert_eq!(rr.take_index, 2);
}

/// Both halves reach the editor from a real hit, and the index advances
/// as the pad cycles — so the readout moves take by take.
#[test]
fn note_on_publishes_take_and_depth() {
    let (mut sampler, last_rr) = sampler_with(3);
    let pad = drum_map::pad_index_for_note(drum_map::KICK).expect("kick pad");
    assert_eq!(published(&last_rr, pad), None, "silent until played");

    let mut seen = Vec::new();
    for _ in 0..6 {
        sampler.note_on(drum_map::KICK, 0.9);
        let rr = published(&last_rr, pad).expect("a hit publishes a reading");
        assert_eq!(rr.take_count, 3, "the layer's depth reaches the editor");
        seen.push(rr.take_index);
    }
    assert_eq!(seen.len(), 6);
    assert!(
        seen.windows(2).all(|w| w[0] != w[1]),
        "consecutive hits must advance the take: {seen:?}"
    );
    let mut distinct = seen.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct, vec![0, 1, 2], "all three takes are reported");

    // Other pads stay unreported — the readout is per pad.
    let snare = drum_map::pad_index_for_note(drum_map::SNARE).expect("snare pad");
    assert_eq!(published(&last_rr, snare), None);
}

/// A single-take pad reports its depth honestly instead of looking like a
/// cycling pad that never moves.
#[test]
fn single_take_pad_reports_depth_one() {
    let (mut sampler, last_rr) = sampler_with(1);
    let pad = drum_map::pad_index_for_note(drum_map::KICK).expect("kick pad");
    sampler.note_on(drum_map::KICK, 0.5);
    let rr = published(&last_rr, pad).expect("a hit publishes a reading");
    assert_eq!(rr.take_count, 1);
    assert_eq!(rr.take_index, 0);
    assert!(!rr.cycles());
}
