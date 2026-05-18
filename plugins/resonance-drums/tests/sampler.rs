use resonance_drums::kit::{LoadedMicBank, LoadedPad, LoadedSample, OutputGroup, VelocityLayer};
use resonance_drums::dsp::{pick_rr, pick_velocity_layer, DrumSampler};
use resonance_drums::{drum_map, kit};

#[test]
fn velocity_layer_single() {
    assert_eq!(pick_velocity_layer(0.0, 1), 0);
    assert_eq!(pick_velocity_layer(0.5, 1), 0);
    assert_eq!(pick_velocity_layer(1.0, 1), 0);
}

#[test]
fn velocity_layer_two() {
    assert_eq!(pick_velocity_layer(0.0, 2), 0);
    assert_eq!(pick_velocity_layer(0.25, 2), 0);
    assert_eq!(pick_velocity_layer(0.49, 2), 0);
    assert_eq!(pick_velocity_layer(0.5, 2), 1);
    assert_eq!(pick_velocity_layer(0.99, 2), 1);
    assert_eq!(pick_velocity_layer(1.0, 2), 1);
}

#[test]
fn velocity_layer_ten() {
    assert_eq!(pick_velocity_layer(0.0, 10), 0);
    assert_eq!(pick_velocity_layer(0.09, 10), 0);
    assert_eq!(pick_velocity_layer(0.1, 10), 1);
    assert_eq!(pick_velocity_layer(0.5, 10), 5);
    assert_eq!(pick_velocity_layer(0.95, 10), 9);
    assert_eq!(pick_velocity_layer(1.0, 10), 9);
}

#[test]
fn velocity_layer_clamps() {
    // Out-of-range input (shouldn't happen in practice but shouldn't panic).
    assert_eq!(pick_velocity_layer(-1.0, 10), 0);
    assert_eq!(pick_velocity_layer(1.5, 10), 9);
    assert_eq!(pick_velocity_layer(f32::NAN, 10), 0);
}

#[test]
fn velocity_layer_large() {
    // MAX_LAYERS boundary. Every input should still produce a valid index.
    for n in [16usize, 28, 32] {
        for v_pct in 0..=100 {
            let v = v_pct as f32 / 100.0;
            let idx = pick_velocity_layer(v, n);
            assert!(idx < n, "n={n} v={v} idx={idx}");
        }
    }
}

#[test]
fn rr_cycles_round_robin() {
    let mut counter = 0u32;
    let mut picks = Vec::new();
    for _ in 0..9 {
        picks.push(pick_rr(&mut counter, 3));
    }
    assert_eq!(picks, vec![0, 1, 2, 0, 1, 2, 0, 1, 2]);
}

#[test]
fn rr_single_take() {
    let mut counter = 0u32;
    for _ in 0..5 {
        assert_eq!(pick_rr(&mut counter, 1), 0);
    }
}

#[test]
fn rr_counter_wraps() {
    let mut counter = u32::MAX - 1;
    assert_eq!(pick_rr(&mut counter, 3), ((u32::MAX - 1) % 3) as usize);
    assert_eq!(pick_rr(&mut counter, 3), (u32::MAX % 3) as usize);
    // Next call wraps to 0.
    assert_eq!(pick_rr(&mut counter, 3), 0);
}

#[test]
fn rr_two_takes() {
    let mut counter = 0u32;
    let mut picks = Vec::new();
    for _ in 0..6 {
        picks.push(pick_rr(&mut counter, 2));
    }
    assert_eq!(picks, vec![0, 1, 0, 1, 0, 1]);
}

