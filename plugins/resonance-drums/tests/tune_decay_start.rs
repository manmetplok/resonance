//! Pitch, decay and sample start per pad (drums-plugin-rework.md §7 E8).
//!
//! - `pad_N_tune` +12 plays an octave up (and −12 an octave down), by
//!   fractional playback (Hermite down, band-limited up) — on a resident take
//!   and on a streamed one, whose tuned render is bit-identical to the
//!   resident one (the same frames, read through the head / ring path),
//!   live with a stepped reader and offline with reader threads.
//! - At 0 st the voice stays on the integer path: the output is the
//!   sample itself, bit for bit.
//! - `pad_N_decay` (with `pad_N_hold`) ends the tail; Off plays it all.
//! - `pad_N_start` skips into the sample.
//! - A voice two octaves up in 4096-frame live blocks — a whole ring a
//!   block — is served by the reader *inside* the block, so it does not
//!   underrun every block.

use std::io::Write;
use std::path::{Path, PathBuf};

use resonance_drums::drum_map::{self, PAD_MAPPINGS};
use resonance_drums::dsp::{DrumSampler, PortBuffers};
use resonance_drums::kit::{
    LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer, NUM_OUTPUT_PORTS,
};
use resonance_drums::kit_loader::cache::SampleCache;
use resonance_drums::params::{decay_label, tune_from_label, tune_label, DrumParams, DECAY_OFF_MS};
use resonance_drums::stream::reader::ReaderPool;
use resonance_drums::stream::{Pace, RenderMode};
use resonance_plugin::Param;

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;
/// The test tone: 300 Hz at 48 kHz, a period of 160 frames.
const TONE_HZ: f32 = 300.0;

fn tone_frame(i: usize) -> f32 {
    (i as f32 * TONE_HZ * std::f32::consts::TAU / SR).sin() * 0.5
}

/// A 16-bit mono 48 kHz WAV of the test tone, `frames` long.
fn write_tone(path: &Path, frames: usize) {
    let mut data = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        data.extend_from_slice(&((tone_frame(i) * 32_767.0) as i16).to_le_bytes());
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&48_000u32.to_le_bytes());
    out.extend_from_slice(&96_000u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    std::fs::File::create(path)
        .unwrap()
        .write_all(&out)
        .unwrap();
}

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("drums-e8-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A kit with `take` on the kick's one close bank and nothing else.
fn kick_kit(take: LoadedSample) -> Vec<LoadedPad> {
    PAD_MAPPINGS
        .iter()
        .enumerate()
        .map(|(i, m)| LoadedPad {
            name: m.name.to_string(),
            choke_group: m.choke_group,
            output_group: m.output_group,
            close_mics: if i == 0 {
                vec![LoadedMicBank {
                    position: "KickIn".to_string(),
                    setup_key: String::new(),
                    layers: vec![VelocityLayer::new(vec![take.clone()])],
                }]
            } else {
                Vec::new()
            },
            extra_banks: Vec::new(),
            overhead: None,
        })
        .collect()
}

/// A resident take of the test tone, `frames` long (the same samples the
/// 16-bit file decodes to).
fn resident_tone(frames: usize) -> LoadedSample {
    LoadedSample::mono(
        (0..frames)
            .map(|i| ((tone_frame(i) * 32_767.0) as i16) as f32 / 32_768.0)
            .collect(),
    )
}

/// Render one kick hit at full velocity for `frames` frames, through
/// `params` (Stereo: everything on Main), and return Main's left channel.
fn render_kick(
    sampler: &mut DrumSampler,
    params: &DrumParams,
    frames: usize,
    pump: Option<&ReaderPool>,
) -> Vec<f32> {
    render_kick_in(sampler, params, frames, BLOCK, pump)
}

/// [`render_kick`] in blocks of `block` frames.
fn render_kick_in(
    sampler: &mut DrumSampler,
    params: &DrumParams,
    frames: usize,
    block: usize,
    pump: Option<&ReaderPool>,
) -> Vec<f32> {
    sampler.update_global_settings(params);
    sampler.note_on(drum_map::KICK, 1.0);
    let mut out = Vec::with_capacity(frames);
    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0; block], vec![0.0; block]))
        .collect();
    while out.len() < frames {
        {
            let mut ports: Vec<PortBuffers<'_>> = bufs
                .iter_mut()
                .map(|(l, r)| PortBuffers {
                    left: l.as_mut_slice(),
                    right: r.as_mut_slice(),
                })
                .collect();
            sampler.render_block(&mut ports, block, params, &[]);
        }
        out.extend_from_slice(&bufs[0].0);
        if let Some(pool) = pump {
            pool.pump();
        }
    }
    out.truncate(frames);
    out
}

