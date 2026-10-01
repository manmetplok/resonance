//! The status bar and the inspector's SAMPLE stage must only display
//! measurements. These tests cover the three figures that used to be
//! invented: output level (dead OUT meter), sample memory (fake "RAM"),
//! and the pad's sample identity / waveform (ba todo #1276).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use resonance_drums::drum_map::{self, PAD_MAPPINGS};
use resonance_drums::dsp::{DrumSampler, PortBuffers};
use resonance_drums::kit::{LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer};
use resonance_drums::params::DrumParams;
use resonance_drums::sample_info::{self, ENVELOPE_BUCKETS};

const NUM_PORTS: usize = 7;

fn layer(value: f32, takes: usize) -> VelocityLayer {
    VelocityLayer::new((0..takes)
            .map(|_| LoadedSample::from_data(vec![value; 64]))
            .collect())
}

fn tom_pad(mapping_index: usize) -> LoadedPad {
    let m = &PAD_MAPPINGS[mapping_index];
    LoadedPad {
        name: m.name.to_string(),
        choke_group: m.choke_group,
        output_group: m.output_group,
        close_mics: vec![LoadedMicBank {
            position: "Tom01".to_string(),
            setup_key: "07_Tom01_md421".to_string(),
            layers: vec![layer(0.25, 1), layer(0.5, 3)],
        }],
        overhead: Some(LoadedMicBank {
            position: "OHsAB".to_string(),
            setup_key: "23_OHsAB_e914".to_string(),
            layers: vec![layer(0.125, 1)],
        }),
    }
}

fn silent_pad(mapping_index: usize) -> LoadedPad {
    let m = &PAD_MAPPINGS[mapping_index];
    LoadedPad {
        name: m.name.to_string(),
        choke_group: m.choke_group,
        output_group: m.output_group,
        close_mics: Vec::new(),
        overhead: None,
    }
}

fn loaded_sampler() -> (DrumSampler, Arc<[AtomicU32; 2]>) {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut sampler = DrumSampler::new(rx);
    let out_peak: Arc<[AtomicU32; 2]> = Arc::new(std::array::from_fn(|_| AtomicU32::new(0)));
    sampler.set_out_peak(out_peak.clone());
    sampler.pads = (0..PAD_MAPPINGS.len())
        .map(|i| if i == 9 { tom_pad(i) } else { silent_pad(i) })
        .collect();
    (sampler, out_peak)
}

/// Render one block and return the loudest absolute sample per channel
/// across every port, so the test can compare against what was published.
fn render_and_measure(sampler: &mut DrumSampler, frames: usize) -> [f32; 2] {
    let mut port_data: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_PORTS)
        .map(|_| (vec![0.0; frames], vec![0.0; frames]))
        .collect();
    {
        let mut ports: Vec<PortBuffers<'_>> = port_data
            .iter_mut()
            .map(|(l, r)| PortBuffers {
                left: l.as_mut_slice(),
                right: r.as_mut_slice(),
            })
            .collect();
        sampler.render_block(&mut ports, frames, &DrumParams::default(), &[]);
    }
    let mut peak = [0.0f32; 2];
    for (l, r) in &port_data {
        for s in l {
            peak[0] = peak[0].max(s.abs());
        }
        for s in r {
            peak[1] = peak[1].max(s.abs());
        }
    }
    peak
}

fn published(out_peak: &Arc<[AtomicU32; 2]>) -> [f32; 2] {
    [
        f32::from_bits(out_peak[0].load(Ordering::Relaxed)),
        f32::from_bits(out_peak[1].load(Ordering::Relaxed)),
    ]
}

/// The OUT meter reads the real block peak of the rendered output.
#[test]
fn out_peak_matches_the_rendered_block() {
    let (mut sampler, out_peak) = loaded_sampler();
    sampler.note_on(drum_map::TOM_HIGH, 1.0);

    let measured = render_and_measure(&mut sampler, 32);
    assert!(
        measured[0] > 0.0 && measured[1] > 0.0,
        "the test fixture should produce audio, got {measured:?}"
    );
    assert_eq!(
        published(&out_peak),
        measured,
        "the published OUT peak must be the block's actual peak"
    );
}

