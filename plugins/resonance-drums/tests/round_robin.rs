use resonance_drums::drum_map::{self, PAD_MAPPINGS};
use resonance_drums::kit::{LoadedMicBank, LoadedPad, LoadedSample, OutputGroup, VelocityLayer};
use resonance_drums::sampler::{pick_rr, pick_velocity_layer, DrumSampler, MAX_LAYERS};
use resonance_drums::voice::VoiceState;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Build a minimal `LoadedPad` with the given number of velocity layers and
/// round-robin takes per layer. Samples are 1-frame stubs.
fn make_pad(
    n_layers: usize,
    rr_per_layer: usize,
    output_group: OutputGroup,
    choke_group: Option<u8>,
) -> LoadedPad {
    let layers: Vec<VelocityLayer> = (0..n_layers)
        .map(|_| VelocityLayer {
            round_robins: (0..rr_per_layer)
                .map(|_| LoadedSample {
                    data: vec![0.0; 2],
                    frames: 1,
                })
                .collect(),
        })
        .collect();
    LoadedPad {
        name: "test".to_string(),
        choke_group,
        output_group,
        close_mics: vec![LoadedMicBank {
            position: "close".to_string(),
            setup_key: String::new(),
            layers,
        }],
        overhead: None,
    }
}

/// Build a full pad set from PAD_MAPPINGS with the given layer/rr counts.
fn make_all_pads(n_layers: usize, rr_per_layer: usize) -> Vec<LoadedPad> {
    PAD_MAPPINGS
        .iter()
        .map(|m| make_pad(n_layers, rr_per_layer, m.output_group, m.choke_group))
        .collect()
}

/// Create a `DrumSampler` with a disconnected channel (no loader).
fn make_sampler() -> DrumSampler {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    DrumSampler::new(rx)
}

/// Collect the `rr_index` values from all active voices for a given pad.
fn active_rr_indices(sampler: &DrumSampler, pad_index: usize) -> Vec<usize> {
    sampler
        .voices
        .iter()
        .filter(|v| v.active && v.pad_index == pad_index)
        .map(|v| v.rr_index)
        .collect()
}

// ---------------------------------------------------------------------------
// pick_rr unit tests
// ---------------------------------------------------------------------------

#[test]
fn pick_rr_cycles_through_all_takes() {
    let mut counter = 0u32;
    let mut picks = Vec::new();
    for _ in 0..9 {
        picks.push(pick_rr(&mut counter, 3));
    }
    assert_eq!(picks, vec![0, 1, 2, 0, 1, 2, 0, 1, 2]);
}

#[test]
fn pick_rr_single_take_always_zero() {
    let mut counter = 0u32;
    for _ in 0..5 {
        assert_eq!(pick_rr(&mut counter, 1), 0);
    }
}

#[test]
fn pick_rr_wraps_at_u32_max() {
    let mut counter = u32::MAX - 1;
    assert_eq!(pick_rr(&mut counter, 3), ((u32::MAX - 1) % 3) as usize);
    assert_eq!(pick_rr(&mut counter, 3), (u32::MAX % 3) as usize);
    // Next call wraps to 0.
    assert_eq!(pick_rr(&mut counter, 3), 0);
}

#[test]
fn pick_rr_never_repeats_consecutively() {
    for n_rrs in 2..=8 {
        let mut counter = 0u32;
        let mut prev = pick_rr(&mut counter, n_rrs);
        for hit in 1..20 {
            let curr = pick_rr(&mut counter, n_rrs);
            assert_ne!(
                prev, curr,
                "n_rrs={n_rrs} hit={hit}: consecutive picks should differ"
            );
            prev = curr;
        }
    }
}

#[test]
fn pick_rr_covers_all_indices_in_one_cycle() {
    for n_rrs in 1..=8 {
        let mut counter = 0u32;
        let mut seen = vec![false; n_rrs];
        for _ in 0..n_rrs {
            let idx = pick_rr(&mut counter, n_rrs);
            seen[idx] = true;
        }
        assert!(
            seen.iter().all(|&s| s),
            "n_rrs={n_rrs}: not all indices visited in first cycle"
        );
    }
}

#[test]
fn pick_rr_counter_advances_independently_of_n_rrs() {
    // The counter should advance by 1 each call regardless of n_rrs.
    let mut counter = 0u32;
    pick_rr(&mut counter, 5);
    assert_eq!(counter, 1);
    pick_rr(&mut counter, 5);
    assert_eq!(counter, 2);
    pick_rr(&mut counter, 5);
    assert_eq!(counter, 3);
}

