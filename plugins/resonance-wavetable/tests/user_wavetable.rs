//! User wavetables: WAV import and frame slicing, the runtime mip build, and
//! the state round-trip that brings a table back with a project.
//!
//! The mip tests hold a user table to the bundled tables' own standard: built
//! from a bundled frame it must land on the bundled mips, and read across the
//! keyboard it must not alias (the `mip_aliasing` measurement, on a naive saw
//! — the worst thing a user can import).

use std::f32::consts::TAU;
use std::io::Cursor;
use std::time::{Duration, Instant};

use resonance_plugin::{EventIterator, NoteEvent, OutputBuffer, ResonancePlugin};
use resonance_wavetable::dsp::oscillator::{phase_inc, plan_tap, read_tap};
use resonance_wavetable::dsp::user_table::{UserTable, MAX_USER_FRAMES};
use resonance_wavetable::dsp::wavetable::{
    load_bundled, Wavetable, NUM_OCTAVES, USER_WAVETABLE_INDEX, WAVETABLE_SIZE,
};
use resonance_wavetable::user_wavetable::import::{
    clm_frame_size, import_wav_bytes, slice_frames,
};
use resonance_wavetable::user_wavetable::state::{encode_frames, STATE_KEY};
use resonance_wavetable::ResonanceWavetable;
use serde_json::{json, Value};

const N: usize = WAVETABLE_SIZE;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// `len` samples of harmonic `h` of a cycle `len` long.
fn sine(len: usize, h: usize) -> Vec<f32> {
    (0..len)
        .map(|i| (TAU * h as f32 * i as f32 / len as f32).sin())
        .collect()
}

/// A 32-bit float WAV holding `channels` interleaved.
fn wav_f32(samples: &[f32], channels: u16) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels,
        sample_rate: 44_100,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut bytes = Cursor::new(Vec::new());
    let mut w = hound::WavWriter::new(&mut bytes, spec).unwrap();
    for &s in samples {
        w.write_sample(s).unwrap();
    }
    w.finalize().unwrap();
    bytes.into_inner()
}

/// `wav` with a Serum `clm ` chunk declaring `frame_size`, placed right after
/// `fmt ` where Serum writes it.
fn with_clm(wav: &[u8], frame_size: usize) -> Vec<u8> {
    let fmt_len = u32::from_le_bytes(wav[16..20].try_into().unwrap()) as usize;
    let at = 20 + fmt_len + (fmt_len & 1);
    let text = format!("<!>{frame_size} 10000000 wavetable (www.xferrecords.com)");
    let mut body = text.into_bytes();
    let len = body.len();
    if len % 2 == 1 {
        body.push(0);
    }
    let mut out = wav[..at].to_vec();
    out.extend_from_slice(b"clm ");
    out.extend_from_slice(&(len as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out.extend_from_slice(&wav[at..]);
    let riff = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff.to_le_bytes());
    out
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).fold(0.0f32, |m, (x, y)| m.max((x - y).abs()))
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("resonance_wt_user_{}_{name}", std::process::id()))
}

// ---------------------------------------------------------------------------
// WAV -> frames
// ---------------------------------------------------------------------------

#[test]
fn a_multi_frame_file_slices_into_2048_sample_frames() {
    // Three frames — harmonics 1, 2, 3 — at half scale over a DC offset.
    let mut samples = Vec::new();
    for h in 1..=3 {
        samples.extend(sine(N, h).iter().map(|s| 0.5 * s + 0.1));
    }
    let imported = import_wav_bytes(&wav_f32(&samples, 1)).unwrap();

    assert_eq!(imported.num_frames, 3);
    assert_eq!(imported.source_frame_size, N);
    assert_eq!(imported.frames.len(), 3 * N);
    // DC gone, normalised to unit peak, frame for frame in order.
    for h in 1..=3 {
        let frame = &imported.frames[(h - 1) * N..h * N];
        assert!(max_diff(frame, &sine(N, h)) < 1e-4, "frame {h} is not harmonic {h}");
    }
}

#[test]
fn a_single_cycle_of_any_length_is_resampled_to_one_frame() {
    for len in [600usize, 2500] {
        let cycle: Vec<f32> = sine(len, 1)
            .iter()
            .zip(sine(len, 3))
            .map(|(a, b)| 0.3 * a + 0.1 * b)
            .collect();
        let imported = import_wav_bytes(&wav_f32(&cycle, 1)).unwrap();

        assert_eq!(imported.num_frames, 1, "{len}-sample cycle");
        assert_eq!(imported.source_frame_size, len);
        let expected: Vec<f32> = sine(N, 1)
            .iter()
            .zip(sine(N, 3))
            .map(|(a, b)| 0.3 * a + 0.1 * b)
            .collect();
        let scale = peak(&expected);
        let expected: Vec<f32> = expected.iter().map(|s| s / scale).collect();
        assert!(
            max_diff(&imported.frames, &expected) < 1e-4,
            "{len}-sample cycle did not resample to the same wave"
        );
    }
}

