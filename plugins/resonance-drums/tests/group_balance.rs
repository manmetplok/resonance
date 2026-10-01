//! Per-output-group level and routing regressions (ba todo #1232).
//!
//! Two things are pinned here:
//!
//! 1. **Nothing in the plugin attenuates a group.** The field report behind
//!    todo #1232 claimed the kit ships with Hats/Toms roughly 9 dB under
//!    Kick/Snare. It does not: `PadParams` is uniform across all 30 pads, so
//!    the only spread is what the samples themselves were recorded at. This
//!    measures the shipped fallback kit end to end and pins that spread.
//!
//! 2. **Where overhead takes play (E11).** Every cymbal, ride and china
//!    piece in Drummica is recorded on the overheads only
//!    (`close_mic_positions: &[]`): that take is the cymbal's whole sound,
//!    so in Multi (the headless sampler's routing) it plays on the pad's
//!    own port, Cymbals — its sub-track is not silent. A close-miked pad's
//!    overhead take goes to the Overhead port; Stereo puts the whole kit
//!    on Main.

use resonance_drums::drum_map::{self, PAD_MAPPINGS};
use resonance_drums::dsp::{DrumSampler, PortBuffers};
use resonance_drums::kit::{LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer};
use resonance_drums::params::DrumParams;

const SR: f32 = 48_000.0;
const NUM_PORTS: usize = 7;

const PORT_KICK: usize = 1;
const PORT_SNARE: usize = 2;
const PORT_TOMS: usize = 3;
const PORT_HATS: usize = 4;
const PORT_CYMBALS: usize = 5;
const PORT_OVERHEAD: usize = 6;

fn make_sampler() -> DrumSampler {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    DrumSampler::new(rx)
}

/// Render one note to completion and return the peak magnitude per port.
fn peaks_for_hit(sampler: &mut DrumSampler, note: u8, velocity: f32, blocks: usize) -> [f32; NUM_PORTS] {
    sampler.reset();
    sampler.note_on(note, velocity);

    let frames = 512;
    let params = DrumParams::default();
    let mut peaks = [0.0f32; NUM_PORTS];
    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_PORTS)
        .map(|_| (vec![0.0; frames], vec![0.0; frames]))
        .collect();
    for _ in 0..blocks {
        {
            let mut ports: Vec<PortBuffers<'_>> = bufs
                .iter_mut()
                .map(|(l, r)| PortBuffers {
                    left: l.as_mut_slice(),
                    right: r.as_mut_slice(),
                })
                .collect();
            sampler.render_block(&mut ports, frames, &params, &[]);
        }
        for (p, (l, r)) in bufs.iter().enumerate() {
            for (a, b) in l.iter().zip(r.iter()) {
                peaks[p] = peaks[p].max(a.abs()).max(b.abs());
            }
        }
    }
    peaks
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

// ---------------------------------------------------------------------------
// 1. Shipped level balance
// ---------------------------------------------------------------------------

/// The bundled fallback kit — what a freshly instantiated plugin plays
/// before any Drummica kit is loaded — puts every group within a normal
/// kit-voicing window of the loudest one. Hats and cymbals sitting a few dB
/// under the kick is how a drum kit is supposed to read; ~9 dB down (the
/// number in the field report) is not, and would fail this.
#[test]
fn bundled_kit_groups_are_within_a_few_db_of_each_other() {
    let mut sampler = make_sampler();
    sampler.load_defaults(SR);

    // One representative pad per group, all struck at the same velocity.
    let probes: [(&str, u8, usize); 5] = [
        ("Kick", drum_map::KICK, PORT_KICK),
        ("Snare", drum_map::SNARE, PORT_SNARE),
        ("Toms", drum_map::TOM_MID, PORT_TOMS),
        ("Hats", drum_map::HIHAT_CLOSED, PORT_HATS),
        ("Cymbals", drum_map::CRASH_16_EDGE, PORT_CYMBALS),
    ];

    let mut measured: Vec<(&str, f32)> = Vec::new();
    for (name, note, port) in probes {
        let peaks = peaks_for_hit(&mut sampler, note, 100.0 / 127.0, 200);
        assert!(
            peaks[port] > 0.0,
            "{name} group port {port} produced no audio at all"
        );
        measured.push((name, db(peaks[port])));
    }

    let loudest = measured
        .iter()
        .map(|(_, d)| *d)
        .fold(f32::NEG_INFINITY, f32::max);
    for (name, d) in &measured {
        assert!(
            loudest - d < 8.0,
            "{name} is {:.2} dB under the loudest group ({:?}) — the shipped \
             kit must not bury a group; a >=8 dB gap means a per-group \
             attenuation crept into the defaults",
            loudest - d,
            measured
        );
    }
}

