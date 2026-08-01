//! Per-block cost of the realtime render path.
//!
//! Every benchmark here models one 128-frame audio callback at 48 kHz —
//! the engine's real quantum, a 2.667 ms budget. Numbers are therefore
//! directly comparable to the `dsp load` telemetry: 26.7 µs == 1 % load.
//!
//! The harness project carries **no CLAP plugin instances**, so what is
//! measured is pure engine overhead: MIDI event collection, clip
//! readback, gain ramps / summing, latency compensation and metering.
//! Plugin DSP is out of scope (it lives in `plugins/`).

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use resonance_audio::__test_support::{
    mix_track_clips, sum_to_output, sum_to_stereo, RenderBenchHarness,
};
use resonance_audio::{collect_midi_events_bounce, ABMeterTap};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const QUANTUM: usize = 128;

/// 272 bars of 4/4 at 120 BPM.
const PROJECT_BARS: u32 = 272;

fn tempo_map() -> TempoMap {
    let mut tm = TempoMap::default();
    tm.bpm = 120.0;
    tm.numerator = 4;
    tm.denominator = 4;
    tm.rebuild_bar_table(SR);
    tm
}

fn project_frames() -> u64 {
    // 272 bars * 4 beats * 0.5 s * 48 kHz
    PROJECT_BARS as u64 * 4 * SR as u64 / 2
}

// ---------------------------------------------------------------------------
// builders
// ---------------------------------------------------------------------------

/// One MIDI clip spanning the whole project with `notes` evenly spread
/// notes — the shape an arrangement-length generated part has.
fn midi_clip(track_id: TrackId, id: u64, notes: usize) -> MidiClip {
    let total_ticks = PROJECT_BARS as u64 * 4 * TICKS_PER_QUARTER_NOTE as u64;
    let step = (total_ticks / notes.max(1) as u64).max(1);
    MidiClip {
        id,
        track_id,
        start_sample: 0,
        duration_ticks: total_ticks,
        notes: (0..notes)
            .map(|i| MidiNote {
                note: 36 + (i % 24) as u8,
                velocity: 0.8,
                start_tick: i as u64 * step,
                duration_ticks: step / 2,
            })
            .collect(),
        name: format!("clip{id}"),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    }
}

fn audio_clip(track_id: TrackId, id: u64, start: u64, frames: usize) -> AudioClip {
    AudioClip {
        id,
        track_id,
        start_sample: start,
        source: ClipSource::Memory(
            (0..frames * 2).map(|i| (i as f32 * 0.001).sin()).collect(),
        ),
        name: format!("audio{id}"),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: WarpAlgorithm::default(),
        warp_markers: Vec::new(),
    }
}

/// The reported project: 6 instrument tracks (5 synths + drums) and 2
/// vocal audio tracks, routed through 4 busses.
fn realistic_harness(notes_per_track: usize) -> RenderBenchHarness {
    let mut tracks = Vec::new();
    let mut midi_clips = Vec::new();
    let mut clips = Vec::new();
    let mut busses = Vec::new();

    for b in 0..4u64 {
        busses.push(Bus::new(b + 1, format!("bus{b}")));
    }

    for t in 0..6u64 {
        let id = t + 1;
        let track = Track::with_type(id, format!("inst{t}"), TrackType::Instrument);
        track.set_output(TrackOutput::Bus((t % 4) + 1));
        tracks.push(track);
        midi_clips.push(midi_clip(id, id, notes_per_track));
    }
    for t in 6..8u64 {
        let id = t + 1;
        let track = Track::with_type(id, format!("vox{t}"), TrackType::Audio);
        track.set_output(TrackOutput::Bus((t % 4) + 1));
        tracks.push(track);
        // Four takes across the arrangement.
        for k in 0..4u64 {
            clips.push(audio_clip(
                id,
                id * 100 + k,
                k * SR as u64 * 60,
                SR as usize * 50,
            ));
        }
    }

    RenderBenchHarness::new(
        tracks,
        busses,
        clips,
        midi_clips,
        Vec::new(),
        tempo_map(),
        QUANTUM,
        SR,
    )
}

// ---------------------------------------------------------------------------
// benches
// ---------------------------------------------------------------------------

/// Timeline MIDI event collection for one block. Scales with the number
/// of notes *in the whole clip*, not the number landing in the block.
fn bench_collect_midi(c: &mut Criterion) {
    let tm = tempo_map();
    let mut g = c.benchmark_group("collect_midi_events");
    for &notes in &[128usize, 512, 2048, 8192] {
        let clips = vec![midi_clip(1, 1, notes)];
        let mut out = Vec::with_capacity(4096);
        g.throughput(Throughput::Elements(notes as u64));
        g.bench_with_input(BenchmarkId::from_parameter(notes), &notes, |b, _| {
            let mut playhead = 0u64;
            b.iter(|| {
                collect_midi_events_bounce(
                    black_box(&clips),
                    1,
                    playhead,
                    QUANTUM,
                    black_box(&tm),
                    SR,
                    &mut out,
                );
                playhead = (playhead + QUANTUM as u64) % project_frames();
                black_box(out.len())
            });
        });
    }
    g.finish();
}