#[test]
fn a_clm_chunk_sets_the_frame_size() {
    // Four 1024-sample frames: without the chunk this would read as two
    // 2048-sample frames of nonsense.
    let mut samples = Vec::new();
    for h in 1..=4 {
        samples.extend(sine(1024, h));
    }
    let wav = with_clm(&wav_f32(&samples, 1), 1024);
    assert_eq!(clm_frame_size(&wav), Some(1024));

    let imported = import_wav_bytes(&wav).unwrap();
    assert_eq!(imported.num_frames, 4);
    assert_eq!(imported.source_frame_size, 1024);
    for h in 1..=4 {
        let frame = &imported.frames[(h - 1) * N..h * N];
        assert!(max_diff(frame, &sine(N, h)) < 1e-4, "frame {h} is not harmonic {h}");
    }
}

#[test]
fn a_plain_wav_has_no_clm_frame_size() {
    assert_eq!(clm_frame_size(&wav_f32(&sine(N, 1), 1)), None);
}

#[test]
fn a_long_file_is_thinned_evenly_to_the_frame_cap() {
    // Frame `i` is a sine at amplitude `i + 1`, so a frame's level says
    // which source frame it was.
    let source = MAX_USER_FRAMES + 44;
    let mut samples = Vec::new();
    for i in 0..source {
        samples.extend(sine(N, 1).iter().map(|s| s * (i + 1) as f32));
    }
    let imported = slice_frames(&samples, None).unwrap();

    assert_eq!(imported.num_frames, MAX_USER_FRAMES);
    assert_eq!(imported.source_frames, source);
    // First and last source frames both survive: the whole morph is kept.
    let level = |f: usize| peak(&imported.frames[f * N..(f + 1) * N]) * source as f32;
    assert!((level(0) - 1.0).abs() < 1e-2, "first frame is source {}", level(0));
    assert!(
        (level(MAX_USER_FRAMES - 1) - source as f32).abs() < 1e-2,
        "last frame is source {}",
        level(MAX_USER_FRAMES - 1)
    );
}

#[test]
fn a_file_that_is_not_whole_frames_drops_its_tail() {
    let mut samples = sine(N, 1);
    samples.extend(sine(N, 2));
    samples.extend(vec![0.9; 300]);
    let imported = slice_frames(&samples, None).unwrap();
    assert_eq!(imported.num_frames, 2);
    assert!(max_diff(&imported.frames[N..], &sine(N, 2)) < 1e-4);
}

#[test]
fn stereo_is_mixed_to_mono() {
    // Left: the wave. Right: silence. The mono mix is the wave at half
    // level, which normalisation brings back to full scale.
    let wave = sine(N, 2);
    let interleaved: Vec<f32> = wave.iter().flat_map(|&s| [s, 0.0]).collect();
    let imported = import_wav_bytes(&wav_f32(&interleaved, 2)).unwrap();
    assert_eq!(imported.num_frames, 1);
    assert!(max_diff(&imported.frames, &wave) < 1e-4);
}

#[test]
fn silent_empty_and_non_wav_files_are_refused() {
    assert!(import_wav_bytes(&wav_f32(&vec![0.0; N], 1)).is_err());
    assert!(import_wav_bytes(&wav_f32(&[], 1)).is_err());
    assert!(import_wav_bytes(b"definitely not a wav file").is_err());
    // A constant is pure DC, so it is silence once the DC is gone.
    assert!(slice_frames(&vec![0.5; N], None).is_err());
}

// ---------------------------------------------------------------------------
// Mips
// ---------------------------------------------------------------------------

#[test]
fn a_user_table_rejects_malformed_frames() {
    assert!(UserTable::build(&[]).is_err());
    assert!(UserTable::build(&vec![0.1; N + 1]).is_err());
    assert!(UserTable::build(&vec![0.1; N * (MAX_USER_FRAMES + 1)]).is_err());
    let mut bad = sine(N, 1);
    bad[7] = f32::NAN;
    assert!(UserTable::build(&bad).is_err());
}

