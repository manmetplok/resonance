//! Click-free voice stealing (drums-plugin-rework.md §7 E1).
//!
//! With every voice busy, a new hit used to overwrite the stolen voice in
//! place: whatever that voice was playing stopped dead between two
//! samples, which is a click on every steal. A stolen voice now moves to
//! a tail slot and fades out over `STEAL_FADE_MS` while the new hit
//! starts in the slot it left.
//!
//! The kit here makes clicks measurable. Every pad plays a long sample
//! that ramps up linearly over `ATTACK` frames and then holds a constant
//! level, so the rendered mix is smooth except where a voice starts or
//! stops — and the only stops in this pattern are steals. The largest
//! sample-to-sample step in the mix is then exactly the click size.

use std::f32::consts::FRAC_PI_2;

use resonance_drums::drum_map::{NUM_PADS, PAD_MAPPINGS};
use resonance_drums::dsp::{DrumSampler, Hit, PortBuffers};
use resonance_drums::kit::{
    LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer, NUM_OUTPUT_PORTS,
};
use resonance_drums::params::DrumParams;
use resonance_drums::voice::{fade_frames, MAX_VOICES, STEAL_FADE_MS, TAIL_SLOTS};

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;
/// Level each voice holds once its attack is over.
const LEVEL: f32 = 0.05;
/// Attack ramp length in frames: a voice's own onset rises by at most
/// `LEVEL / ATTACK` per sample.
const ATTACK: usize = 64;
/// One hit every this many frames.
const SPACING: usize = 16;
/// Hits in the pattern: the first `MAX_VOICES` fill every slot, the rest
/// each steal one.
const HITS: usize = MAX_VOICES + 192;

fn ramp_sample(frames: usize) -> LoadedSample {
    let mut data = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let v = LEVEL * (i.min(ATTACK) as f32 / ATTACK as f32);
        data.push(v);
        data.push(v);
    }
    LoadedSample::from_data(data)
}

/// Every pad: one close mic, one layer, one take, no overhead and no
/// choke group, so each hit is exactly one voice and nothing but a steal
/// ever ends one early.
fn ramp_pads() -> Vec<LoadedPad> {
    // Longer than the whole render, so no voice ends on its own.
    let frames = HITS * SPACING + 8 * BLOCK;
    PAD_MAPPINGS
        .iter()
        .map(|m| LoadedPad {
            name: m.name.to_string(),
            choke_group: None,
            output_group: m.output_group,
            close_mics: vec![LoadedMicBank {
                position: "test".to_string(),
                setup_key: String::new(),
                layers: vec![VelocityLayer::new(vec![ramp_sample(frames)])],
            }],
            extra_banks: Vec::new(),
            overhead: None,
        })
        .collect()
}

/// Render the saturation pattern and return the left channel of the
/// kit mix (every port summed), plus how many voices were active when
/// the first steal happened.
fn render_pattern() -> (Vec<f32>, usize) {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut sampler = DrumSampler::new(rx);
    sampler.set_sample_rate(SR);
    sampler.pads = ramp_pads();
    let params = DrumParams::default();
    // Unity everywhere, so the mix is the voices' own signal.
    params.master_volume.set_value(0.0); // 0 dB: unity
    for pad in &params.pads {
        pad.volume.set_value(0.0); // 0 dB: unity
        // No choke groups (the kit has none; the hats default to one).
        pad.choke.set_value(0);
    }
    sampler.update_global_settings(&params);

    let notes: Vec<u8> = PAD_MAPPINGS.iter().map(|m| m.note).collect();
    assert_eq!(notes.len(), NUM_PADS);
    let all: Vec<(usize, u8)> = (0..HITS)
        .map(|i| (i * SPACING, notes[i % NUM_PADS]))
        .collect();

    let total = HITS * SPACING + 4 * BLOCK;
    let blocks = total.div_ceil(BLOCK);
    let mut mix = Vec::with_capacity(blocks * BLOCK);
    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]))
        .collect();
    let mut active_at_saturation = 0;
    for b in 0..blocks {
        let start = b * BLOCK;
        let hits: Vec<Hit> = all
            .iter()
            .filter(|(at, _)| (start..start + BLOCK).contains(at))
            .map(|&(at, note)| Hit {
                frame: at - start,
                note,
                velocity: 1.0,
            })
            .collect();
        {
            let mut ports: Vec<PortBuffers<'_>> = bufs
                .iter_mut()
                .map(|(l, r)| PortBuffers {
                    left: l.as_mut_slice(),
                    right: r.as_mut_slice(),
                })
                .collect();
            sampler.render_block(&mut ports, BLOCK, &params, &hits);
        }
        if start <= MAX_VOICES * SPACING && MAX_VOICES * SPACING < start + BLOCK {
            active_at_saturation = sampler.voices.iter().filter(|v| v.active).count();
        }
        for i in 0..BLOCK {
            mix.push(bufs.iter().map(|(l, _)| l[i]).sum());
        }
    }
    (mix, active_at_saturation)
}