/// Audio clip readback for one block, over a track carrying `n` clips.
fn bench_mix_clips(c: &mut Criterion) {
    let mut g = c.benchmark_group("mix_track_clips");
    for &n in &[1usize, 8, 32, 128] {
        let clips: Vec<AudioClip> = (0..n as u64)
            .map(|k| audio_clip(1, k, k * SR as u64 * 4, SR as usize * 4))
            .collect();
        let mut l = vec![0.0f32; QUANTUM];
        let mut r = vec![0.0f32; QUANTUM];
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            let mut playhead = 0u64;
            b.iter(|| {
                black_box(mix_track_clips(
                    black_box(&clips),
                    1,
                    playhead,
                    QUANTUM,
                    &mut l,
                    &mut r,
                ));
                playhead = (playhead + QUANTUM as u64) % (SR as u64 * 60);
            });
        });
    }
    g.finish();
}

/// A/B loudness metering of the master output — fed once per block from
/// `mix_audio`. Feed only (K-weighting + true-peak oversampling); this
/// part is O(frames) and session-length independent.
fn bench_ab_meter(c: &mut Criterion) {
    let mut g = c.benchmark_group("ab_meter_feed");
    let block: Vec<f32> = (0..QUANTUM * 2).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
    let mut tap = ABMeterTap::new(SR as f32);
    g.bench_function("feed_only", |b| {
        b.iter(|| tap.feed_interleaved(black_box(&block), 2, QUANTUM))
    });
    g.finish();
}

/// The gated-integration readout alone (the part that grows with
/// session length), separated from the per-sample filter work.
fn bench_ab_meter_snapshot(c: &mut Criterion) {
    let mut g = c.benchmark_group("ab_meter_snapshot_only");
    let block: Vec<f32> = (0..QUANTUM * 2).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
    for &session_secs in &[0u64, 60, 600] {
        let mut tap = ABMeterTap::new(SR as f32);
        let warm_blocks = session_secs * SR as u64 / QUANTUM as u64;
        for _ in 0..warm_blocks {
            tap.feed_interleaved(&block, 2, QUANTUM);
        }
        g.bench_with_input(
            BenchmarkId::from_parameter(session_secs),
            &session_secs,
            |b, _| b.iter(|| black_box(tap.snapshot())),
        );
    }
    g.finish();
}

/// Per-sample summing helpers, the innermost mixing loops.
fn bench_summing(c: &mut Criterion) {
    let src_l = vec![0.5f32; QUANTUM];
    let src_r = vec![-0.25f32; QUANTUM];
    let mut dst_l = vec![0.0f32; QUANTUM];
    let mut dst_r = vec![0.0f32; QUANTUM];
    let mut out = vec![0.0f32; QUANTUM * 2];

    let mut g = c.benchmark_group("summing");
    g.bench_function("sum_to_stereo", |b| {
        b.iter(|| {
            sum_to_stereo(
                &mut dst_l,
                &mut dst_r,
                QUANTUM,
                black_box(&src_l),
                black_box(&src_r),
                (0.5, 0.7),
                (0.5, 0.7),
            )
        })
    });
    g.bench_function("sum_to_output", |b| {
        b.iter(|| {
            sum_to_output(
                &mut out,
                2,
                QUANTUM,
                black_box(&src_l),
                black_box(&src_r),
                (0.5, 0.7),
                (0.5, 0.7),
            )
        })
    });
    g.finish();
}

/// One whole live render block for the reported project shape, with no
/// plugin instances — i.e. the engine's own per-block overhead.
fn bench_render_block(c: &mut Criterion) {
    let mut g = c.benchmark_group("render_block_project");
    for &notes in &[0usize, 256, 1024, 4096] {
        let mut h = realistic_harness(notes);
        g.bench_with_input(BenchmarkId::from_parameter(notes), &notes, |b, _| {
            let mut playhead = 0u64;
            b.iter(|| {
                black_box(h.render(playhead)[0]);
                playhead = (playhead + QUANTUM as u64) % project_frames();
            });
        });
    }
    g.finish();
}

criterion_group!(
    benches,
    bench_collect_midi,
    bench_mix_clips,
    bench_ab_meter,
    bench_ab_meter_snapshot,
    bench_summing,
    bench_render_block,
);
criterion_main!(benches);