fn resident_sampler(take: LoadedSample) -> DrumSampler {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut s = DrumSampler::new(rx);
    s.set_sample_rate(SR);
    s.pads = kick_kit(take);
    s
}

fn params_tuned(st: f32) -> DrumParams {
    let p = DrumParams::default();
    p.pads[0].tune.set_value(st);
    p
}

/// The tone's frequency in `signal[from..to]`, from its upward zero
/// crossings, interpolated to the sub-frame.
fn frequency(signal: &[f32], from: usize, to: usize) -> f32 {
    let mut crossings = Vec::new();
    for i in from + 1..to {
        let (a, b) = (signal[i - 1], signal[i]);
        if a < 0.0 && b >= 0.0 {
            crossings.push(i as f32 - 1.0 + a / (a - b));
        }
    }
    assert!(
        crossings.len() > 4,
        "too few crossings ({})",
        crossings.len()
    );
    let periods = (crossings.len() - 1) as f32;
    SR * periods / (crossings[crossings.len() - 1] - crossings[0])
}

fn rms(signal: &[f32]) -> f32 {
    (signal.iter().map(|s| s * s).sum::<f32>() / signal.len().max(1) as f32).sqrt()
}

#[test]
fn tune_zero_plays_the_sample_bit_for_bit() {
    let take = resident_tone(20_000);
    let samples: Vec<f32> = take.samples().to_vec();
    let mut s = resident_sampler(take);
    let out = render_kick(&mut s, &params_tuned(0.0), 10_000, None);
    for (i, (&got, &want)) in out.iter().zip(&samples).enumerate() {
        assert_eq!(got.to_bits(), want.to_bits(), "frame {i}");
    }
}

#[test]
fn tune_plus_12_is_an_octave_up_and_minus_12_an_octave_down_resident() {
    for (st, want) in [
        (12.0, 2.0 * TONE_HZ),
        (-12.0, TONE_HZ / 2.0),
        (7.0, TONE_HZ * 1.498_307),
    ] {
        let mut s = resident_sampler(resident_tone(96_000));
        let out = render_kick(&mut s, &params_tuned(st), 20_000, None);
        let got = frequency(&out, 1_000, 19_000);
        assert!(
            (got - want).abs() / want < 0.002,
            "tune {st:+} st: {got:.2} Hz, want {want:.2} Hz"
        );
    }
    // A cent is a cent: +0.5 st (50 cents).
    let mut s = resident_sampler(resident_tone(96_000));
    let out = render_kick(&mut s, &params_tuned(0.5), 30_000, None);
    let want = TONE_HZ * 2f32.powf(0.5 / 12.0);
    let got = frequency(&out, 1_000, 29_000);
    assert!(
        (got - want).abs() < 0.2,
        "+50 ct: {got:.3} Hz, want {want:.3} Hz"
    );
}

/// The Hermite interpolation is smooth: an octave-up sine stays a sine
/// (no step bigger than the tone's own slope allows).
#[test]
fn tuned_playback_has_no_steps() {
    let mut s = resident_sampler(resident_tone(96_000));
    let out = render_kick(&mut s, &params_tuned(12.0), 10_000, None);
    // Max slope of 0.5·sin(2π·600·t) per frame at 48 kHz.
    let slope = 0.5 * std::f32::consts::TAU * 600.0 / SR;
    // Past the onset: a pitched-up voice reads band-limited (DSP2-09),
    // and the sinc rings by a few percent over the sine's abrupt start
    // for the kernel's half-width (10 output frames at +12 st).
    for i in 32..out.len() {
        assert!(
            (out[i] - out[i - 1]).abs() <= slope * 1.05,
            "step {} at {i}",
            (out[i] - out[i - 1]).abs()
        );
    }
}