// ---------------------------------------------------------------------------
// pick_velocity_layer unit tests
// ---------------------------------------------------------------------------

#[test]
fn velocity_layer_boundary_values() {
    // With 4 layers, boundaries are at 0.25, 0.5, 0.75.
    assert_eq!(pick_velocity_layer(0.0, 4), 0);
    assert_eq!(pick_velocity_layer(0.24, 4), 0);
    assert_eq!(pick_velocity_layer(0.25, 4), 1);
    assert_eq!(pick_velocity_layer(0.49, 4), 1);
    assert_eq!(pick_velocity_layer(0.5, 4), 2);
    assert_eq!(pick_velocity_layer(0.74, 4), 2);
    assert_eq!(pick_velocity_layer(0.75, 4), 3);
    assert_eq!(pick_velocity_layer(1.0, 4), 3);
}

#[test]
fn velocity_layer_max_layer_count() {
    // Ensure it works up to MAX_LAYERS.
    for v_pct in 0..=100 {
        let v = v_pct as f32 / 100.0;
        let idx = pick_velocity_layer(v, MAX_LAYERS);
        assert!(idx < MAX_LAYERS, "v={v} idx={idx}");
    }
}

// ---------------------------------------------------------------------------
// DrumSampler round-robin integration tests
// ---------------------------------------------------------------------------

#[test]
fn note_on_cycles_rr_across_hits() {
    let mut sampler = make_sampler();
    sampler.pads = make_all_pads(1, 4);

    let note = drum_map::KICK;
    let mut rr_sequence = Vec::new();
    for _ in 0..8 {
        sampler.reset();
        sampler.note_on(note, 0.8);
        let indices = active_rr_indices(&sampler, 0);
        assert_eq!(indices.len(), 1, "expected 1 voice per hit");
        rr_sequence.push(indices[0]);
    }
    assert_eq!(rr_sequence, vec![0, 1, 2, 3, 0, 1, 2, 3]);
}

#[test]
fn note_on_single_rr_always_zero() {
    let mut sampler = make_sampler();
    sampler.pads = make_all_pads(1, 1);

    for _ in 0..5 {
        sampler.reset();
        sampler.note_on(drum_map::SNARE, 0.5);
        assert_eq!(active_rr_indices(&sampler, 1), vec![0]);
    }
}

#[test]
fn independent_rr_counters_per_pad() {
    let mut sampler = make_sampler();
    sampler.pads = make_all_pads(1, 3);

    // Advance kick to rr 1.
    sampler.reset();
    sampler.note_on(drum_map::KICK, 0.8);
    assert_eq!(active_rr_indices(&sampler, 0), vec![0]);

    sampler.reset();
    sampler.note_on(drum_map::KICK, 0.8);
    assert_eq!(active_rr_indices(&sampler, 0), vec![1]);

    // Snare's counter is still at 0.
    sampler.reset();
    sampler.note_on(drum_map::SNARE, 0.8);
    assert_eq!(active_rr_indices(&sampler, 1), vec![0]);
}

#[test]
fn independent_rr_counters_per_velocity_layer() {
    let mut sampler = make_sampler();
    sampler.pads = make_all_pads(2, 3);

    let note = drum_map::KICK;

    // Two soft hits (layer 0).
    sampler.reset();
    sampler.note_on(note, 0.1);
    assert_eq!(active_rr_indices(&sampler, 0), vec![0]);
    sampler.reset();
    sampler.note_on(note, 0.1);
    assert_eq!(active_rr_indices(&sampler, 0), vec![1]);

    // Hard hit (layer 1) should start at rr 0.
    sampler.reset();
    sampler.note_on(note, 0.9);
    assert_eq!(active_rr_indices(&sampler, 0), vec![0]);
}