/// A block with nothing sounding publishes silence, so the meter falls
/// back to −∞ rather than freezing on the last hit.
#[test]
fn out_peak_returns_to_silence() {
    let (mut sampler, out_peak) = loaded_sampler();
    sampler.note_on(drum_map::TOM_HIGH, 1.0);
    render_and_measure(&mut sampler, 32);
    assert!(published(&out_peak)[0] > 0.0);

    // The fixture take is 32 frames long, so the next block is silent.
    let measured = render_and_measure(&mut sampler, 32);
    assert_eq!(measured, [0.0, 0.0], "fixture take should have ended");
    assert_eq!(published(&out_peak), [0.0, 0.0]);
}

/// The status bar's memory readout counts the decoded samples actually
/// held, not an invented RAM figure.
#[test]
fn sample_bytes_counts_every_take() {
    let (sampler, _peak) = loaded_sampler();
    // One tom pad: close mic 1 take + 3 takes, overhead 1 take, each 64
    // f32 samples; every other pad is empty.
    let expected = 5 * 64 * std::mem::size_of::<f32>();
    assert_eq!(sampler.total_sample_bytes(), expected);
    assert_eq!(sample_info::total_sample_bytes(&sampler.pads), expected);
}

#[test]
fn byte_formatting_is_readable() {
    assert_eq!(sample_info::format_bytes(0), "0 B");
    assert_eq!(sample_info::format_bytes(2048), "2 kB");
    assert_eq!(sample_info::format_bytes(3 * 1024 * 1024), "3.0 MB");
}

/// The inspector's sample identity describes the take a full-velocity hit
/// plays: the reference (close-mic) bank's loudest layer, first take.
#[test]
fn pad_sample_info_describes_the_full_velocity_take() {
    let pad = tom_pad(9);
    let info = sample_info::info_for_pad(&pad, 48_000.0).expect("pad has banks");

    assert_eq!(info.position, "Tom01");
    assert_eq!(info.setup_key, "07_Tom01_md421");
    assert_eq!(info.layer_count, 2);
    assert_eq!(info.layer_index, 1, "loudest layer is the last one");
    assert_eq!(info.take_count, 3);
    assert_eq!(info.frames, 32);
    assert_eq!(info.layer_text(), "layer 2/2 · take 1/3");
    assert_eq!(info.source_text(), "Tom01 · 07_Tom01_md421");
    assert_eq!(info.duration_text().as_deref(), Some("00:00.001"));
}

/// A pad with no close mic falls back to the overhead bank — the same
/// bank `note_on` uses to pick the layer and round robin.
#[test]
fn pad_sample_info_falls_back_to_overhead() {
    let mut pad = tom_pad(9);
    pad.close_mics.clear();
    let info = sample_info::info_for_pad(&pad, 48_000.0).expect("overhead bank");
    assert_eq!(info.position, "OHsAB");

    // No banks at all: no invented detail.
    assert!(sample_info::info_for_pad(&silent_pad(0), 48_000.0).is_none());
}

/// The waveform is the take's own min/max envelope.
#[test]
fn envelope_is_measured_from_the_take() {
    // A ramp from -1 to +1 across 240 frames, stereo interleaved.
    let frames = 240usize;
    let mut data = Vec::with_capacity(frames * 2);
    for f in 0..frames {
        let v = -1.0 + 2.0 * f as f32 / (frames - 1) as f32;
        data.push(v);
        data.push(v);
    }
    let env = sample_info::envelope(&data, frames);
    assert_eq!(env.len(), ENVELOPE_BUCKETS.min(frames));
    let (first_lo, _) = env[0];
    let (_, last_hi) = env[env.len() - 1];
    assert!((first_lo + 1.0).abs() < 1.0e-6, "first bucket floor: {first_lo}");
    assert!((last_hi - 1.0).abs() < 1.0e-6, "last bucket ceiling: {last_hi}");
    for (lo, hi) in &env {
        assert!(lo <= hi, "min/max inverted: {lo} > {hi}");
    }

    // Short takes yield one bucket per frame rather than padded zeroes.
    let short = sample_info::envelope(&[0.5, -0.5, 0.25, -0.25], 2);
    assert_eq!(short, vec![(-0.5, 0.5), (-0.25, 0.25)]);

    // Nothing decoded means no shape at all.
    assert!(sample_info::envelope(&[], 0).is_empty());
}