#[test]
fn rr_consecutive_hits_never_repeat_with_multiple_takes() {
    // With n >= 2 round robins the pick_rr function should never return
    // the same index twice in a row.
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
fn rr_covers_all_indices() {
    // After exactly n_rrs hits every index in [0, n_rrs) should have
    // appeared at least once.
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

// -------------------------------------------------------------------
// Integration tests: verify round-robin through DrumSampler::note_on
// -------------------------------------------------------------------

/// Build a minimal `LoadedPad` with the given number of round-robin
/// takes per velocity layer. Each "sample" is a trivial 1-frame stereo
/// buffer — we never render audio in these tests, only inspect the
/// `rr_index` assigned to the voices.
fn make_test_pad(
    n_layers: usize,
    rr_per_layer: usize,
    output_group: kit::OutputGroup,
    choke_group: Option<u8>,
) -> LoadedPad {
    let layers: Vec<VelocityLayer> = (0..n_layers)
        .map(|_| VelocityLayer {
            round_robins: (0..rr_per_layer)
                .map(|_| LoadedSample {
                    data: vec![0.0; 2], // 1 stereo frame
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

/// Create a `DrumSampler` disconnected from any loader (the receiver
/// end of the channel is held but no sender will ever push to it).
fn make_test_sampler() -> DrumSampler {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    DrumSampler::new(rx)
}

/// Collect the `rr_index` from every active voice that was spawned for
/// a given pad after the most recent `note_on`.
fn active_rr_indices(sampler: &DrumSampler, pad_index: usize) -> Vec<usize> {
    sampler
        .voices
        .iter()
        .filter(|v| v.active && v.pad_index == pad_index)
        .map(|v| v.rr_index)
        .collect()
}

#[test]
fn note_on_cycles_rr_through_voices() {
    let mut sampler = make_test_sampler();
    // Pad 0 (Kick, note 36) with 1 layer and 4 round-robin takes.
    sampler.pads = drum_map::PAD_MAPPINGS
        .iter()
        .map(|m| make_test_pad(1, 4, m.output_group, m.choke_group))
        .collect();

    let note = drum_map::KICK; // pad index 0
    let mut rr_sequence = Vec::new();
    for _ in 0..8 {
        // Reset all voices so we can inspect only the freshly spawned
        // ones after each note_on.
        sampler.reset();
        sampler.note_on(note, 0.8);
        let indices = active_rr_indices(&sampler, 0);
        // With one close-mic bank and no overhead, exactly 1 voice.
        assert_eq!(indices.len(), 1, "expected exactly 1 voice per hit");
        rr_sequence.push(indices[0]);
    }
    assert_eq!(
        rr_sequence,
        vec![0, 1, 2, 3, 0, 1, 2, 3],
        "round-robin should cycle 0..3 and wrap"
    );
}

#[test]
fn note_on_rr_single_take_always_zero() {
    let mut sampler = make_test_sampler();
    sampler.pads = drum_map::PAD_MAPPINGS
        .iter()
        .map(|m| make_test_pad(1, 1, m.output_group, m.choke_group))
        .collect();

    let note = drum_map::SNARE; // pad index 1
    for _ in 0..5 {
        sampler.reset();
        sampler.note_on(note, 0.5);
        let indices = active_rr_indices(&sampler, 1);
        assert_eq!(indices, vec![0]);
    }
}

#[test]
fn note_on_empty_pad_is_noop() {
    let mut sampler = make_test_sampler();
    // Build pads where pad 0 has zero close mics and no overhead.
    sampler.pads = drum_map::PAD_MAPPINGS
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
    let active: Vec<_> = sampler.voices.iter().filter(|v| v.active).collect();
    assert!(
        active.is_empty(),
        "no voices should be spawned for an empty pad"
    );
}

#[test]
fn different_pads_have_independent_rr_counters() {
    let mut sampler = make_test_sampler();
    // All pads get 1 layer, 3 round-robin takes.
    sampler.pads = drum_map::PAD_MAPPINGS
        .iter()
        .map(|m| make_test_pad(1, 3, m.output_group, m.choke_group))
        .collect();

    // Hit Kick twice (should advance to rr 0, then 1).
    sampler.reset();
    sampler.note_on(drum_map::KICK, 0.8);
    let kick_rr_0 = active_rr_indices(&sampler, 0);
    assert_eq!(kick_rr_0, vec![0]);

    sampler.reset();
    sampler.note_on(drum_map::KICK, 0.8);
    let kick_rr_1 = active_rr_indices(&sampler, 0);
    assert_eq!(kick_rr_1, vec![1]);

    // Hit Snare for the first time — its counter should still be at 0,
    // independent of the Kick counter.
    sampler.reset();
    sampler.note_on(drum_map::SNARE, 0.8);
    let snare_rr_0 = active_rr_indices(&sampler, 1);
    assert_eq!(
        snare_rr_0,
        vec![0],
        "snare rr should start at 0 independently"
    );
}

#[test]
fn different_velocity_layers_have_independent_rr_counters() {
    let mut sampler = make_test_sampler();
    // Pad 0 with 2 velocity layers, each with 3 round-robin takes.
    sampler.pads = drum_map::PAD_MAPPINGS
        .iter()
        .map(|m| make_test_pad(2, 3, m.output_group, m.choke_group))
        .collect();

    let note = drum_map::KICK;

    // Soft hit (velocity 0.1 -> layer 0). Hit twice.
    sampler.reset();
    sampler.note_on(note, 0.1);
    assert_eq!(active_rr_indices(&sampler, 0), vec![0]);

    sampler.reset();
    sampler.note_on(note, 0.1);
    assert_eq!(active_rr_indices(&sampler, 0), vec![1]);

    // Hard hit (velocity 0.9 -> layer 1). Its RR counter is independent.
    sampler.reset();
    sampler.note_on(note, 0.9);
    assert_eq!(
        active_rr_indices(&sampler, 0),
        vec![0],
        "hard layer rr should start at 0 independently of the soft layer"
    );
}

#[test]
fn kit_swap_resets_rr_counters() {
    let (tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut sampler = DrumSampler::new(rx);
    sampler.pads = drum_map::PAD_MAPPINGS
        .iter()
        .map(|m| make_test_pad(1, 3, m.output_group, m.choke_group))
        .collect();

    // Advance the Kick's RR counter.
    sampler.note_on(drum_map::KICK, 0.8); // rr 0
    sampler.note_on(drum_map::KICK, 0.8); // rr 1

    // Send a new kit through the channel and swap it in.
    let new_pads: Vec<LoadedPad> = drum_map::PAD_MAPPINGS
        .iter()
        .map(|m| make_test_pad(1, 3, m.output_group, m.choke_group))
        .collect();
    tx.send(new_pads).unwrap();
    sampler.try_swap_kit();

    // After the kit swap, the RR counter should be back to 0.
    sampler.note_on(drum_map::KICK, 0.8);
    let indices = active_rr_indices(&sampler, 0);
    assert_eq!(
        indices,
        vec![0],
        "rr counter should reset to 0 after kit swap"
    );
}

#[test]
fn all_voices_from_single_hit_share_rr_index() {
    let mut sampler = make_test_sampler();
    // Build a kick pad with 2 close-mic banks (KickIn + KickOut) and
    // an overhead, each with 4 round-robin takes.
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

    // Fill all pad slots; only pad 0 matters.
    sampler.pads = std::iter::once(kick_pad)
        .chain(
            drum_map::PAD_MAPPINGS[1..]
                .iter()
                .map(|m| make_test_pad(1, 1, m.output_group, m.choke_group)),
        )
        .collect();

    sampler.note_on(drum_map::KICK, 0.8);

    // Should have spawned 3 voices: KickIn, KickOut, OH.
    let active: Vec<_> = sampler
        .voices
        .iter()
        .filter(|v| v.active && v.pad_index == 0)
        .collect();
    assert_eq!(
        active.len(),
        3,
        "kick with 2 close + OH should spawn 3 voices"
    );

    // All three must share the same rr_index.
    let rr = active[0].rr_index;
    for v in &active {
        assert_eq!(
            v.rr_index, rr,
            "all voices from a single hit must share the same rr_index"
        );
    }
}