/// Nothing in the param defaults singles a pad out: every pad ships with the
/// same volume / pan / per-mic trims. This is the cheap invariant behind
/// the measurement above.
#[test]
fn pad_param_defaults_are_uniform_across_all_pads() {
    let params = DrumParams::default();
    let first = &params.pads[0];
    for (i, pad) in params.pads.iter().enumerate() {
        assert_eq!(
            pad.volume.value(),
            first.volume.value(),
            "pad {i} ships a different default volume"
        );
        assert_eq!(pad.pan.value(), first.pan.value(), "pad {i} pan differs");
        for (slot, trim) in pad.trims.iter().enumerate() {
            assert_eq!(
                trim.value(),
                first.trims[slot].value(),
                "pad {i} mic trim {slot} differs"
            );
        }
        assert!(!pad.mute.value(), "pad {i} ships muted");
    }
}

// ---------------------------------------------------------------------------
// 2. Overhead-only pads still feed their own group port
// ---------------------------------------------------------------------------

fn layer(value: f32) -> VelocityLayer {
    VelocityLayer {
        round_robins: vec![LoadedSample::from_data(vec![value; 64])],
    }
}

/// Build a pad shaped the way the Drummica loader builds it: one close bank
/// per declared close-mic position (none at all for cymbals) plus a shared
/// overhead bank.
fn drummica_shaped_pad(index: usize) -> LoadedPad {
    let m = &PAD_MAPPINGS[index];
    LoadedPad {
        name: m.name.to_string(),
        choke_group: m.choke_group,
        output_group: m.output_group,
        close_mics: m
            .close_mic_positions
            .iter()
            .map(|pos| LoadedMicBank {
                position: pos.to_string(),
                setup_key: String::new(),
                layers: vec![layer(0.5)],
            })
            .collect(),
        overhead: Some(LoadedMicBank {
            position: "OHsAB".to_string(),
            setup_key: String::new(),
            layers: vec![layer(0.25)],
        }),
    }
}

fn drummica_shaped_sampler() -> DrumSampler {
    let mut sampler = make_sampler();
    sampler.pads = (0..PAD_MAPPINGS.len()).map(drummica_shaped_pad).collect();
    sampler
}

/// E11 (drums-plugin-rework.md §7): in Multi, an overhead-only cymbal's
/// take — its whole sound — plays on its own port (Cymbals), not on
/// Overhead, so the Cymbals sub-track is not silent (ba #1232).
#[test]
fn cymbal_overheads_land_on_the_cymbals_port_in_multi() {
    let mut sampler = drummica_shaped_sampler();

    for note in [
        drum_map::CRASH_16_EDGE,
        drum_map::CRASH_18_EDGE,
        drum_map::RIDE_EDGE,
        drum_map::RIDE_BELL,
        drum_map::CHINA_EDGE,
    ] {
        let peaks = peaks_for_hit(&mut sampler, note, 0.8, 2);
        assert!(
            peaks[PORT_CYMBALS] > 0.0,
            "note {note}: the cymbal's overhead take must reach the Cymbals port"
        );
        assert_eq!(
            peaks[PORT_OVERHEAD], 0.0,
            "note {note}: an overhead-only cymbal does not double into the Overhead port"
        );
    }
}

/// Close-miked pads play their close banks on their group port and their
/// overhead take on the shared Overhead port (E11).
#[test]
fn close_miked_pads_still_send_their_overhead_to_the_overhead_port() {
    let mut sampler = drummica_shaped_sampler();

    for (note, group_port) in [
        (drum_map::KICK, PORT_KICK),
        (drum_map::SNARE, PORT_SNARE),
        (drum_map::TOM_MID, PORT_TOMS),
        (drum_map::HIHAT_CLOSED, PORT_HATS),
    ] {
        let peaks = peaks_for_hit(&mut sampler, note, 0.8, 2);
        assert!(
            peaks[group_port] > 0.0,
            "note {note}: group port {group_port} lost its close mic"
        );
        assert!(
            peaks[PORT_OVERHEAD] > 0.0,
            "note {note}: overhead take should still reach the Overhead port"
        );
    }
}

/// Every declared output group is reachable in Multi: a port no kit can
/// ever feed shows up in the host as a permanently dead sub-track. A
/// Drummica-shaped kit feeds every one — its overhead-only cymbals play on
/// Cymbals (E11) — and so does the bundled kit, but for Overhead (it has
/// no overhead bank).
#[test]
fn every_group_port_is_reachable() {
    let reach = |sampler: &mut DrumSampler| {
        let mut reached = [false; NUM_PORTS];
        for m in PAD_MAPPINGS.iter() {
            let peaks = peaks_for_hit(sampler, m.note, 0.8, 2);
            for (p, peak) in peaks.iter().enumerate() {
                if *peak > 0.0 {
                    reached[p] = true;
                }
            }
        }
        reached
    };
    let drummica = reach(&mut drummica_shaped_sampler());
    assert!(
        drummica[PORT_CYMBALS],
        "Drummica's overhead-only cymbals play on Cymbals"
    );
    let mut bundled = make_sampler();
    bundled.load_defaults(SR);
    let bundled = reach(&mut bundled);
    let reached: Vec<bool> = drummica.iter().zip(&bundled).map(|(a, b)| *a || *b).collect();

    for port in [
        PORT_KICK,
        PORT_SNARE,
        PORT_TOMS,
        PORT_HATS,
        PORT_CYMBALS,
        PORT_OVERHEAD,
    ] {
        assert!(reached[port], "no pad ever writes to output port {port}");
    }
}