/// The runtime builder is the build-time construction on an FFT: fed a
/// bundled frame's full-band level, it must reproduce that frame's every
/// mip level.
#[test]
fn user_mips_match_the_bundled_construction() {
    let tables = load_bundled();
    // Basic's saw (frame 2) and a dense sync-sweep frame (table 9).
    for (table, frame) in [(0usize, 2usize), (9, 40)] {
        let bundled = &tables[table];
        let source = bundled.mip(frame, 0).to_vec();
        let user = UserTable::build(&source).unwrap();
        // SAFETY: `user` outlives every read of `view` in this loop body.
        let view = unsafe { user.view() };
        for octave in 0..NUM_OCTAVES {
            let d = max_diff(view.mip(0, octave), bundled.mip(frame, octave));
            assert!(
                d < 2e-3,
                "table {table} frame {frame} level {octave}: user mip differs by {d}"
            );
        }
    }
}

const ALIAS_N: usize = 1 << 15;
const WARMUP: usize = 64;
const GUARD_BINS: f64 = 10.0;

fn render_osc(table: &Wavetable, freq: f32, sr: f32) -> Vec<f64> {
    let tap = plan_tap(table, 0.0, freq, sr);
    let inc = phase_inc(freq, sr);
    let mut phase = 0.0f64;
    let mut out = Vec::with_capacity(ALIAS_N);
    for i in 0..(ALIAS_N + WARMUP) {
        let s = read_tap(table, &tap, phase);
        if i >= WARMUP {
            out.push(s as f64);
        }
        phase += inc;
        if phase >= 1.0 {
            phase -= 1.0;
        }
    }
    out
}

/// In-place iterative radix-2 FFT over (re, im).
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -std::f64::consts::TAU / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (ang * k as f64).sin_cos();
                let a = start + k;
                let b = a + len / 2;
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// Energy off the harmonic series relative to the fundamental, in dB, under
/// a 7-term Blackman-Harris window (as in `mip_aliasing.rs`).
fn alias_db(x: &[f64], freq: f32, sr: f32) -> f64 {
    const A: [f64; 7] = [
        0.271_051_400_693_42,
        -0.433_297_939_234_48,
        0.218_122_999_543_11,
        -0.065_925_446_388_03,
        0.010_811_742_098_37,
        -0.000_776_584_825_22,
        0.000_013_887_217_35,
    ];
    let n = x.len();
    let mut re: Vec<f64> = x
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let t = std::f64::consts::TAU * i as f64 / n as f64;
            let w: f64 = A.iter().enumerate().map(|(k, a)| a * (k as f64 * t).cos()).sum();
            s * w
        })
        .collect();
    let mut im = vec![0.0; n];
    fft(&mut re, &mut im);
    let bin_hz = sr as f64 / n as f64;
    let f0_bin = freq as f64 / bin_hz;
    let mut fundamental = 0.0;
    let mut off_harmonic = 0.0;
    for k in (GUARD_BINS as usize + 1)..=n / 2 {
        let p = re[k] * re[k] + im[k] * im[k];
        let h = (k as f64 / f0_bin).round().max(1.0);
        let on_harmonic = (k as f64 - h * f0_bin).abs() <= GUARD_BINS;
        if on_harmonic && h == 1.0 {
            fundamental += p;
        } else if !on_harmonic {
            off_harmonic += p;
        }
    }
    assert!(fundamental > 0.0, "{freq} Hz: no fundamental");
    10.0 * (off_harmonic.max(1e-300) / fundamental).log10()
}

/// A naive (aliasing-rich, every harmonic to Nyquist) saw imported as a user
/// table reads alias-free across the keyboard, to the bundled tables' bound.
#[test]
fn a_user_table_is_band_limited_across_the_keyboard() {
    let naive_saw: Vec<f32> = (0..N).map(|i| 2.0 * i as f32 / N as f32 - 1.0).collect();
    let imported = slice_frames(&naive_saw, None).unwrap();
    let user = UserTable::build(&imported.frames).unwrap();
    // SAFETY: `user` outlives every read of `view` below.
    let view = unsafe { user.view() };

    let sr = 48_000.0;
    let mut worst = (f64::NEG_INFINITY, 0.0f32);
    // Every third key of the keyboard, plus just above each level's limit.
    let mut freqs: Vec<f32> = (21..=108)
        .step_by(3)
        .map(|n| 440.0 * ((n as f32 - 69.0) / 12.0).exp2())
        .collect();
    freqs.extend((2..10).map(|k| 8.175_799 * (1u32 << k) as f32 * 1.02));
    for f in freqs {
        let x = render_osc(&view, f, sr);
        assert!(x.iter().any(|s| s.abs() > 0.1), "{f} Hz rendered silence");
        let db = alias_db(&x, f, sr);
        // Below ~70 Hz the bundled tables are held to -78 dB (the dense
        // levels' interpolation floor, see `mip_aliasing.rs`).
        let limit = if f < 70.0 { -78.0 } else { -80.0 };
        assert!(db < limit, "user saw aliases at {f:.1} Hz: {db:.1} dB (limit {limit})");
        if db > worst.0 {
            worst = (db, f);
        }
    }
    eprintln!("user saw @ 48 kHz: worst off-harmonic {:.1} dB at {:.1} Hz", worst.0, worst.1);
}