/// A streamed take (a small preload: the head is 4096 frames) plays the
/// same tuned frames as the resident one, bit for bit — the pitched voice
/// reads its tail through the ring — live with a stepped reader that
/// catches up between blocks, and offline with reader threads.
#[test]
fn tune_on_a_streamed_take_matches_the_resident_take_bit_for_bit() {
    let dir = Dir::new("stream");
    let path = dir.0.join("tone.wav");
    write_tone(&path, 96_000);
    let cache = SampleCache::new();
    let (whole, _) = cache.get_or_decode_preload(&path, SR, 0).unwrap();
    let (split, _) = cache.get_or_decode_preload(&path, SR, 4_096).unwrap();
    assert!(split.tail().is_some(), "the take streams");
    assert_eq!(split.resident_frames(), 4_096);

    for st in [12.0, 24.0, -12.0, 3.25] {
        let params = params_tuned(st);
        let frames = 40_000;
        let mut resident = resident_sampler(LoadedSample::from_shared(whole.clone()));
        let want = render_kick(&mut resident, &params, frames, None);
        assert!(rms(&want[8_000..]) > 0.1, "{st:+} st: the tail sounds");

        // Live, stepped reader.
        let pool = ReaderPool::stepped();
        let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
        let mut live = DrumSampler::with_reader_pool(rx, &pool);
        live.set_sample_rate(SR);
        live.set_render_mode(RenderMode::Realtime);
        live.pads = kick_kit(LoadedSample::from_shared(split.clone()));
        pool.pump();
        let got = render_kick(&mut live, &params, frames, Some(&pool));
        assert_eq!(live.stream_underruns(), 0, "{st:+} st: live underran");
        let diff = got
            .iter()
            .zip(&want)
            .position(|(a, b)| a.to_bits() != b.to_bits());
        assert_eq!(diff, None, "{st:+} st: live stream differs from resident");

        // Offline, reader threads, waited for.
        let threads = ReaderPool::new(2);
        let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
        let mut offline = DrumSampler::with_reader_pool(rx, &threads);
        offline.set_sample_rate(SR);
        offline.set_render_mode(RenderMode::Offline);
        offline.pads = kick_kit(LoadedSample::from_shared(split.clone()));
        let got = render_kick(&mut offline, &params, frames, None);
        assert_eq!(offline.stream_underruns(), 0, "{st:+} st: offline underran");
        let diff = got
            .iter()
            .zip(&want)
            .position(|(a, b)| a.to_bits() != b.to_bits());
        assert_eq!(
            diff, None,
            "{st:+} st: offline stream differs from resident"
        );
    }

    // And it is the octave.
    let mut s = resident_sampler(LoadedSample::from_shared(whole));
    let out = render_kick(&mut s, &params_tuned(12.0), 30_000, None);
    let got = frequency(&out, 9_000, 29_000);
    assert!((got - 600.0).abs() < 1.2, "{got} Hz on the tail");
}