fn max_step(signal: &[f32]) -> (f32, usize) {
    signal
        .windows(2)
        .enumerate()
        .map(|(i, w)| ((w[1] - w[0]).abs(), i + 1))
        .fold((0.0, 0), |acc, x| if x.0 > acc.0 { x } else { acc })
}

#[test]
fn stealing_a_sounding_voice_does_not_click() {
    let (mix, active) = render_pattern();
    assert_eq!(active, MAX_VOICES, "the pattern must saturate every voice");
    assert!(
        mix.iter().any(|s| s.abs() > LEVEL),
        "the pattern rendered (near) silence"
    );

    // The bound, from the only two things that move the mix:
    //
    // - onsets: each voice's attack rises by LEVEL / ATTACK per sample,
    //   and with a hit every SPACING frames at most ATTACK / SPACING of
    //   them overlap;
    // - steal fades: an equal-power fade `cos(t·π/2)` over N frames falls
    //   by at most LEVEL · π / (2N) per sample, and at most N / SPACING
    //   (rounded up) fade at once.
    //
    // Summed, that is about 0.16 × LEVEL at 48 kHz. A hard cut drops a
    // held voice by LEVEL in one sample, so the old code measured ≈ 1.0
    // × LEVEL here; the 1.25 margin only absorbs float rounding.
    let n = fade_frames(STEAL_FADE_MS, SR) as f32;
    let onsets = (ATTACK / SPACING) as f32 * LEVEL / ATTACK as f32;
    let fades = (n / SPACING as f32).ceil() * LEVEL * FRAC_PI_2 / n;
    let bound = 1.25 * (onsets + fades);
    assert!(
        bound < 0.5 * LEVEL,
        "bound {bound} would not tell a click apart"
    );

    let (step, at) = max_step(&mix);
    assert!(
        step <= bound,
        "voice steal clicks: a {step} step at frame {at} (bound {bound}, a hard cut \
         is {LEVEL})"
    );
}

/// The stolen voice really fades rather than vanishing: right after the
/// first steal there are more voices sounding than slots, and the extra
/// ones are gone again a steal-fade later.
#[test]
fn a_stolen_voice_fades_out_in_a_tail_slot() {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut sampler = DrumSampler::new(rx);
    sampler.set_sample_rate(SR);
    sampler.pads = ramp_pads();
    let params = DrumParams::default();
    sampler.update_global_settings(&params);
    let note = PAD_MAPPINGS[0].note;
    for _ in 0..MAX_VOICES {
        sampler.note_on(note, 1.0);
    }
    assert_eq!(sampler.tail_voices_active(), 0);
    sampler.note_on(note, 1.0);
    assert_eq!(
        sampler.voices.iter().filter(|v| v.active).count(),
        MAX_VOICES
    );
    assert_eq!(
        sampler.tail_voices_active(),
        1,
        "the victim must keep sounding"
    );

    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]))
        .collect();
    let mut ports: Vec<PortBuffers<'_>> = bufs
        .iter_mut()
        .map(|(l, r)| PortBuffers {
            left: l.as_mut_slice(),
            right: r.as_mut_slice(),
        })
        .collect();
    assert!(fade_frames(STEAL_FADE_MS, SR) as usize <= BLOCK);
    sampler.render_block(&mut ports, BLOCK, &params, &[]);
    assert_eq!(
        sampler.tail_voices_active(),
        0,
        "the tail must be gone after one steal fade"
    );
}