// ---------------------------------------------------------------------------
// The plugin: state round-trip and fallback
// ---------------------------------------------------------------------------

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

fn set_param(plugin: &ResonanceWavetable, id: &str, value: f64) {
    let p = (0..plugin.param_count())
        .map(|i| plugin.param(i))
        .find(|p| p.id() == id)
        .unwrap_or_else(|| panic!("no param `{id}`"));
    p.set_plain(value);
}

/// Hold a note for a few blocks, left and right concatenated.
fn render_note(plugin: &mut ResonanceWavetable) -> Vec<f32> {
    let mut out = Vec::new();
    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    for block in 0..8 {
        let events = if block == 0 {
            vec![NoteEvent::NoteOn {
                note: 57,
                velocity: 0.8,
                timing: 3,
            }]
        } else {
            Vec::new()
        };
        let mut iter = EventIterator::new(&events);
        let mut outs = [OutputBuffer {
            left: &mut left,
            right: &mut right,
        }];
        plugin.process(&mut outs, BLOCK, &mut iter, None);
        out.extend_from_slice(&left);
        out.extend_from_slice(&right);
    }
    out
}

fn state_of(plugin: &ResonanceWavetable) -> Value {
    serde_json::from_slice(&plugin.save_state()).unwrap()
}

/// Two frames with plenty of harmonics, so a wrong table can't sound right.
fn test_frames() -> Vec<f32> {
    let mut frames: Vec<f32> = (0..N)
        .map(|i| {
            let t = TAU * i as f32 / N as f32;
            0.6 * t.sin() + 0.3 * (5.0 * t).sin() + 0.1 * (11.0 * t).cos()
        })
        .collect();
    frames.extend(sine(N, 7));
    frames
}

#[test]
fn a_saved_user_table_restores_from_the_state_alone() {
    // Import from a real file…
    let path = temp_path("roundtrip.wav");
    std::fs::write(&path, wav_f32(&test_frames(), 1)).unwrap();
    let path_str = path.to_string_lossy().into_owned();

    let mut a = ResonanceWavetable::new();
    a.user_wavetables().restore_file(0, &path_str).unwrap();
    a.initialize(SR, BLOCK as u32);
    set_param(&a, "osc1_wavetable", USER_WAVETABLE_INDEX as f64);
    set_param(&a, "osc1_position", 0.3);
    let from_file = render_note(&mut a);
    assert!(a.engine().user_table(0).is_some(), "the import never reached the engine");
    let state = a.save_state();

    // …then lose the file: the project must not care.
    std::fs::remove_file(&path).unwrap();

    let mut b = ResonanceWavetable::new();
    assert!(b.load_state(&state));
    b.initialize(SR, BLOCK as u32);
    let restored = render_note(&mut b);

    assert!(peak(&from_file) > 0.01, "the user table rendered silence");
    assert_eq!(from_file, restored, "the restored table does not sound the same");
    let (ia, ib) = (a.user_wavetables().info(0), b.user_wavetables().info(0));
    assert_eq!(ia.frames, ib.frames);
    assert_eq!(ib.path, path_str);
    assert_eq!(ib.name, "resonance_wt_user_".to_string() + &std::process::id().to_string() + "_roundtrip");
    assert!(ib.error.is_none());

    // And a re-save reproduces the document exactly.
    assert_eq!(state_of(&b), serde_json::from_slice::<Value>(&state).unwrap());
}

#[test]
fn the_user_table_sounds_different_from_the_fallback() {
    // Guards the round-trip test above against a vacuous pass where neither
    // side ever reached the user table.
    let mut user = ResonanceWavetable::new();
    user.user_wavetables()
        .restore_frames(0, "", "frames", test_frames())
        .unwrap();
    user.initialize(SR, BLOCK as u32);
    set_param(&user, "osc1_wavetable", USER_WAVETABLE_INDEX as f64);

    let mut basic = ResonanceWavetable::new();
    basic.initialize(SR, BLOCK as u32);
    set_param(&basic, "osc1_wavetable", 0.0);

    assert_ne!(render_note(&mut user), render_note(&mut basic));
}