/// +24 st reads four take frames per output frame, so a 4096-frame live
/// block consumes a whole ring (16384 frames) and an 8192-frame one (the
/// largest the host asks for) two. Served only between blocks (the
/// reader cannot refill behind a voice whose progress is published at
/// the block's end), the voice has at most a ring a block: exactly enough
/// at 4096, nothing to spare, and an underrun every block past it. It publishes inside the block
/// (`MID_BLOCK_PUBLISH_FRAMES`), so a reader that runs meanwhile — here a
/// stepped one, pumped where the voice publishes — keeps it whole: no
/// underruns, bit-identical to the resident take.
#[test]
fn a_tuned_voice_in_big_live_blocks_is_served_inside_the_block() {
    let dir = Dir::new("big-blocks");
    let path = dir.0.join("tone.wav");
    write_tone(&path, 400_000);
    let cache = SampleCache::new();
    let (whole, _) = cache.get_or_decode_preload(&path, SR, 0).unwrap();
    let (split, _) = cache.get_or_decode_preload(&path, SR, 4_096).unwrap();
    assert!(split.tail().is_some(), "the take streams");
    let params = params_tuned(24.0);
    // 320k take frames: some 10 to 20 big blocks on the tail.
    let frames = 80_000;
    for big in [4_096usize, 8_192] {
        let want = render_kick_in(
            &mut resident_sampler(LoadedSample::from_shared(whole.clone())),
            &params,
            frames,
            big,
            None,
        );
        assert!(rms(&want[60_000..]) > 0.1, "the tail sounds");

        let live = |served_inside: bool| -> (Vec<f32>, u64) {
            let pool = ReaderPool::stepped();
            let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
            let mut s = DrumSampler::with_reader_pool(rx, &pool);
            s.set_sample_rate(SR);
            s.set_render_mode(RenderMode::Realtime);
            s.pads = kick_kit(LoadedSample::from_shared(split.clone()));
            if served_inside {
                let reader = pool.clone();
                s.stream_set().set_mid_block_hook(Box::new(move || {
                    reader.pump();
                }));
            }
            let got = render_kick_in(&mut s, &params, frames, big, Some(&pool));
            (got, s.stream_underruns())
        };

        if big > 4_096 {
            // The control: past a ring a block, a reader that runs only
            // between blocks cannot keep up — every block underruns. (At
            // 4096 the ring is exactly enough, with nothing to spare.)
            let (_, underruns) = live(false);
            assert!(
                underruns >= 5,
                "{big}: served between blocks only, the voice underruns every block \
                 ({underruns})"
            );
        }

        let (got, underruns) = live(true);
        assert_eq!(underruns, 0, "{big}: served inside the block, it never underruns");
        let diff = got
            .iter()
            .zip(&want)
            .position(|(a, b)| a.to_bits() != b.to_bits());
        assert_eq!(diff, None, "{big}: the streamed render differs from the resident one");
    }
}

/// The tune resolves to the cent, as it reads: a value between two
/// cents plays the cent its label shows.
#[test]
fn tune_plays_the_cent_it_shows() {
    use resonance_drums::dsp::PadSettings;
    let p = DrumParams::default();
    p.pads[0].tune.set_value(1.004_9);
    assert_eq!(tune_label(p.pads[0].tune.value()), "+1.00 st");
    let rate = PadSettings::from_params(&p.pads[0], SR).rate;
    assert_eq!(rate, (1.0f32 / 12.0).exp2());
}

#[test]
fn the_reader_deadline_runs_in_output_time() {
    assert_eq!(Pace::at_rate(4_096, 1.0), Pace::unity(4_096));
    let up = Pace::at_rate(4_096, 2.0);
    assert_eq!(
        up.head_left, 2_048,
        "an octave up reaches its tail twice as soon"
    );
    assert_eq!(up.rate_q16, 2 << 16);
    let down = Pace::at_rate(4_096, 0.5);
    assert_eq!(down.head_left, 8_192);
}

#[test]
fn decay_ends_the_tail_and_off_plays_it_all() {
    let full = {
        let mut s = resident_sampler(resident_tone(48_000));
        render_kick(&mut s, &DrumParams::default(), 24_000, None)
    };
    let p = DrumParams::default();
    assert_eq!(p.pads[0].decay.value(), DECAY_OFF_MS, "Off by default");
    p.pads[0].hold.set_value(20.0);
    p.pads[0].decay.set_value(80.0);
    let cut = {
        let mut s = resident_sampler(resident_tone(48_000));
        render_kick(&mut s, &p, 24_000, None)
    };
    // Through the hold: untouched.
    assert_eq!(&cut[..960], &full[..960], "the hold leaves the start alone");
    // During the decay: quieter, and still sounding.
    let mid = 960 + 1_920;
    assert!(rms(&cut[mid..mid + 480]) < 0.5 * rms(&full[mid..mid + 480]));
    assert!(rms(&cut[mid..mid + 480]) > 0.0);
    // After hold + decay (100 ms = 4800 frames): silence; without, the tail.
    assert!(
        rms(&full[6_000..24_000]) > 0.1,
        "the sample is longer than that"
    );
    assert_eq!(rms(&cut[4_800..24_000]), 0.0, "decayed to silence");
    // Smoothly: no step bigger than the tone's own.
    let slope = 0.5 * std::f32::consts::TAU * TONE_HZ / SR;
    for i in 1..6_000 {
        assert!((cut[i] - cut[i - 1]).abs() <= slope * 1.05, "step at {i}");
    }
}

