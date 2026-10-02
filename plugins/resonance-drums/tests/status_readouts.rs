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
        extra_banks: Vec::new(),
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
        extra_banks: Vec::new(),
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

// ---------------------------------------------------------------------------
// K5: the last hit, per-port meters, every take's waveform
// ---------------------------------------------------------------------------

use resonance_drums::last_hit::{self, LastHit, LastHits};

/// A packed hit reads back as itself; the sequence number tells two
/// identical hits apart, and 0 means "no hit yet".
#[test]
fn a_last_hit_round_trips() {
    let hit = LastHit {
        pad: 29,
        velocity: 98,
        layer: 4,
        layers: 7,
        take: 1,
        takes: 3,
        seq: 513,
    };
    assert_eq!(last_hit::unpack(last_hit::pack(&hit)), Some(hit));
    assert_eq!(last_hit::unpack(0), None);
    assert_eq!(hit.cell_text(), "layer 5/7 · take 2/3");
    let again = LastHit { seq: 514, ..hit };
    assert_ne!(last_hit::pack(&hit), last_hit::pack(&again));
}

/// Each hit the sampler plays is published: its pad, the velocity it
/// struck with, and the layer and take it played — a new sequence number
/// each time, even for the same cell.
#[test]
fn the_sampler_publishes_every_hit() {
    let (mut sampler, _peak) = loaded_sampler();
    let hits = Arc::new(LastHits::default());
    sampler.set_last_hits(hits.clone());
    assert!(hits.latest().is_none());

    sampler.note_on(drum_map::TOM_HIGH, 1.0);
    let first = hits.pad(9).expect("the tom hit is published");
    assert_eq!(hits.latest(), Some(first));
    assert_eq!((first.pad, first.velocity), (9, 127));
    assert_eq!((first.layer, first.layers), (1, 2), "the loud layer");
    assert_eq!(first.takes, 3);

    sampler.note_on(drum_map::TOM_HIGH, 1.0);
    let second = hits.pad(9).unwrap();
    assert_ne!(second.seq, first.seq, "a second hit must read as new");
    assert_eq!(second.take, (first.take + 1) % 3, "the takes cycle");

    // A note on a pad with nothing loaded plays nothing, and publishes
    // nothing.
    sampler.note_on(PAD_MAPPINGS[0].note, 1.0);
    assert!(hits.pad(0).is_none());
    assert_eq!(hits.latest(), Some(second));
}

/// Each port's meter reads that port's block peak: the tom's close mic on
/// its own port, its overhead on Overhead, silence elsewhere.
#[test]
fn port_peaks_match_each_port() {
    let (mut sampler, _peak) = loaded_sampler();
    let ports: Arc<[AtomicU32; NUM_PORTS]> = Arc::new(std::array::from_fn(|_| AtomicU32::new(0)));
    sampler.set_port_peak(ports.clone());
    sampler.note_on(drum_map::TOM_HIGH, 1.0);

    let frames = 32;
    let mut port_data: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_PORTS)
        .map(|_| (vec![0.0; frames], vec![0.0; frames]))
        .collect();
    {
        let mut bufs: Vec<PortBuffers<'_>> = port_data
            .iter_mut()
            .map(|(l, r)| PortBuffers {
                left: l.as_mut_slice(),
                right: r.as_mut_slice(),
            })
            .collect();
        let params = DrumParams::default();
        params
            .output_mode
            .set_value(resonance_drums::params::OUTPUT_MODE_MULTI);
        sampler.render_block(&mut bufs, frames, &params, &[]);
    }
    let mut sounding = 0;
    for (port, (l, r)) in port_data.iter().enumerate() {
        let want = l.iter().chain(r).fold(0.0f32, |m, s| m.max(s.abs()));
        let got = f32::from_bits(ports[port].load(Ordering::Relaxed));
        assert_eq!(got, want, "port {port}");
        if want > 0.0 {
            sounding += 1;
        }
    }
    assert_eq!(sounding, 2, "the tom's close port and Overhead");
}

/// The inspector can draw any take a hit played: the info carries every
/// take of the reference bank, and which banks the pad holds.
#[test]
fn pad_sample_info_carries_every_take_and_the_banks() {
    let pad = tom_pad(9);
    let info = sample_info::info_for_pad(&pad, 48_000.0).unwrap();
    assert_eq!(info.takes.len(), 2, "one entry per layer");
    assert_eq!(info.takes[0].len(), 1);
    assert_eq!(info.takes[1].len(), 3);
    let loud = info.take(1, 2).expect("layer 2, take 3");
    assert_eq!(loud.frames, 32, "64 interleaved stereo samples");
    assert!(loud.envelope.iter().all(|&(lo, hi)| lo == 0.5 && hi == 0.5));
    assert!(info.take(2, 0).is_none() && info.take(0, 1).is_none());
    assert_eq!(
        info.banks.close,
        [("Tom01".to_string(), "07_Tom01_md421".to_string())]
    );
    assert!(info.banks.overhead && !info.banks.bleed && !info.banks.room);
}