#[test]
fn kit_swap_resets_rr_counters() {
    let (tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut sampler = DrumSampler::new(rx);
    sampler.pads = make_all_pads(1, 3);

    // Advance kick counter.
    sampler.note_on(drum_map::KICK, 0.8);
    sampler.note_on(drum_map::KICK, 0.8);

    // Swap in new kit.
    tx.send(make_all_pads(1, 3)).unwrap();
    sampler.try_swap_kit();

    // Counter should be back at 0.
    sampler.note_on(drum_map::KICK, 0.8);
    assert_eq!(active_rr_indices(&sampler, 0), vec![0]);
}

#[test]
fn empty_pad_triggers_no_voices() {
    let mut sampler = make_sampler();
    sampler.pads = PAD_MAPPINGS
        .iter()
        .map(|m| LoadedPad {
            name: m.name.to_string(),
            choke_group: m.choke_group,
            output_group: m.output_group,
            close_mics: Vec::new(),
            overhead: None,
        })
        .collect();

    sampler.note_on(drum_map::KICK, 0.8);
    assert!(sampler.voices.iter().all(|v| !v.active));
}

#[test]
fn all_voices_from_multi_bank_hit_share_rr() {
    let mut sampler = make_sampler();

    let layers = || -> Vec<VelocityLayer> {
        vec![VelocityLayer {
            round_robins: (0..4)
                .map(|_| LoadedSample {
                    data: vec![0.0; 2],
                    frames: 1,
                })
                .collect(),
        }]
    };
    let kick_pad = LoadedPad {
        name: "Kick".to_string(),
        choke_group: None,
        output_group: OutputGroup::Kick,
        close_mics: vec![
            LoadedMicBank {
                position: "KickIn".to_string(),
                setup_key: String::new(),
                layers: layers(),
            },
            LoadedMicBank {
                position: "KickOut".to_string(),
                setup_key: String::new(),
                layers: layers(),
            },
        ],
        overhead: Some(LoadedMicBank {
            position: "OH".to_string(),
            setup_key: String::new(),
            layers: layers(),
        }),
    };

    sampler.pads = std::iter::once(kick_pad)
        .chain(
            PAD_MAPPINGS[1..]
                .iter()
                .map(|m| make_pad(1, 1, m.output_group, m.choke_group)),
        )
        .collect();

    sampler.note_on(drum_map::KICK, 0.8);

    let active: Vec<_> = sampler
        .voices
        .iter()
        .filter(|v| v.active && v.pad_index == 0)
        .collect();
    assert_eq!(active.len(), 3, "kick with 2 close + OH = 3 voices");

    let rr = active[0].rr_index;
    for v in &active {
        assert_eq!(v.rr_index, rr, "all voices must share the same rr_index");
    }
}

#[test]
fn note_off_does_not_affect_rr_state() {
    let mut sampler = make_sampler();
    sampler.pads = make_all_pads(1, 3);

    // Hit and advance to rr 0.
    sampler.reset();
    sampler.note_on(drum_map::KICK, 0.8);
    assert_eq!(active_rr_indices(&sampler, 0), vec![0]);

    // note_off should be a no-op for drums.
    sampler.note_off(drum_map::KICK);

    // Next hit should continue at rr 1.
    sampler.reset();
    sampler.note_on(drum_map::KICK, 0.8);
    assert_eq!(active_rr_indices(&sampler, 0), vec![1]);
}

#[test]
fn choke_does_not_reset_rr_counter() {
    let mut sampler = make_sampler();
    sampler.pads = make_all_pads(1, 3);

    // Use hi-hat which has a choke group.
    let note = drum_map::HIHAT_CLOSED; // pad index 2

    sampler.note_on(note, 0.8); // rr 0
    sampler.note_on(note, 0.8); // rr 1

    // Choke via note-level choke.
    sampler.choke_note(note);

    // Verify voices are releasing, not active-playing.
    let releasing: Vec<_> = sampler
        .voices
        .iter()
        .filter(|v| v.active && v.pad_index == 2 && v.state == VoiceState::Releasing)
        .collect();
    assert!(!releasing.is_empty(), "choke should trigger release");

    // Next hit should continue RR sequence at 2, not reset to 0.
    sampler.reset();
    sampler.note_on(note, 0.8);
    assert_eq!(active_rr_indices(&sampler, 2), vec![2]);
}

#[test]
fn choke_group_triggers_release_and_rr_continues() {
    let mut sampler = make_sampler();
    sampler.pads = make_all_pads(1, 4);

    // Hi-hat closed and open share choke group 1.
    // Hit closed twice (rr 0, 1).
    sampler.note_on(drum_map::HIHAT_CLOSED, 0.8);
    sampler.note_on(drum_map::HIHAT_CLOSED, 0.8);

    // Hit open — should choke the closed voices but the closed pad's
    // RR counter stays advanced.
    sampler.reset();
    sampler.note_on(drum_map::HIHAT_OPEN, 0.8);
    assert_eq!(
        active_rr_indices(&sampler, 3),
        vec![0],
        "open hat rr starts at 0"
    );

    // Next closed hit continues at rr 2.
    sampler.reset();
    sampler.note_on(drum_map::HIHAT_CLOSED, 0.8);
    assert_eq!(active_rr_indices(&sampler, 2), vec![2]);
}

#[test]
fn overhead_only_pad_uses_rr() {
    let mut sampler = make_sampler();

    // Place an OH-only pad at index 12 (Crash 16 Edge, note 49).
    sampler.pads = PAD_MAPPINGS
        .iter()
        .enumerate()
        .map(|(i, m)| {
            if i == 12 {
                LoadedPad {
                    name: "CymbalOH".to_string(),
                    choke_group: None,
                    output_group: OutputGroup::Cymbals,
                    close_mics: Vec::new(),
                    overhead: Some(LoadedMicBank {
                        position: "OH".to_string(),
                        setup_key: String::new(),
                        layers: vec![VelocityLayer {
                            round_robins: (0..3)
                                .map(|_| LoadedSample {
                                    data: vec![0.0; 2],
                                    frames: 1,
                                })
                                .collect(),
                        }],
                    }),
                }
            } else {
                make_pad(1, 1, m.output_group, m.choke_group)
            }
        })
        .collect();

    let note = drum_map::CRASH_16_EDGE;
    let mut rr_seq = Vec::new();
    for _ in 0..6 {
        sampler.reset();
        sampler.note_on(note, 0.8);
        let indices = active_rr_indices(&sampler, 12);
        assert_eq!(indices.len(), 1, "OH-only pad should spawn 1 voice");
        rr_seq.push(indices[0]);
    }
    assert_eq!(rr_seq, vec![0, 1, 2, 0, 1, 2]);
}

#[test]
fn voice_stealing_does_not_corrupt_rr_sequence() {
    let mut sampler = make_sampler();
    sampler.pads = make_all_pads(1, 4);

    // Fire enough hits without resetting to exhaust all voices (64 max).
    // The RR counter should still cycle correctly even though old voices
    // are being stolen.
    let note = drum_map::KICK;
    let mut rr_seq = Vec::new();
    for i in 0..80 {
        // Don't reset — let voices accumulate and get stolen.
        sampler.note_on(note, 0.8);
        // Find the most recently spawned voice (highest age).
        let newest = sampler
            .voices
            .iter()
            .filter(|v| v.active && v.pad_index == 0)
            .max_by_key(|v| v.age);
        if let Some(v) = newest {
            rr_seq.push(v.rr_index);
        } else {
            panic!("hit {i}: no active voice found after note_on");
        }
    }
    // Verify the RR pattern: should be 0,1,2,3 repeating.
    for (i, &rr) in rr_seq.iter().enumerate() {
        assert_eq!(rr, i % 4, "hit {i}: expected rr {} got {rr}", i % 4);
    }
}

#[test]
fn unmapped_note_does_not_affect_rr() {
    let mut sampler = make_sampler();
    sampler.pads = make_all_pads(1, 3);

    // Hit kick once.
    sampler.reset();
    sampler.note_on(drum_map::KICK, 0.8);
    assert_eq!(active_rr_indices(&sampler, 0), vec![0]);

    // Send an unmapped MIDI note — should be silently ignored.
    sampler.note_on(127, 0.8);

    // Next kick hit should still be rr 1.
    sampler.reset();
    sampler.note_on(drum_map::KICK, 0.8);
    assert_eq!(active_rr_indices(&sampler, 0), vec![1]);
}

#[test]
fn many_layers_each_with_own_rr_counter() {
    let mut sampler = make_sampler();
    let n_layers = 8;
    let n_rr = 3;
    sampler.pads = make_all_pads(n_layers, n_rr);

    let note = drum_map::KICK;

    // Hit each velocity layer twice. Each layer's RR counter is independent.
    for layer_idx in 0..n_layers {
        let velocity = (layer_idx as f32 + 0.5) / n_layers as f32;
        let actual_layer = pick_velocity_layer(velocity, n_layers);
        assert_eq!(actual_layer, layer_idx, "velocity mapping sanity check");

        sampler.reset();
        sampler.note_on(note, velocity);
        assert_eq!(
            active_rr_indices(&sampler, 0),
            vec![0],
            "layer {layer_idx} first hit should be rr 0"
        );

        sampler.reset();
        sampler.note_on(note, velocity);
        assert_eq!(
            active_rr_indices(&sampler, 0),
            vec![1],
            "layer {layer_idx} second hit should be rr 1"
        );
    }
}