/// Render `frames` frames (at most `BLOCK`) and return the left channel
/// of every port summed.
fn render_frames(
    sampler: &mut DrumSampler,
    bufs: &mut [(Vec<f32>, Vec<f32>)],
    params: &DrumParams,
    frames: usize,
    hits: &[Hit],
) -> Vec<f32> {
    {
        let mut ports: Vec<PortBuffers<'_>> = bufs
            .iter_mut()
            .map(|(l, r)| PortBuffers {
                left: l.as_mut_slice(),
                right: r.as_mut_slice(),
            })
            .collect();
        sampler.render_block(&mut ports, frames, params, hits);
    }
    (0..frames)
        .map(|i| bufs.iter().map(|(l, _)| l[i]).sum())
        .collect()
}

/// A burst of hits on one frame with a low polyphony ceiling: every hit
/// past the ceiling is a steal, all in the same frame. The sounding
/// victims need a tail each; the old overflow rule (fewest fade frames
/// left, which ties for every tail filled on the same frame) reused tail
/// 0 again and again, hard-cutting a full-level voice.
#[test]
fn a_burst_of_steals_on_one_frame_does_not_hard_cut_a_sounding_voice() {
    const POLY: usize = 4;
    const BURST: usize = 20;
    const { assert!(BURST > 16, "the burst must overflow the old 16 tails") };

    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut sampler = DrumSampler::new(rx);
    sampler.set_sample_rate(SR);
    sampler.pads = ramp_pads();
    let params = DrumParams::default();
    params.master_volume.set_value(0.0); // 0 dB: unity
    for pad in &params.pads {
        pad.volume.set_value(0.0); // 0 dB: unity
        // No choke groups (the kit has none; the hats default to one).
        pad.choke.set_value(0);
    }
    params.polyphony.set_value(POLY as i32);
    sampler.update_global_settings(&params);
    let notes: Vec<u8> = PAD_MAPPINGS.iter().map(|m| m.note).collect();

    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]))
        .collect();
    // POLY voices sounding at their held level.
    let first: Vec<Hit> = (0..POLY)
        .map(|i| Hit {
            frame: 0,
            note: notes[i],
            velocity: 1.0,
        })
        .collect();
    let mut mix = render_frames(&mut sampler, &mut bufs, &params, BLOCK, &first);
    let held = *mix.last().unwrap();
    assert!(
        (held - POLY as f32 * LEVEL).abs() < 1e-4,
        "the {POLY} voices must be sounding at their held level, mix = {held}"
    );

    // The burst, on one frame, every hit on a pad of its own.
    let burst: Vec<Hit> = (0..BURST)
        .map(|i| Hit {
            frame: 0,
            note: notes[POLY + i % (NUM_PADS - POLY)],
            velocity: 1.0,
        })
        .collect();
    mix.extend(render_frames(
        &mut sampler,
        &mut bufs,
        &params,
        BLOCK,
        &burst,
    ));

    // The only things moving the mix: POLY steal fades of the sounding
    // voices, and BURST onsets — every hit of the burst starts, the ones
    // stolen on the same frame inside their own steal fade.
    let n = fade_frames(STEAL_FADE_MS, SR) as f32;
    let fades = POLY as f32 * LEVEL * FRAC_PI_2 / n;
    let onsets = BURST as f32 * LEVEL / ATTACK as f32;
    let bound = 1.25 * (fades + onsets);
    assert!(
        bound < 0.5 * LEVEL,
        "bound {bound} would not tell a cut apart"
    );
    let (step, at) = max_step(&mix);
    assert!(
        step <= bound,
        "a {step} step at frame {at} (bound {bound}; a hard cut of one voice is {LEVEL})"
    );
}

/// Overflowing the tails reuses the quietest one. Fill every tail with a
/// sounding victim, let them fade a little, then steal one more sounding
/// voice: the tail it takes must be the one furthest into its fade.
#[test]
fn overflowing_the_tails_reuses_the_quietest() {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut sampler = DrumSampler::new(rx);
    sampler.set_sample_rate(SR);
    sampler.pads = ramp_pads();
    let params = DrumParams::default();
    params.polyphony.set_value(1);
    sampler.update_global_settings(&params);
    let note = PAD_MAPPINGS[0].note;
    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]))
        .collect();
    // One sounding voice stolen per frame: TAIL_SLOTS + 1 victims, each
    // a frame further into its fade than the next.
    sampler.note_on(note, 1.0);
    for _ in 0..=TAIL_SLOTS {
        render_frames(&mut sampler, &mut bufs, &params, 1, &[]);
        sampler.note_on(note, 1.0);
    }
    assert!(TAIL_SLOTS + 1 < fade_frames(STEAL_FADE_MS, SR) as usize);
    assert_eq!(sampler.tail_voices_active(), TAIL_SLOTS);
    // Victim k (0-based) was stolen after sounding one frame and has
    // faded TAIL_SLOTS - k frames since, so its read position is
    // TAIL_SLOTS + 1 - k. The one reused must be victim 0 — the furthest
    // into its fade, i.e. the quietest — leaving positions 1..=TAIL_SLOTS.
    let mut positions: Vec<usize> = sampler
        .tail_voices()
        .iter()
        .filter(|v| v.active)
        .map(|v| v.position)
        .collect();
    positions.sort_unstable();
    let want: Vec<usize> = (1..=TAIL_SLOTS).collect();
    assert_eq!(positions, want, "the quietest tail was not the one reused");
}