// ---------------------------------------------------------------------------
// fix6: E15 lead banks, streamed heads, shared envelopes
// ---------------------------------------------------------------------------

/// A pad whose only bank is an overhead slot layered on slot 1 (E15) —
/// what `note_on` leads with when slot 1 plays nothing on it — has an
/// info, and its banks say it holds overheads (so the inspector offers
/// MICS for it).
#[test]
fn a_pad_led_by_an_extra_overhead_slot_has_an_info() {
    use resonance_drums::kit::{BankKind, ExtraBank};
    let mut pad = silent_pad(12);
    pad.extra_banks.push(ExtraBank {
        kind: BankKind::Overhead { slot: 1 },
        bank: LoadedMicBank {
            position: "OHsXY".to_string(),
            setup_key: "25_OHsXY_USM69i".to_string(),
            layers: vec![layer(0.5, 2)],
        },
    });
    let info = sample_info::info_for_pad(&pad, 48_000.0).expect("the slot-2 overhead leads");
    assert_eq!(info.setup_key, "25_OHsXY_USM69i");
    assert!(info.banks.overhead);
    // Bleed or room alone never lead.
    let mut bleed_only = silent_pad(12);
    bleed_only.extra_banks.push(ExtraBank {
        kind: BankKind::Bleed,
        bank: LoadedMicBank {
            position: "SNBtm".to_string(),
            setup_key: "09_SNBtm_e906".to_string(),
            layers: vec![layer(0.5, 1)],
        },
    });
    assert!(sample_info::info_for_pad(&bleed_only, 48_000.0).is_none());
}

/// A mono 16-bit WAV of `frames` frames, every sample `value`.
fn wav_bytes(frames: usize, value: i16) -> Vec<u8> {
    let data_len = frames * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&48_000u32.to_le_bytes());
    out.extend_from_slice(&96_000u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for _ in 0..frames {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

/// A streamed take keeps only its head in memory: the info says how much
/// of the take the envelope covers, so the inspector draws the head over
/// its share of the width rather than stretched across all of it.
#[test]
fn a_streamed_take_says_how_much_of_it_the_envelope_covers() {
    let frames = 48_000;
    let bytes = wav_bytes(frames, 8_000);
    let dir = std::env::temp_dir().join(format!("resonance-drums-sinfo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("long.wav");
    std::fs::write(&path, &bytes).unwrap();
    let meta = std::fs::metadata(&path).unwrap();
    let data = resonance_drums::kit::decode_sample_streamed(
        bytes,
        48_000.0,
        4_800,
        &path,
        meta.len(),
        meta.modified().ok(),
    )
    .unwrap();
    assert!(data.tail().is_some(), "the take did not stream");
    let take = LoadedSample::from_shared(Arc::new(data));
    let mut pad = silent_pad(9);
    pad.close_mics.push(LoadedMicBank {
        position: "Tom01".to_string(),
        setup_key: "07_Tom01_md421".to_string(),
        layers: vec![VelocityLayer::new(vec![take])],
    });
    let info = sample_info::info_for_pad(&pad, 48_000.0).unwrap();
    assert_eq!(info.frames, frames);
    assert_eq!(info.resident_frames, 4_800);
    assert!((info.resident_fraction() - 0.1).abs() < 1e-6);
    let shape = info.take(0, 0).unwrap();
    assert_eq!((shape.frames, shape.resident_frames), (frames, 4_800));
    assert!((shape.resident_fraction() - 0.1).abs() < 1e-6);
    // Whole takes cover all of themselves.
    let whole = sample_info::info_for_pad(&tom_pad(9), 48_000.0).unwrap();
    assert_eq!(whole.resident_fraction(), 1.0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Rebuilding the infos for a kit whose takes did not change reuses
/// their envelopes (the same allocation) instead of measuring them again;
/// a take that is gone is forgotten.
#[test]
fn rebuilt_infos_share_the_envelopes_of_unchanged_takes() {
    let pads = vec![tom_pad(9)];
    let first = sample_info::infos_for_pads(&pads, 48_000.0);
    let second = sample_info::infos_for_pads(&pads, 48_000.0);
    let (a, b) = (first[0].as_ref().unwrap(), second[0].as_ref().unwrap());
    assert!(Arc::ptr_eq(&a.envelope, &b.envelope));
    for (la, lb) in a.takes.iter().zip(&b.takes) {
        for (ta, tb) in la.iter().zip(lb) {
            assert!(Arc::ptr_eq(&ta.envelope, &tb.envelope));
        }
    }
    // A different decoded take with the same samples is measured anew.
    let other = sample_info::infos_for_pads(&[tom_pad(9)], 48_000.0);
    assert!(!Arc::ptr_eq(&a.envelope, &other[0].as_ref().unwrap().envelope));
}
