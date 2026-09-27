//! Render-pool stress bench (realtime-multithreading.md §6, P1 exit).
//!
//! The P1 stress project — 8 guitar tracks each running NAM amp + cab IR +
//! reverb, plus 4 wavetable instrument tracks playing chords — rendered
//! through the whole audio callback at quantum 128 / 48 kHz, once per
//! thread count. For each it prints the per-callback time against the
//! 2.667 ms budget: mean, p99, max and the number of over-budget cycles,
//! plus the pool's own critical-path / efficiency numbers.
//!
//! This measures the render, not the device: it runs as fast as it can on
//! an ordinary (non-realtime) thread, so it shows the CPU headroom the pool
//! buys, not scheduling jitter. The live check is the same project in the
//! app with `RESONANCE_AUDIO_STATS=1` for ten minutes.
//!
//! Needs the release bundles (`scripts/bundle.sh`), a `.nam` model and a
//! cab IR `.wav`:
//!
//! ```sh
//! RESONANCE_BENCH_NAM=/path/model.nam RESONANCE_BENCH_IR=/path/cab.wav \
//!     cargo bench -p resonance-audio --bench render_pool
//! ```
//!
//! Without them it looks under `~/.local/share/resonance/amp-models` and
//! `~/Documents/Guitar/IR`, and skips (exit 0) when nothing is found.
//! `RESONANCE_BENCH_BLOCKS` sets the measured length (default 4000 blocks,
//! 10.7 s of audio); `RESONANCE_BENCH_THREADS` a comma list of thread
//! counts (default `1,2,4,8`); `RESONANCE_BENCH_GUITARS` the guitar-track
//! count (default 8).

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use resonance_audio::test_support::{physical_cores, ClapBundle, MixAudioHarness, PluginSlot};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const SYNTHS: u64 = 4;

/// Guitar tracks: `RESONANCE_BENCH_GUITARS`, default 8 (the spec's
/// stress project).
fn guitars() -> u64 {
    std::env::var("RESONANCE_BENCH_GUITARS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8)
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn find_file(dir: &Path, ext: &str) -> Option<PathBuf> {
    let mut entries: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, ext) {
                return Some(found);
            }
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(ext))
        {
            return Some(path);
        }
    }
    None
}

fn input(var: &str, fallback_dir: &str, ext: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(var) {
        return Some(PathBuf::from(path));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    find_file(&home.join(fallback_dir), ext)
}

struct Plugins {
    amp: ClapBundle,
    ir: ClapBundle,
    reverb: ClapBundle,
    wavetable: ClapBundle,
    nam: PathBuf,
    cab: PathBuf,
}

impl Plugins {
    fn load() -> Option<Self> {
        let bundled = workspace_root().join("target/bundled");
        let bundle = |name: &str| {
            ClapBundle::load(&bundled.join(format!("{name}.clap")))
                .map_err(|e| eprintln!("skip: {name}.clap: {e:?} (run scripts/bundle.sh)"))
                .ok()
        };
        let nam = input(
            "RESONANCE_BENCH_NAM",
            ".local/share/resonance/amp-models",
            "nam",
        );
        let cab = input("RESONANCE_BENCH_IR", "Documents/Guitar/IR", "wav");
        let (Some(nam), Some(cab)) = (nam, cab) else {
            eprintln!("skip: no NAM model / cab IR (set RESONANCE_BENCH_NAM / RESONANCE_BENCH_IR)");
            return None;
        };
        Some(Self {
            amp: bundle("resonance-amp")?,
            ir: bundle("resonance-ir")?,
            reverb: bundle("resonance-reverb")?,
            wavetable: bundle("resonance-wavetable")?,
            nam,
            cab,
        })
    }

    fn instance(bundle: &ClapBundle, state: Option<(&str, &Path)>) -> PluginSlot {
        let id = bundle.descriptors()[0].id.clone();
        let mut inst = bundle.create_instance(&id, SR).expect("instance");
        if let Some((key, path)) = state {
            let saved = inst.save_state().expect("plugin has state");
            let mut json: serde_json::Value = serde_json::from_slice(&saved).expect("JSON state");
            assert!(
                set_key(&mut json, key, path.to_string_lossy().as_ref()),
                "state has no `{key}`: {json}"
            );
            let bytes = serde_json::to_vec(&json).unwrap();
            assert!(inst.reload_with_state(&bytes), "state reload");
        }
        PluginSlot::new(inst)
    }
}

/// Set `key` wherever it appears in `value`.
fn set_key(value: &mut serde_json::Value, key: &str, to: &str) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            let mut found = false;
            for (k, v) in map.iter_mut() {
                if k == key {
                    *v = serde_json::Value::String(to.to_owned());
                    found = true;
                } else {
                    found |= set_key(v, key, to);
                }
            }
            found
        }
        serde_json::Value::Array(items) => {
            items.iter_mut().fold(false, |f, v| set_key(v, key, to) | f)
        }
        _ => false,
    }
}

fn noise(len: usize, seed: u32) -> Vec<f32> {
    let mut s = seed | 1;
    (0..len)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((s >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.5
        })
        .collect()
}