/// "Quietest" counts the AHD envelope (E8): a tail whose decay has all
/// but ended is reused before one that is less far into its steal fade
/// but still ringing at full level. Here the snare (decay 5 ms) is
/// stolen into a tail when its envelope is nearly done; by fade alone it
/// is the loudest tail, by what it plays the quietest.
#[test]
fn overflowing_the_tails_counts_the_decay_envelope() {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut sampler = DrumSampler::new(rx);
    sampler.set_sample_rate(SR);
    sampler.pads = ramp_pads();
    let params = DrumParams::default();
    params.polyphony.set_value(2);
    let (kick, snare) = (PAD_MAPPINGS[0].note, PAD_MAPPINGS[1].note);
    params.pads[1].hold.set_value(0.0);
    params.pads[1].decay.set_value(5.0); // 240 frames
    sampler.update_global_settings(&params);
    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]))
        .collect();
    let mut run = |s: &mut DrumSampler, frames: usize| {
        render_frames(s, &mut bufs, &params, frames, &[]);
    };

    sampler.note_on(snare, 1.0);
    run(&mut sampler, 180);
    // Kick hits two frames apart, each stealing the last: 16 kick tails.
    sampler.note_on(kick, 1.0);
    for _ in 0..16 {
        run(&mut sampler, 2);
        sampler.note_on(kick, 1.0);
    }
    // The snare stolen by a snare: into a tail, its envelope nearly done.
    sampler.note_on(snare, 1.0);
    // Fill the other tails.
    for _ in 0..TAIL_SLOTS - 17 {
        run(&mut sampler, 1);
        sampler.note_on(kick, 1.0);
    }
    run(&mut sampler, 1);
    assert_eq!(sampler.tail_voices_active(), TAIL_SLOTS);
    let snare_tail = |s: &DrumSampler| {
        s.tail_voices()
            .iter()
            .find(|v| v.active && v.pad_index == 1)
            .copied()
    };
    let tail = snare_tail(&sampler).expect("the snare is in a tail");
    let fade_only = sampler
        .tail_voices()
        .iter()
        .filter(|v| v.active)
        .map(|v| v.current_gain())
        .fold(f32::INFINITY, f32::min);
    assert!(
        tail.current_gain() > fade_only,
        "by fade alone the snare tail is not the quietest"
    );
    assert!(tail.audible_gain() < 0.01, "its envelope is nearly done");

    // One more steal: the snare's tail is the one reused.
    sampler.note_on(kick, 1.0);
    assert!(
        snare_tail(&sampler).is_none(),
        "the tail with its envelope all but done was not the one reused"
    );
}

/// CLAP `reset` kills the tails too: nothing fades into the next render.
#[test]
fn reset_clears_the_tails() {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut sampler = DrumSampler::new(rx);
    sampler.set_sample_rate(SR);
    sampler.pads = ramp_pads();
    let params = DrumParams::default();
    params.polyphony.set_value(1);
    sampler.update_global_settings(&params);
    let note = PAD_MAPPINGS[0].note;
    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]))
        .collect();
    sampler.note_on(note, 1.0);
    render_frames(&mut sampler, &mut bufs, &params, 1, &[]);
    sampler.note_on(note, 1.0);
    assert_eq!(sampler.tail_voices_active(), 1);
    sampler.reset();
    assert_eq!(sampler.tail_voices_active(), 0);
    let out = render_frames(&mut sampler, &mut bufs, &params, BLOCK, &[]);
    assert!(out.iter().all(|s| *s == 0.0), "a tail rang on after reset");
}