#[test]
fn a_missing_file_falls_back_to_the_bundled_table_and_says_so() {
    let gone = "/nonexistent/resonance/gone.wav";
    let doc = json!({
        "params": { "osc1_wavetable": USER_WAVETABLE_INDEX },
        STATE_KEY: { "osc1": { "path": gone, "name": "gone" } }
    });

    let mut plugin = ResonanceWavetable::new();
    // A table from an earlier project must not survive the missing file.
    plugin
        .user_wavetables()
        .restore_frames(0, "", "stale", test_frames())
        .unwrap();
    assert!(plugin.load_state(&serde_json::to_vec(&doc).unwrap()));
    plugin.initialize(SR, BLOCK as u32);
    let fallback = render_note(&mut plugin);

    let info = plugin.user_wavetables().info(0);
    assert!(!info.is_loaded());
    assert_eq!(info.path, gone, "the project should still name its file");
    let err = info.error.expect("a missing file must be reported");
    assert!(err.contains("not found"), "unhelpful error: {err}");
    assert!(plugin.engine().user_table(0).is_none());

    // It plays bundled table 0, exactly.
    let mut basic = ResonanceWavetable::new();
    basic.initialize(SR, BLOCK as u32);
    set_param(&basic, "osc1_wavetable", 0.0);
    assert_eq!(fallback, render_note(&mut basic));

    // And the path survives a re-save, so the file is found again later.
    let saved = state_of(&plugin);
    assert_eq!(saved[STATE_KEY]["osc1"]["path"], json!(gone));
    assert!(saved[STATE_KEY]["osc1"].get("frames").is_none());
}

#[test]
fn a_default_instance_saves_no_user_table_state() {
    let plugin = ResonanceWavetable::new();
    assert!(state_of(&plugin).get(STATE_KEY).is_none());
}

#[test]
fn state_without_user_tables_clears_them() {
    let mut plugin = ResonanceWavetable::new();
    plugin
        .user_wavetables()
        .restore_frames(1, "", "x", test_frames())
        .unwrap();
    plugin.initialize(SR, BLOCK as u32);
    assert!(plugin.engine().user_table(1).is_some());

    assert!(plugin.load_state(&serde_json::to_vec(&json!({ "params": {} })).unwrap()));
    render_note(&mut plugin);
    assert!(plugin.engine().user_table(1).is_none());
    assert!(!plugin.user_wavetables().info(1).is_loaded());
    assert!(state_of(&plugin).get(STATE_KEY).is_none());
}

#[test]
fn embedded_frames_are_restored_verbatim_per_oscillator() {
    let frames = test_frames();
    let doc = json!({
        "params": {},
        STATE_KEY: { "osc2": {
            "path": "", "name": "embedded", "frame_size": N,
            "frames": encode_frames(&frames),
        } }
    });
    let mut plugin = ResonanceWavetable::new();
    assert!(plugin.load_state(&serde_json::to_vec(&doc).unwrap()));
    plugin.initialize(SR, BLOCK as u32);

    assert!(plugin.engine().user_table(0).is_none());
    assert_eq!(plugin.engine().user_table(1).map(|t| t.num_frames()), Some(2));
    assert_eq!(
        plugin.user_wavetables().info(1).frames.as_deref(),
        Some(frames.as_slice())
    );
    assert_eq!(state_of(&plugin)[STATE_KEY], doc[STATE_KEY]);
}

#[test]
fn a_background_load_lands_in_the_engine() {
    let path = temp_path("background.wav");
    std::fs::write(&path, wav_f32(&test_frames(), 1)).unwrap();

    let mut plugin = ResonanceWavetable::new();
    plugin.initialize(SR, BLOCK as u32);
    let shared = plugin.user_wavetables().clone();
    let generation = shared.request_file(0, path.to_string_lossy().into_owned());

    let deadline = Instant::now() + Duration::from_secs(30);
    while shared.info(0).generation < generation {
        assert!(Instant::now() < deadline, "the background import never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    std::fs::remove_file(&path).unwrap();
    let info = shared.info(0);
    assert!(info.error.is_none(), "{:?}", info.error);
    assert_eq!(info.num_frames(), 2);

    render_note(&mut plugin);
    assert_eq!(plugin.engine().user_table(0).map(|t| t.num_frames()), Some(2));

    // A failed load after it leaves the table in place and reports.
    let generation = shared.request_file(0, "/nonexistent/nope.wav".into());
    while shared.info(0).generation < generation {
        assert!(Instant::now() < deadline, "the failing import never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    render_note(&mut plugin);
    assert!(shared.info(0).error.is_some());
    assert!(shared.info(0).is_loaded());
    assert!(plugin.engine().user_table(0).is_some());
}