fn guitar_clip(id: ClipId, blocks: usize) -> AudioClip {
    AudioClip {
        id,
        track_id: id,
        start_sample: 0,
        source: ClipSource::memory(noise(BLOCK * blocks * 2, id as u32 * 31 + 7)),
        name: format!("gtr{id}"),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::Linear,
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::Linear,
        gain_db: 0.0,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: WarpAlgorithm::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

/// Four-note chords, one per beat, held for the whole beat.
fn chord_clip(id: ClipId, track_id: TrackId, blocks: usize) -> MidiClip {
    let beat = TICKS_PER_QUARTER_NOTE as u64;
    let beats = (blocks * BLOCK) as u64 * 2 / SR as u64 + 2;
    let notes = (0..beats)
        .flat_map(|b| {
            let root = 48 + ((b * 5 + track_id) % 12) as u8;
            [0u8, 4, 7, 11].map(move |i| MidiNote {
                note: root + i,
                velocity: 0.8,
                start_tick: b * beat,
                duration_ticks: beat,
            })
        })
        .collect();
    MidiClip {
        id,
        track_id,
        start_sample: 0,
        duration_ticks: beats * beat,
        notes,
        name: format!("chords{id}"),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    }
}

fn project(plugins: &Plugins, blocks: usize) -> MixAudioHarness {
    let mut tracks = Vec::new();
    let mut slots: Vec<(PluginInstanceId, PluginSlot)> = Vec::new();
    let mut next_plugin = 1000u64;
    let mut add = |track: &mut Track, slot: PluginSlot| {
        next_plugin += 1;
        track.push_plugin(next_plugin);
        slots.push((next_plugin, slot));
    };
    for id in 1..=guitars() {
        let mut t = Track::new(id, format!("guitar {id}"));
        add(
            &mut t,
            Plugins::instance(&plugins.amp, Some(("model_path", &plugins.nam))),
        );
        add(
            &mut t,
            Plugins::instance(&plugins.ir, Some(("ir_path", &plugins.cab))),
        );
        add(&mut t, Plugins::instance(&plugins.reverb, None));
        t.set_volume(0.3);
        tracks.push(t);
    }
    let mut midi = Vec::new();
    for n in 0..SYNTHS {
        let id = 100 + n;
        let mut t = Track::with_type(id, format!("synth {n}"), TrackType::Instrument);
        add(&mut t, Plugins::instance(&plugins.wavetable, None));
        t.set_volume(0.2);
        tracks.push(t);
        midi.push(chord_clip(200 + n, id, blocks));
    }
    let clips = (1..=guitars()).map(|id| guitar_clip(id, blocks)).collect();
    let h = MixAudioHarness::new(
        tracks,
        Vec::new(),
        clips,
        midi,
        Vec::new(),
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    for (id, slot) in slots {
        h.edit_plugins(|p| p.insert(id, Arc::new(slot)));
    }
    h.shared().playing.store(true, Ordering::Relaxed);
    h
}

fn main() {
    // `cargo bench` passes `--bench`; ignore arguments.
    let Some(plugins) = Plugins::load() else {
        return;
    };
    let blocks: usize = std::env::var("RESONANCE_BENCH_BLOCKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4000);
    let threads: Vec<usize> = std::env::var("RESONANCE_BENCH_THREADS")
        .ok()
        .map(|v| v.split(',').filter_map(|t| t.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![1, 2, 4, 8]);
    let budget_us = BLOCK as f64 / SR as f64 * 1e6;
    println!(
        "render_pool stress: {} x (amp + cab IR + reverb) + {SYNTHS} x wavetable, \
         q{BLOCK} @ {SR} Hz, budget {budget_us:.0} µs, {blocks} blocks",
        guitars()
    );
    println!(
        "model: {}\ncab:   {}",
        plugins.nam.display(),
        plugins.cab.display()
    );
    println!("physical cores: {}", physical_cores());
    println!(
        "{:>7} {:>9} {:>9} {:>9} {:>6} {:>10} {:>8} {:>10}",
        "threads", "mean µs", "p99 µs", "max µs", "over", "crit µs", "effic", "join µs"
    );
    let warmup = 200;
    for &t in &threads {
        let mut h = project(&plugins, blocks + warmup + 16);
        h.set_render_threads(t, 0);
        for _ in 0..warmup {
            h.render();
        }
        h.take_pass_stats();
        let mut times = Vec::with_capacity(blocks);
        let (mut jobs, mut capacity, mut join, mut crit) = (0u64, 0u64, 0u64, 0u64);
        for _ in 0..blocks {
            let start = Instant::now();
            h.render();
            times.push(start.elapsed().as_secs_f64() * 1e6);
            let s = h.take_pass_stats();
            jobs += s.jobs_ns;
            capacity += s.wall_ns * s.threads as u64;
            join += s.join_wait_ns;
            crit = crit.max(s.critical_ns);
        }
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        let over = times.iter().filter(|&&us| us > budget_us).count();
        let mut sorted = times.clone();
        sorted.sort_by(f64::total_cmp);
        let p99 = sorted[(sorted.len() * 99 / 100).min(sorted.len() - 1)];
        let max = *sorted.last().unwrap();
        println!(
            "{t:>7} {mean:>9.0} {p99:>9.0} {max:>9.0} {over:>6} {:>10.0} {:>7.0}% {:>10.1}",
            crit as f64 / 1e3,
            jobs as f64 / capacity.max(1) as f64 * 100.0,
            join as f64 / blocks as f64 / 1e3,
        );
    }
}