#[test]
fn start_skips_into_the_sample() {
    let take = resident_tone(20_000);
    let samples: Vec<f32> = take.samples().to_vec();
    let p = DrumParams::default();
    p.pads[0].start.set_value(10.0); // 480 frames at 48 kHz
    let mut s = resident_sampler(take);
    let out = render_kick(&mut s, &p, 8_000, None);
    for (i, &got) in out.iter().enumerate() {
        assert_eq!(got.to_bits(), samples[i + 480].to_bits(), "frame {i}");
    }
}

/// On a streamed take the start lands inside the head, and the rest
/// streams as before — tuned too.
#[test]
fn start_on_a_streamed_take_matches_the_resident_take() {
    let dir = Dir::new("start");
    let path = dir.0.join("tone.wav");
    write_tone(&path, 60_000);
    let cache = SampleCache::new();
    let (whole, _) = cache.get_or_decode_preload(&path, SR, 0).unwrap();
    let (split, _) = cache.get_or_decode_preload(&path, SR, 4_096).unwrap();
    let p = DrumParams::default();
    p.pads[0].start.set_value(50.0); // 2400 frames, inside the 4096 head
    p.pads[0].tune.set_value(5.0);
    let mut resident = resident_sampler(LoadedSample::from_shared(whole));
    let want = render_kick(&mut resident, &p, 20_000, None);
    let pool = ReaderPool::stepped();
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut live = DrumSampler::with_reader_pool(rx, &pool);
    live.set_sample_rate(SR);
    live.set_render_mode(RenderMode::Realtime);
    live.pads = kick_kit(LoadedSample::from_shared(split));
    let got = render_kick(&mut live, &p, 20_000, Some(&pool));
    assert!(rms(&want) > 0.1);
    assert_eq!(live.stream_underruns(), 0);
    assert!(got
        .iter()
        .zip(&want)
        .all(|(a, b)| a.to_bits() == b.to_bits()));
}

#[test]
fn the_params_read_and_parse() {
    let p = DrumParams::default();
    let pad = &p.pads[3];
    assert_eq!(pad.tune.id(), "pad_3_tune");
    assert_eq!((pad.tune.min_plain(), pad.tune.max_plain()), (-24.0, 24.0));
    assert_eq!(pad.tune.display(0.0), "0.00 st");
    assert_eq!(pad.tune.display(12.0), "+12.00 st");
    assert_eq!(pad.tune.display(-0.5), "-0.50 st");
    assert_eq!(tune_label(0.004), "0.00 st");
    assert_eq!(tune_from_label("+7 st"), Some(7.0));
    assert_eq!(tune_from_label("50 ct"), Some(0.5));
    assert_eq!(tune_from_label("-25 cents"), Some(-0.25));
    assert_eq!(tune_from_label("99"), Some(24.0), "clamped");

    assert_eq!(pad.hold.id(), "pad_3_hold");
    assert_eq!(pad.hold.display(250.0), "250 ms");
    assert_eq!(pad.hold.parse("1.5 s"), Some(1_500.0));
    assert_eq!(pad.decay.id(), "pad_3_decay");
    assert_eq!(pad.decay.display(DECAY_OFF_MS as f64), "Off");
    assert_eq!(decay_label(120.0), "120 ms");
    assert_eq!(pad.decay.parse("off"), Some(DECAY_OFF_MS as f64));
    assert_eq!(pad.start.id(), "pad_3_start");
    assert_eq!((pad.start.min_plain(), pad.start.max_plain()), (0.0, 100.0));
    assert_eq!(pad.start.default_plain(), 0.0);
}
