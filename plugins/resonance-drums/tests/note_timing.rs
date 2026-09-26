//! Note events land at their sample offset inside the block (DSP-01).
//!
//! `process()` used to drain every event before rendering and start each
//! new voice at frame 0, so a hit fired up to one block early and two
//! hits on the same pad in one block merged into one louder hit. These
//! tests drive the plugin through `process()` with timed events and
//! compare against a hit at offset 0, which is the reference shape.

use resonance_drums::drum_map;
use resonance_drums::kit::NUM_OUTPUT_PORTS;
use resonance_drums::ResonanceDrums;
use resonance_plugin::{EventIterator, NoteEvent, OutputBuffer, ResonancePlugin};

const SAMPLE_RATE: f32 = 48_000.0;
const BLOCK: usize = 1024;
/// Pad 0 (the kick) routes its close mic to output port 1.
const KICK_PORT: usize = 1;
/// The hi-hats (open and closed share a choke group) route to port 4.
const HATS_PORT: usize = 4;

fn booted_plugin() -> ResonanceDrums {
    let mut plugin = ResonanceDrums::new();
    assert!(plugin.initialize(SAMPLE_RATE, BLOCK as u32));
    plugin
}

fn on(note: u8, timing: u32) -> NoteEvent {
    NoteEvent::NoteOn {
        note,
        velocity: 1.0,
        timing,
    }
}

/// Render `blocks` blocks, feeding `events[i]` into block `i`, and return
/// each port's left channel concatenated over all blocks.
///
/// One silent block is rendered first and dropped: the plugin's very
/// first block ramps the master volume in from unity, which would make a
/// hit at frame 0 and the same hit at frame 300 differ for a reason that
/// has nothing to do with timing.
fn render(events: &[Vec<NoteEvent>], blocks: usize) -> Vec<Vec<f32>> {
    let mut plugin = booted_plugin();
    let mut out: Vec<Vec<f32>> = vec![Vec::new(); NUM_OUTPUT_PORTS];
    for block in 0..=blocks {
        let block_events: &[NoteEvent] = match block {
            0 => &[],
            _ => events.get(block - 1).map_or(&[], |v| v.as_slice()),
        };
        let mut buffers: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
            .map(|_| (vec![0.0; BLOCK], vec![0.0; BLOCK]))
            .collect();
        {
            let mut ports: Vec<OutputBuffer<'_>> = buffers
                .iter_mut()
                .map(|(l, r)| OutputBuffer {
                    left: l.as_mut_slice(),
                    right: r.as_mut_slice(),
                })
                .collect();
            let mut iter = EventIterator::new(block_events);
            plugin.process(&mut ports, BLOCK, &mut iter, None);
        }
        if block == 0 {
            continue;
        }
        for (port, (l, _)) in buffers.into_iter().enumerate() {
            out[port].extend_from_slice(&l);
        }
    }
    out
}

fn first_non_zero(signal: &[f32]) -> Option<usize> {
    signal.iter().position(|s| *s != 0.0)
}

/// Bit-identical comparison that names the first differing frame instead
/// of dumping both signals.
fn assert_same(got: &[f32], want: &[f32], what: &str) {
    assert_eq!(got.len(), want.len());
    if let Some(i) = got.iter().zip(want).position(|(a, b)| a.to_bits() != b.to_bits()) {
        panic!("{what}: first difference at frame {i}: {} vs {}", got[i], want[i]);
    }
}

fn assert_sounds(signal: &[f32], what: &str) {
    assert!(
        signal.iter().any(|s| *s != 0.0),
        "{what}: rendered silence, so the comparison would be vacuous"
    );
}

#[test]
fn a_hit_starts_at_its_offset() {
    let reference = render(&[vec![on(drum_map::KICK, 0)]], 2);
    let offset = render(&[vec![on(drum_map::KICK, 300)]], 2);
    let reference = &reference[KICK_PORT];
    let offset = &offset[KICK_PORT];
    assert_sounds(reference, "kick at 0");
    assert_sounds(offset, "kick at 300");

    let onset = first_non_zero(offset).expect("non-silent");
    assert!(onset >= 300, "kick timed at 300 sounded at frame {onset}");
    assert!(
        offset[..300].iter().all(|s| *s == 0.0),
        "nothing may sound before the event's offset"
    );
    // Same hit, shifted by 300 frames, across the block boundary too.
    assert_same(
        &offset[300..],
        &reference[..2 * BLOCK - 300],
        "the hit started at 300 is not the same hit shifted",
    );
}

#[test]
fn two_hits_on_one_pad_in_one_block_are_two_onsets() {
    let single = render(&[vec![on(drum_map::KICK, 0)]], 2);
    let double = render(&[vec![on(drum_map::KICK, 0), on(drum_map::KICK, 512)]], 2);
    let single = &single[KICK_PORT];
    let double = &double[KICK_PORT];
    assert_sounds(single, "single kick");

    // Before the second hit the output is exactly the first hit alone.
    assert_same(&double[..512], &single[..512], "second hit sounded early");
    // From 512 on it is the first hit plus a second copy starting at 512.
    for i in 0..(2 * BLOCK - 512) {
        let want = single[512 + i] + single[i];
        assert!(
            (double[512 + i] - want).abs() <= 1e-6,
            "frame {}: got {}, want {want} (two onsets summed)",
            512 + i,
            double[512 + i]
        );
    }
}

#[test]
fn a_choke_takes_effect_at_its_offset() {
    let free = render(&[vec![on(drum_map::KICK, 0)]], 3);
    let choked = render(
        &[vec![
            on(drum_map::KICK, 0),
            NoteEvent::Choke {
                note: drum_map::KICK,
                timing: 512,
            },
        ]],
        3,
    );
    let free = &free[KICK_PORT];
    let choked = &choked[KICK_PORT];
    assert_sounds(free, "free kick");
    assert_same(&choked[..512], &free[..512], "choke applied before its offset");
    assert!(
        choked[512..].iter().zip(&free[512..]).any(|(a, b)| a != b),
        "the choke never took effect"
    );
    // The release fade (1024 frames) ends a fade-length after the choke.
    let tail = 512 + resonance_drums::voice::RELEASE_SAMPLES;
    assert!(
        choked[tail..].iter().all(|s| *s == 0.0),
        "choked voice still sounds after its fade"
    );
}

#[test]
fn a_choke_group_cut_takes_effect_at_the_closing_hit() {
    let open_only = render(&[vec![on(drum_map::HIHAT_OPEN, 0)]], 2);
    let cut = render(
        &[vec![
            on(drum_map::HIHAT_OPEN, 0),
            on(drum_map::HIHAT_CLOSED, 600),
        ]],
        2,
    );
    let open_only = &open_only[HATS_PORT];
    let cut = &cut[HATS_PORT];
    assert_sounds(open_only, "open hat");
    assert_same(
        &cut[..600],
        &open_only[..600],
        "the closed hat choked the open one before it was played",
    );
}

#[test]
fn out_of_range_and_unsorted_offsets_are_clamped() {
    // An offset past the block end is clamped into the block; an event
    // earlier than one already applied is applied at the current cursor.
    let out = render(
        &[vec![
            on(drum_map::KICK, 700),
            on(drum_map::SNARE, 100),
            on(drum_map::KICK, 50_000),
        ]],
        2,
    );
    let kick = &out[KICK_PORT];
    assert_sounds(kick, "kick");
    let onset = first_non_zero(kick).expect("non-silent");
    assert!(onset >= 700, "kick timed at 700 sounded at {onset}");
    assert!(onset < BLOCK, "clamped hits must still sound in their block");
}
