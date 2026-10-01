//! Disk streaming (drums-plugin-rework.md §7 E14, slice K6b).
//!
//! A long take keeps only its head in memory and streams its tail from
//! the WAV file through a per-voice ring that a reader pool fills. These
//! tests pin what that must never change and what it must survive:
//!
//! - a streamed render is **bit-identical** to a fully resident render of
//!   the same MIDI, at the file's rate and through the resampler, live
//!   (never waiting, the reader stepped between blocks) and offline (as
//!   fast as the CPU goes, the host having declared it);
//! - a 64-voice saturation pattern at 128-frame blocks, paced in real
//!   time with every read slowed like a cold page cache, plays with zero
//!   underruns — at the file rate and through the resampler (ignored by
//!   default: wall-clock bound, see the tests for how to run them);
//! - a stalled reader costs silence for exactly the missing frames, is
//!   counted, and the voice recovers in time once the reader is back;
//! - a reader that is shut down mid-render never blocks the audio thread
//!   (watchdog), through steals, chokes, kit swaps and resets;
//! - only the heads are resident.
//!
//! Every test that compares two renders also checks they are not silent
//! (`feedback_silent_goldens_are_vacuous`).
//!
//! Nothing that runs by default depends on how fast this machine is or
//! how loaded: the reader is a stepped one the test drives, the render
//! is declared offline (so it waits for the reader as long as it takes),
//! or the timing detector runs on a clock the test sets. The suite is
//! checked by running this binary as eight copies at once.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use resonance_drums::drum_map::{self, NUM_PADS, PAD_MAPPINGS};
use resonance_drums::dsp::{DrumSampler, Hit, PortBuffers};
use resonance_drums::kit::{
    LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer, NUM_OUTPUT_PORTS,
};
use resonance_drums::kit_loader::cache::SampleCache;
use resonance_drums::params::DrumParams;
use resonance_drums::stream::reader::ReaderPool;
use resonance_drums::stream::{RenderMode, NUM_RINGS};

const HOST: f32 = 48_000.0;

// ---------------------------------------------------------------------------
// Fixture kit: long WAV files on disk.
// ---------------------------------------------------------------------------

/// A deterministic, never-silent signal: two detuned partials under a
/// slow decay, plus a little hash noise, so every frame differs.
fn signal(frame: usize, frames: usize, channel: usize, seed: usize) -> f64 {
    let t = frame as f64;
    let decay = 1.0 - 0.6 * (frame as f64 / frames as f64);
    let noise = ((frame.wrapping_mul(2_654_435_761) ^ (seed * 7919 + channel * 31)) % 2048) as f64
        / 2048.0
        - 0.5;
    decay
        * (0.35 * (t * (0.013 + seed as f64 * 0.001) + channel as f64).sin()
            + 0.2 * (t * 0.0027).sin()
            + 0.08 * noise)
}

/// Write a 24-bit PCM WAV of `frames` frames.
fn write_wav(path: &Path, channels: u16, rate: u32, frames: usize, seed: usize) {
    let block_align = channels as usize * 3;
    let mut data = Vec::with_capacity(frames * block_align);
    for f in 0..frames {
        for ch in 0..channels as usize {
            let s = (signal(f, frames, ch, seed) * 8_388_607.0) as i32;
            data.extend_from_slice(&s.to_le_bytes()[..3]);
        }
    }
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * block_align as u32).to_le_bytes());
    out.extend_from_slice(&(block_align as u16).to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    std::fs::File::create(path)
        .unwrap()
        .write_all(&out)
        .unwrap();
}

/// Six takes at `rate`: close mics alternate mono / stereo, overheads are
/// stereo. `seconds` long each.
struct Fixture {
    dir: PathBuf,
    close: Vec<PathBuf>,
    overhead: Vec<PathBuf>,
}

impl Fixture {
    fn new(tag: &str, rate: u32, seconds: f32) -> Self {
        let dir =
            std::env::temp_dir().join(format!("drums-streaming-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let frames = (rate as f32 * seconds) as usize;
        let close = (0..4)
            .map(|i| {
                let p = dir.join(format!("close{i}.wav"));
                // Lengths differ a little so takes end at different times.
                write_wav(
                    &p,
                    if i % 2 == 0 { 1 } else { 2 },
                    rate,
                    frames - i * 997,
                    i,
                );
                p
            })
            .collect();
        let overhead = (0..2)
            .map(|i| {
                let p = dir.join(format!("oh{i}.wav"));
                write_wav(&p, 2, rate, frames - i * 1_301, 10 + i);
                p
            })
            .collect();
        Self {
            dir,
            close,
            overhead,
        }
    }

    /// The kit: every pad has two close banks and an overhead, one layer,
    /// one take each — three voices a hit. `preload` 0 keeps every take
    /// whole; otherwise longer takes stream.
    fn kit(&self, cache: &SampleCache, preload: u32) -> Vec<LoadedPad> {
        let take = |path: &Path| -> LoadedSample {
            let (data, _) = cache.get_or_decode_preload(path, HOST, preload).unwrap();
            LoadedSample::from_shared(data)
        };
        let bank = |name: &str, path: &Path| LoadedMicBank {
            position: name.to_string(),
            setup_key: String::new(),
            layers: vec![VelocityLayer::new(vec![take(path)])],
        };
        PAD_MAPPINGS
            .iter()
            .enumerate()
            .map(|(i, m)| LoadedPad {
                name: m.name.to_string(),
                choke_group: m.choke_group,
                output_group: m.output_group,
                close_mics: vec![
                    bank("A", &self.close[i % 4]),
                    bank("B", &self.close[(i + 1) % 4]),
                ],
                extra_banks: Vec::new(),
                overhead: Some(bank("OH", &self.overhead[i % 2])),
            })
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ---------------------------------------------------------------------------
// Rendering.
// ---------------------------------------------------------------------------

/// A sampler on `pads`, read by `pool`, in `mode`. The sender keeps the
/// mailbox open (and swaps kits in).
fn sampler(
    pads: Vec<LoadedPad>,
    pool: &Arc<ReaderPool>,
    mode: RenderMode,
) -> (DrumSampler, crossbeam_channel::Sender<Vec<LoadedPad>>) {
    let (tx, rx) = crossbeam_channel::bounded::<Vec<LoadedPad>>(1);
    let mut s = DrumSampler::with_reader_pool(rx, pool);
    s.set_sample_rate(HOST);
    s.set_render_mode(mode);
    s.pads = pads;
    (s, tx)
}

/// Per-port output buffers for one block.
struct Ports {
    bufs: Vec<(Vec<f32>, Vec<f32>)>,
}

impl Ports {
    fn new(frames: usize) -> Self {
        Self {
            bufs: (0..NUM_OUTPUT_PORTS)
                .map(|_| (vec![0.0; frames], vec![0.0; frames]))
                .collect(),
        }
    }

    fn render(&mut self, s: &mut DrumSampler, frames: usize, params: &DrumParams, hits: &[Hit]) {
        let mut views: Vec<PortBuffers<'_>> = self
            .bufs
            .iter_mut()
            .map(|(l, r)| PortBuffers { left: l, right: r })
            .collect();
        s.render_block(&mut views, frames, params, hits);
    }

    /// Append every port's block, as bits, to `out`.
    fn append_bits(&self, frames: usize, out: &mut Vec<u32>) {
        for (l, r) in &self.bufs {
            out.extend(l[..frames].iter().map(|s| s.to_bits()));
            out.extend(r[..frames].iter().map(|s| s.to_bits()));
        }
    }

    /// The mix of every port, left channel.
    fn mix_left(&self, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| self.bufs.iter().map(|(l, _)| l[i]).sum())
            .collect()
    }
}

/// The hits of `block` in a pattern: every pad struck in turn, `per_block`
/// hits a block, at spread-out offsets — enough to keep 64 voices busy
/// and steal constantly. Hats (a choke group) come round too.
fn pattern(block: usize, frames: usize, per_block: usize, blocks_until_quiet: usize) -> Vec<Hit> {
    if block >= blocks_until_quiet {
        return Vec::new();
    }
    (0..per_block)
        .map(|k| {
            let n = block * per_block + k;
            Hit {
                frame: (k * frames / per_block + (n * 37) % 11).min(frames - 1),
                note: PAD_MAPPINGS[(n * 7) % NUM_PADS].note,
                velocity: 0.3 + 0.7 * ((n * 13) % 10) as f32 / 10.0,
            }
        })
        .collect()
}

/// Render `blocks` blocks of `frames`; with `pump`, the (stepped) reader
/// catches up after each block, as a reader that keeps up would.
fn render(
    s: &mut DrumSampler,
    blocks: usize,
    frames: usize,
    per_block: usize,
    quiet_after: usize,
    pump: Option<&ReaderPool>,
) -> Vec<u32> {
    let params = DrumParams::default();
    let mut ports = Ports::new(frames);
    let mut out = Vec::with_capacity(blocks * frames * NUM_OUTPUT_PORTS * 2);
    for b in 0..blocks {
        let hits = pattern(b, frames, per_block, quiet_after);
        ports.render(s, frames, &params, &hits);
        ports.append_bits(frames, &mut out);
        if let Some(pool) = pump {
            pool.pump();
        }
    }
    out
}

fn loud(bits: &[u32]) -> f32 {
    bits.iter()
        .map(|b| f32::from_bits(*b).abs())
        .fold(0.0, f32::max)
}

/// The first index two renders differ at, as (block, port, frame).
fn first_difference(a: &[u32], b: &[u32], frames: usize) -> Option<(usize, usize, usize)> {
    let i = a.iter().zip(b).position(|(x, y)| x != y)?;
    let per_block = frames * NUM_OUTPUT_PORTS * 2;
    Some((i / per_block, (i % per_block) / (frames * 2), i % frames))
}

/// Takes in `pads` that stream, and every take.
fn streamed_takes(pads: &[LoadedPad]) -> (usize, usize) {
    let takes: Vec<&LoadedSample> = pads
        .iter()
        .flat_map(|p| p.close_mics.iter().chain(p.overhead.iter()))
        .flat_map(|b| b.layers.iter().flat_map(|l| l.round_robins.iter()))
        .collect();
    let streamed = takes.iter().filter(|t| t.tail().is_some()).count();
    (streamed, takes.len())
}

// ---------------------------------------------------------------------------
// Bit identity.
// ---------------------------------------------------------------------------

/// How a bit-identity render streams.
#[derive(Clone, Copy, PartialEq)]
enum Streaming {
    /// Reader threads; the host declares offline rendering, so every
    /// missing frame is waited for.
    DeclaredOffline,
    /// Real time — never a wait — with a stepped reader that catches up
    /// between blocks.
    LiveStepped,
}

/// Streamed and resident renders of the saturation pattern, compared bit
/// for bit; files at `file_rate`, host at 48 kHz.
fn assert_bit_identical(tag: &str, file_rate: u32, preload: u32, how: Streaming) {
    let fixture = Fixture::new(tag, file_rate, 1.6);
    let cache = SampleCache::new();
    let resident = fixture.kit(&cache, 0);
    let streamed = fixture.kit(&cache, preload);
    let (n, all) = streamed_takes(&streamed);
    assert_eq!(
        n, all,
        "{tag}: every take is longer than the preload and streams"
    );
    assert_eq!(streamed_takes(&resident).0, 0);

    let pool = match how {
        Streaming::DeclaredOffline => ReaderPool::new(2),
        Streaming::LiveStepped => ReaderPool::stepped(),
    };
    // 128-frame blocks, two hits a block for 0.8 s, then 0.9 s of tails.
    const FRAMES: usize = 128;
    let blocks = (1.7 * HOST) as usize / FRAMES;
    let quiet_after = (0.8 * HOST) as usize / FRAMES;
    let (mut a, _ta) = sampler(resident, &pool, RenderMode::Realtime);
    let reference = render(&mut a, blocks, FRAMES, 2, quiet_after, None);
    let got = match how {
        Streaming::DeclaredOffline => {
            let (mut b, _tb) = sampler(streamed, &pool, RenderMode::Auto);
            b.set_host_render_mode(Arc::new(std::sync::atomic::AtomicU8::new(
                resonance_drums::stream::HOST_RENDER_OFFLINE,
            )));
            let got = render(&mut b, blocks, FRAMES, 2, quiet_after, None);
            assert!(b.renders_offline());
            (got, b.stream_underruns(), b.stream_ring_misses())
        }
        Streaming::LiveStepped => {
            let (mut b, _tb) = sampler(streamed, &pool, RenderMode::Realtime);
            let got = render(&mut b, blocks, FRAMES, 2, quiet_after, Some(&pool));
            assert_eq!(b.stream_offline_waits(), 0);
            (got, b.stream_underruns(), b.stream_ring_misses())
        }
    };
    let (got, underruns, misses) = got;
    assert!(
        loud(&reference) > 0.05,
        "{tag}: the reference render is silent"
    );
    assert_eq!(
        underruns, 0,
        "{tag}: underruns ({misses} hits found no ring)"
    );
    if let Some((block, port, frame)) = first_difference(&reference, &got, FRAMES) {
        panic!("{tag}: streamed render differs from resident at block {block}, port {port}, frame {frame}");
    }
    pool.shutdown();
}

#[test]
fn streamed_render_is_bit_identical_offline_at_the_file_rate() {
    // A small preload: the tails are most of every take, and the rings
    // wrap many times.
    assert_bit_identical("offline-48k", 48_000, 4_096, Streaming::DeclaredOffline);
}

#[test]
fn streamed_render_is_bit_identical_offline_through_the_resampler() {
    assert_bit_identical("offline-44k1", 44_100, 4_096, Streaming::DeclaredOffline);
}

#[test]
fn streamed_render_is_bit_identical_live_at_the_file_rate() {
    assert_bit_identical("live-48k", 48_000, 32_768, Streaming::LiveStepped);
}

#[test]
fn streamed_render_is_bit_identical_live_through_the_resampler() {
    assert_bit_identical("live-96k", 96_000, 32_768, Streaming::LiveStepped);
}

#[test]
fn streamed_render_is_bit_identical_live_with_a_small_preload() {
    // The rings wrap many times, and every claim is within a ring of its
    // tail from the start.
    assert_bit_identical("live-44k1-small", 44_100, 4_096, Streaming::LiveStepped);
}

/// A clock the test drives, for the timing detector: `advance` moves it
/// on by a fraction of a block's audio length.
struct TestClock(Arc<std::sync::atomic::AtomicU64>);

impl TestClock {
    fn on(s: &mut DrumSampler) -> Self {
        let nanos = Arc::new(std::sync::atomic::AtomicU64::new(1_000_000_000));
        s.set_manual_clock(nanos.clone());
        Self(nanos)
    }

    /// Move on by `speed`-times-faster-than-real-time's worth of a block
    /// of `frames`.
    fn advance(&self, frames: usize, speed: f64) {
        let nanos = (frames as f64 / HOST as f64 / speed * 1e9) as u64;
        self.0
            .fetch_add(nanos, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Blocks of `frames` in one detector window (0.25 s of audio).
fn window_blocks(frames: usize) -> usize {
    (0.25 * HOST as f64 / frames as f64).ceil() as usize
}

/// A bounce renders as fast as it can and the host says nothing: the
/// sampler tells from the timing — on a clock the test drives, so the
/// verdict does not depend on the machine's load — and waits for a slow
/// reader instead of dropping frames. Only a short wait at first; the
/// long one once offline rendering is sustained.
///
/// The long budget is the declared-offline one here
/// (`set_auto_wait_budgets`): the reader is real threads and the budget
/// is wall-clock, and 50 ms a block ran out on a loaded machine (the
/// suite runs binaries side by side), which made the bit-identity and
/// zero-underrun claims below flaky. The verdicts are what this test is
/// about; the shipped 50 ms is a constant.
#[test]
fn an_unannounced_fast_render_waits_for_a_slow_reader() {
    use resonance_drums::dsp::sampler::{AUTO_WAIT_PER_BLOCK, OFFLINE_WAIT_PER_BLOCK};
    let fixture = Fixture::new("auto", 48_000, 2.0);
    let cache = SampleCache::new();
    let resident = fixture.kit(&cache, 0);
    let streamed = fixture.kit(&cache, 32_768);
    let pool = ReaderPool::new(2);
    const FRAMES: usize = 1_024;
    let blocks = (3.0 * HOST) as usize / FRAMES;
    // The hits start once the verdict is sustained (three windows), so
    // every tail frame is waited for with the long budget.
    let first_hit = 4 * window_blocks(FRAMES);
    let hits = |b: usize| -> Vec<Hit> {
        if (first_hit..first_hit + 40).contains(&b) {
            pattern(b, FRAMES, 1, usize::MAX)
        } else {
            Vec::new()
        }
    };
    let params = DrumParams::default();
    let (mut a, _ta) = sampler(resident, &pool, RenderMode::Realtime);
    let mut ports = Ports::new(FRAMES);
    let mut reference = Vec::new();
    for b in 0..blocks {
        ports.render(&mut a, FRAMES, &params, &hits(b));
        ports.append_bits(FRAMES, &mut reference);
    }
    let (mut s, _ts) = sampler(streamed, &pool, RenderMode::Auto);
    s.set_auto_wait_budgets(AUTO_WAIT_PER_BLOCK, OFFLINE_WAIT_PER_BLOCK);
    let clock = TestClock::on(&mut s);
    // Every read takes 1 ms: far slower than an unthrottled render.
    s.stream_set().set_read_latency_us(1_000);
    let mut got = Vec::new();
    let mut verdicts = Vec::new();
    for b in 0..blocks {
        clock.advance(FRAMES, 20.0);
        ports.render(&mut s, FRAMES, &params, &hits(b));
        ports.append_bits(FRAMES, &mut got);
        verdicts.push(s.block_wait_budget());
    }
    let w = window_blocks(FRAMES);
    // The first block starts a window; a window is judged at the start
    // of the block after its last, `w` blocks on.
    assert!(
        verdicts[..w].iter().all(|b| b.is_zero()),
        "live until a window proves otherwise"
    );
    assert_eq!(verdicts[w], AUTO_WAIT_PER_BLOCK, "offline after one window");
    assert_eq!(verdicts[3 * w - 1], AUTO_WAIT_PER_BLOCK);
    assert_eq!(
        verdicts[3 * w],
        OFFLINE_WAIT_PER_BLOCK,
        "sustained after three"
    );
    assert!(verdicts[3 * w..]
        .iter()
        .all(|&b| b == OFFLINE_WAIT_PER_BLOCK));
    assert!(loud(&reference) > 0.05);
    // How often it had to wait depends on how fast the reader threads get
    // the CPU (on a loaded machine, the render is slow enough not to);
    // that it waits at all is pinned below, under a declared mode.
    eprintln!("auto: waited {} times", s.stream_offline_waits());
    assert_eq!(s.stream_underruns(), 0);
    if let Some((block, port, frame)) = first_difference(&reference, &got, FRAMES) {
        panic!("differs at block {block}, port {port}, frame {frame}");
    }
}

/// Declared offline, a render waits for a reader that is stalled when it
/// needs a frame — for as long as the reader takes (here, until another
/// thread lets it go) — and so loses nothing.
#[test]
fn a_declared_offline_render_waits_for_a_stalled_reader() {
    let fixture = Fixture::new("offline-stall", 48_000, 1.0);
    let cache = SampleCache::new();
    let path = &fixture.overhead[0];
    let pool = ReaderPool::new(1);
    let (mut a, _ta) = sampler(single_voice_kit(&cache, path, 0), &pool, RenderMode::Realtime);
    let reference = one_hit(&mut a, 300, |_, _| {});
    let (mut s, _t) = sampler(
        single_voice_kit(&cache, path, 8_192),
        &pool,
        RenderMode::Auto,
    );
    s.set_host_render_mode(Arc::new(std::sync::atomic::AtomicU8::new(
        resonance_drums::stream::HOST_RENDER_OFFLINE,
    )));
    // The reader is stalled from the start; it is let go only once the
    // render has had to wait for it.
    let set = s.stream_set().clone();
    set.set_paused(true);
    let releaser = std::thread::spawn(move || {
        while set.offline_waits() == 0 {
            std::thread::sleep(Duration::from_millis(1));
        }
        set.set_paused(false);
    });
    let got = one_hit(&mut s, 300, |_, _| {});
    releaser.join().unwrap();
    assert!(s.stream_offline_waits() > 0, "the render waited");
    assert_eq!(s.stream_underruns(), 0);
    assert!(loud_f32(&reference) > 0.05);
    assert_eq!(got, reference);
}

fn loud_f32(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, s| m.max(s.abs()))
}

/// A render paced like a live callback — jitter included — is never
/// taken for offline, and never waits.
#[test]
fn a_paced_render_is_not_taken_for_offline() {
    let fixture = Fixture::new("paced", 48_000, 0.5);
    let cache = SampleCache::new();
    let pool = ReaderPool::stepped();
    let (mut s, _t) = sampler(fixture.kit(&cache, 4_096), &pool, RenderMode::Auto);
    let clock = TestClock::on(&mut s);
    let params = DrumParams::default();
    let mut ports = Ports::new(128);
    for b in 0..(2.0 * HOST) as usize / 128 {
        // Early and late blocks in turn, real time on average.
        clock.advance(128, if b % 2 == 0 { 1.6 } else { 0.73 });
        ports.render(&mut s, 128, &params, &pattern(b, 128, 1, usize::MAX));
        assert!(!s.renders_offline(), "block {b} taken for offline");
        assert!(s.block_wait_budget().is_zero());
    }
    assert_eq!(s.stream_offline_waits(), 0);
}

/// An offline verdict ends on the first block that comes at a live pace
/// (live playback resuming after a bounce), after a gap, and after a
/// reset — before that block renders — and has to be earned afresh.
#[test]
fn an_offline_verdict_ends_on_the_first_live_block() {
    use resonance_drums::dsp::sampler::AUTO_WAIT_PER_BLOCK;
    let pool = ReaderPool::stepped();
    let (mut s, _t) = sampler(Vec::new(), &pool, RenderMode::Auto);
    let clock = TestClock::on(&mut s);
    let params = DrumParams::default();
    let mut ports = Ports::new(256);
    let w = window_blocks(256);
    let mut block = |s: &mut DrumSampler, speed: f64| {
        clock.advance(256, speed);
        ports.render(s, 256, &params, &[]);
        s.block_wait_budget()
    };
    let fast_until_offline = |s: &mut DrumSampler, block: &mut dyn FnMut(&mut DrumSampler, f64) -> Duration| {
        for _ in 0..=w {
            block(s, 50.0);
        }
        assert!(s.renders_offline());
    };
    fast_until_offline(&mut s, &mut block);
    // One block at real pace: live at once.
    assert!(block(&mut s, 1.0).is_zero());
    assert!(!s.renders_offline());
    // Fast again: not offline until a whole new window says so (the live
    // block started it).
    for _ in 1..w {
        assert!(block(&mut s, 50.0).is_zero());
    }
    assert_eq!(block(&mut s, 50.0), AUTO_WAIT_PER_BLOCK);
    // A gap (the transport stopped for a second).
    clock.advance(HOST as usize, 1.0);
    assert!(block(&mut s, 50.0).is_zero());
    assert!(!s.renders_offline());
    fast_until_offline(&mut s, &mut block);
    // A slowish window (1.8x real time) ends the verdict too.
    for _ in 0..w {
        block(&mut s, 1.8);
    }
    assert!(block(&mut s, 1.8).is_zero());
    fast_until_offline(&mut s, &mut block);
    // A reset.
    s.reset();
    assert!(!s.renders_offline());
    assert!(block(&mut s, 50.0).is_zero());
}

/// What the host declares overrides the timing: offline waits (long),
/// real time never waits — not even for a reader that is stalled while
/// the blocks come faster than real time.
#[test]
fn the_host_declared_mode_overrides_the_timing() {
    use resonance_drums::dsp::sampler::OFFLINE_WAIT_PER_BLOCK;
    use resonance_drums::stream::{HOST_RENDER_OFFLINE, HOST_RENDER_REALTIME, HOST_RENDER_UNKNOWN};
    use std::sync::atomic::{AtomicU8, Ordering};
    let fixture = Fixture::new("host-mode", 48_000, 1.0);
    let cache = SampleCache::new();
    let pool = ReaderPool::stepped();
    let (mut s, _t) = sampler(
        single_voice_kit(&cache, &fixture.overhead[0], 8_192),
        &pool,
        RenderMode::Auto,
    );
    let host = Arc::new(AtomicU8::new(HOST_RENDER_REALTIME));
    s.set_host_render_mode(host.clone());
    let clock = TestClock::on(&mut s);
    let params = DrumParams::default();
    let mut ports = Ports::new(128);
    let hit = [Hit {
        frame: 0,
        note: drum_map::KICK,
        velocity: 1.0,
    }];
    // Real time, blocks 50x faster than real time, the reader stalled
    // (a stepped pool nobody pumps): never a wait, and the missing frames
    // are underruns.
    for b in 0..200 {
        clock.advance(128, 50.0);
        let t = Instant::now();
        ports.render(&mut s, 128, &params, if b == 0 { &hit } else { &[] });
        assert!(t.elapsed() < Duration::from_millis(250));
        assert!(!s.renders_offline());
        assert!(s.block_wait_budget().is_zero());
    }
    assert_eq!(s.stream_offline_waits(), 0);
    assert!(s.stream_underruns() > 0);
    // Offline, paced at real time: offline anyway, with the long budget.
    // (The reader catches up first: a stalled one would now be waited
    // for, for seconds.)
    pool.pump();
    host.store(HOST_RENDER_OFFLINE, Ordering::Relaxed);
    clock.advance(128, 1.0);
    ports.render(&mut s, 128, &params, &[]);
    assert!(s.renders_offline());
    assert_eq!(s.block_wait_budget(), OFFLINE_WAIT_PER_BLOCK);
    // Nothing declared again: the timing decides, from scratch.
    host.store(HOST_RENDER_UNKNOWN, Ordering::Relaxed);
    clock.advance(128, 50.0);
    ports.render(&mut s, 128, &params, &[]);
    assert!(!s.renders_offline());
    // A mode fixed on the sampler beats the host's.
    host.store(HOST_RENDER_OFFLINE, Ordering::Relaxed);
    s.set_render_mode(RenderMode::Realtime);
    ports.render(&mut s, 128, &params, &[]);
    assert!(!s.renders_offline());
}

// ---------------------------------------------------------------------------
// Saturation over a cold cache, and the reader's throughput.
//
// These run against the wall clock — real reader threads keeping up with
// a render paced in real time — so they are `#[ignore]`d: on a loaded
// machine (the suite runs binaries in parallel) a reader thread can be
// held off the CPU long enough to miss, which says nothing about the
// code. Run them on a quiet machine, in release:
//
//   cargo test --release -p resonance-drums --test streaming -- --ignored --nocapture
// ---------------------------------------------------------------------------

/// 64 voices busy and stealing at 128-frame blocks, paced in real time,
/// files at `file_rate` (44.1 kHz resamples every tail frame), every
/// read delayed 1 ms (a cold page cache on an SSD is ~0.1–0.2 ms per
/// random read; 1 ms is a slow one), read by a pool the size of the
/// process-wide one: no underruns, and bit-identical to the resident
/// render.
fn saturation(tag: &str, file_rate: u32) {
    let readers = resonance_drums::stream::reader::global_readers();
    let fixture = Fixture::new(tag, file_rate, 3.0);
    let cache = SampleCache::new();
    let resident = fixture.kit(&cache, 0);
    let streamed = fixture.kit(&cache, 32_768);
    let pool = ReaderPool::new(readers);
    const FRAMES: usize = 128;
    let blocks = (3.0 * HOST) as usize / FRAMES;
    let quiet_after = (2.0 * HOST) as usize / FRAMES;
    let (mut a, _ta) = sampler(resident, &pool, RenderMode::Realtime);
    // Two hits a block, three voices a hit: 64 voices fill within a
    // dozen blocks and every hit after steals.
    let reference = render(&mut a, blocks, FRAMES, 2, quiet_after, None);
    let (mut b, _tb) = sampler(streamed, &pool, RenderMode::Realtime);
    b.stream_set().set_read_latency_us(1_000);
    let mut peak_voices = 0;
    let params = DrumParams::default();
    let mut ports = Ports::new(FRAMES);
    let mut got = Vec::new();
    let began = Instant::now();
    let block_time = Duration::from_secs_f64(FRAMES as f64 / HOST as f64);
    let mut slowest = Duration::ZERO;
    for blk in 0..blocks {
        let due = began + block_time * blk as u32;
        let now = Instant::now();
        if due > now {
            std::thread::sleep(due - now);
        }
        let t = Instant::now();
        ports.render(
            &mut b,
            FRAMES,
            &params,
            &pattern(blk, FRAMES, 2, quiet_after),
        );
        slowest = slowest.max(t.elapsed());
        ports.append_bits(FRAMES, &mut got);
        peak_voices = peak_voices.max(b.voices.iter().filter(|v| v.active).count());
    }
    eprintln!(
        "{tag}: {readers} readers, slowest block {slowest:?}, {} underruns, {} rings used",
        b.stream_underruns(),
        b.stream_set().rings_allocated()
    );
    assert_eq!(peak_voices, 64, "the pattern saturates the voices");
    assert_eq!(
        b.stream_underruns(),
        0,
        "{tag}: underruns at saturation ({} of them hits that found no ring)",
        b.stream_ring_misses()
    );
    assert!(loud(&reference) > 0.05);
    assert!(
        first_difference(&reference, &got, FRAMES).is_none(),
        "{tag}: saturation render differs"
    );
    // The audio thread never waited on the 1 ms reads.
    assert!(
        slowest < Duration::from_millis(250),
        "{tag}: a block took {slowest:?}"
    );
    pool.shutdown();
}

#[test]
#[ignore = "wall-clock bound: run in release with -- --ignored (see the section comment)"]
fn saturation_over_a_cold_cache_has_no_underruns() {
    saturation("saturation-48k", 48_000);
}

#[test]
#[ignore = "wall-clock bound: run in release with -- --ignored (see the section comment)"]
fn saturation_through_the_resampler_has_no_underruns() {
    saturation("saturation-44k1", 44_100);
}

/// What one reader thread costs at saturation: the 64-voice pattern with
/// 44.1 kHz files (every tail frame resampled to 48 kHz) and a warm page
/// cache, rendered with a stepped reader whose passes are timed. The
/// headroom is how many times real time one reader thread could stream
/// that load; the process-wide pool has `global_readers()` of them.
#[test]
#[ignore = "a measurement: run in release with -- --ignored --nocapture"]
fn reader_throughput_at_saturation() {
    for file_rate in [44_100, 48_000] {
        let fixture = Fixture::new(&format!("throughput-{file_rate}"), file_rate, 3.0);
        let cache = SampleCache::new();
        let pool = ReaderPool::stepped();
        let (mut s, _t) = sampler(fixture.kit(&cache, 32_768), &pool, RenderMode::Realtime);
        const FRAMES: usize = 128;
        let blocks = (3.0 * HOST) as usize / FRAMES;
        let quiet_after = (2.0 * HOST) as usize / FRAMES;
        let params = DrumParams::default();
        let mut ports = Ports::new(FRAMES);
        let mut reading = Duration::ZERO;
        let mut rendering = Duration::ZERO;
        let mut streaming_voices = 0usize;
        for blk in 0..blocks {
            let t = Instant::now();
            ports.render(&mut s, FRAMES, &params, &pattern(blk, FRAMES, 2, quiet_after));
            rendering += t.elapsed();
            streaming_voices += s
                .voices
                .iter()
                .filter(|v| v.active && v.ring != resonance_drums::stream::NO_RING)
                .count();
            let t = Instant::now();
            pool.pump();
            reading += t.elapsed();
        }
        let audio = blocks as f64 * FRAMES as f64 / HOST as f64;
        eprintln!(
            "reader throughput, {file_rate} Hz files -> 48 kHz: {audio:.1} s of audio at {:.1} \
             streaming voices on average, read in {reading:.2?} by one thread (headroom {:.1}x \
             real time), rendered in {rendering:.2?}; {} underruns",
            streaming_voices as f64 / blocks as f64,
            audio / reading.as_secs_f64(),
            s.stream_underruns()
        );
        assert_eq!(s.stream_underruns(), 0);
    }
}

// ---------------------------------------------------------------------------
// Stalls and a dead reader.
// ---------------------------------------------------------------------------

/// One voice, one take, played on its own.
fn single_voice_kit(cache: &SampleCache, path: &Path, preload: u32) -> Vec<LoadedPad> {
    let (data, _) = cache.get_or_decode_preload(path, HOST, preload).unwrap();
    PAD_MAPPINGS
        .iter()
        .map(|m| LoadedPad {
            name: m.name.to_string(),
            choke_group: None,
            output_group: m.output_group,
            close_mics: vec![LoadedMicBank {
                position: "A".to_string(),
                setup_key: String::new(),
                layers: vec![VelocityLayer::new(vec![LoadedSample::from_shared(data.clone())])],
            }],
            extra_banks: Vec::new(),
            overhead: None,
        })
        .collect()
}

/// A stalled reader: the voice plays silence for the frames it does not
/// have, the underrun is counted, nothing panics or waits, and once the
/// reader is back the voice picks up in time — the frames it plays
/// again are the resident render's frames at the same positions.
#[test]
fn a_stalled_reader_costs_silence_for_the_missing_frames_and_recovers() {
    const PRELOAD: u32 = 8_192;
    const FRAMES: usize = 128;
    let fixture = Fixture::new("stall", 48_000, 2.0);
    let cache = SampleCache::new();
    let path = &fixture.overhead[0];
    // A stepped reader: it stalls simply by not being pumped.
    let pool = ReaderPool::stepped();
    let hit = [Hit {
        frame: 0,
        note: drum_map::KICK,
        velocity: 1.0,
    }];
    let params = DrumParams::default();

    // The reference: the whole take, resident.
    let (mut a, _ta) = sampler(
        single_voice_kit(&cache, path, 0),
        &pool,
        RenderMode::Realtime,
    );
    let blocks = (1.5 * HOST) as usize / FRAMES;
    let mut ports = Ports::new(FRAMES);
    let mut reference = Vec::new();
    for b in 0..blocks {
        ports.render(&mut a, FRAMES, &params, if b == 0 { &hit } else { &[] });
        reference.extend(ports.mix_left(FRAMES));
    }

    let (mut s, _ts) = sampler(
        single_voice_kit(&cache, path, PRELOAD),
        &pool,
        RenderMode::Realtime,
    );
    let mut got = Vec::new();
    let head_blocks = PRELOAD as usize / FRAMES;
    // The reader fills the ring, then stalls.
    ports.render(&mut s, FRAMES, &params, &hit);
    got.extend(ports.mix_left(FRAMES));
    pool.pump();
    // Play through the head and the ring and well past it, stalled.
    let ring_blocks = resonance_drums::stream::RING_FRAMES / FRAMES;
    let stalled_until = head_blocks + ring_blocks + 40;
    let mut slowest = Duration::ZERO;
    for _ in 1..stalled_until {
        let t = Instant::now();
        ports.render(&mut s, FRAMES, &params, &[]);
        slowest = slowest.max(t.elapsed());
        got.extend(ports.mix_left(FRAMES));
    }
    assert!(
        slowest < Duration::from_millis(250),
        "a stalled block took {slowest:?}"
    );
    assert!(s.stream_underruns() > 0, "the underrun is counted");
    let missing_from = (head_blocks + ring_blocks) * FRAMES;
    // Everything before the stall bit: the head and the ring's frames.
    assert_eq!(&got[..missing_from], &reference[..missing_from]);
    // The missing frames are silence, not stale ring contents.
    assert!(got[missing_from..stalled_until * FRAMES]
        .iter()
        .all(|&x| x == 0.0));
    assert!(reference[missing_from..stalled_until * FRAMES]
        .iter()
        .any(|&x| x != 0.0));

    // Back: the reader catches up, block by block.
    let mut recovered_at = None;
    for b in stalled_until..blocks {
        pool.pump();
        ports.render(&mut s, FRAMES, &params, &[]);
        let block = ports.mix_left(FRAMES);
        if recovered_at.is_none() && block.iter().all(|&x| x != 0.0) {
            recovered_at = Some(b);
        }
        got.extend(block);
    }
    let at = recovered_at.expect("the voice plays again once the reader is back");
    assert_eq!(at, stalled_until, "the first block after the stall plays");
    // In time: what it plays is the take where the voice would be.
    let at = at * FRAMES;
    assert_eq!(&got[at..], &reference[at..], "recovered out of time");
}

/// The pool's threads run only while a sampler is registered with it:
/// the last one to go joins them, and the next one spawns them again.
#[test]
fn reader_threads_live_only_while_a_sampler_does() {
    let pool = ReaderPool::new(2);
    assert_eq!(pool.running_threads(), 0, "no sampler, no threads");
    let (a, _ta) = sampler(Vec::new(), &pool, RenderMode::Realtime);
    assert_eq!(pool.running_threads(), 2);
    let (b, _tb) = sampler(Vec::new(), &pool, RenderMode::Realtime);
    drop(a);
    assert_eq!(pool.running_threads(), 2, "the second sampler still needs them");
    drop(b);
    assert_eq!(pool.running_threads(), 0, "the last sampler joined them");
    let (c, _tc) = sampler(Vec::new(), &pool, RenderMode::Realtime);
    assert_eq!(pool.running_threads(), 2, "spawned again on demand");
    drop(c);
    assert_eq!(pool.running_threads(), 0);
}

/// A ring is open for the readers from its claim until its voice has
/// ended and the reader has dropped the stream; a scan skips it after.
#[test]
fn rings_are_open_only_while_streamed() {
    let fixture = Fixture::new("open", 48_000, 0.5);
    let cache = SampleCache::new();
    let pool = ReaderPool::stepped();
    let (mut s, _t) = sampler(fixture.kit(&cache, 4_096), &pool, RenderMode::Realtime);
    let params = DrumParams::default();
    let mut ports = Ports::new(128);
    let hit = [Hit {
        frame: 0,
        note: drum_map::KICK,
        velocity: 1.0,
    }];
    ports.render(&mut s, 128, &params, &hit);
    assert_eq!(s.stream_set().rings_open(), 3, "one hit, three streams");
    pool.pump();
    assert_eq!(s.stream_set().rings_open(), 3);
    // Play the takes out, the reader keeping up between blocks.
    for _ in 0..(0.6 * HOST) as usize / 128 {
        ports.render(&mut s, 128, &params, &[]);
        pool.pump();
    }
    assert_eq!(s.voices.iter().filter(|v| v.active).count(), 0);
    assert_eq!(s.stream_underruns(), 0);
    assert_eq!(s.stream_set().rings_open(), 0, "every ring closed again");
    assert_eq!(s.stream_set().rings_in_use(), 0);
}

/// A fresh claim gets one chunk, and the rest of its ring only once its
/// voice is within a ring's length of its tail: a voice choked on its
/// head has cost one read.
#[test]
fn a_ring_fills_as_its_voice_nears_its_tail() {
    use resonance_drums::stream::RING_FRAMES;
    const PRELOAD: u32 = 32_768;
    let fixture = Fixture::new("one-chunk", 48_000, 2.0);
    let cache = SampleCache::new();
    let pool = ReaderPool::stepped();
    let (mut s, _t) = sampler(fixture.kit(&cache, PRELOAD), &pool, RenderMode::Realtime);
    let params = DrumParams::default();
    let mut ports = Ports::new(128);
    let hit = [Hit {
        frame: 0,
        note: drum_map::KICK,
        velocity: 1.0,
    }];
    ports.render(&mut s, 128, &params, &hit);
    pool.pump();
    assert_eq!(s.stream_set().frames_buffered(), 3 * 4_096, "one chunk each");
    // Still more than a ring's length from the tail: nothing more.
    let near = (PRELOAD as usize - RING_FRAMES) / 128;
    for _ in 1..near - 1 {
        ports.render(&mut s, 128, &params, &[]);
        pool.pump();
    }
    assert_eq!(s.stream_set().frames_buffered(), 3 * 4_096);
    for _ in 0..4 {
        ports.render(&mut s, 128, &params, &[]);
        pool.pump();
    }
    assert_eq!(
        s.stream_set().frames_buffered(),
        3 * RING_FRAMES as u64,
        "within a ring of the tail: a full ring each"
    );
}

/// The reader serves the voice that runs out first: one near its tail
/// before a fresh claim, whatever their ring order.
#[test]
fn the_reader_serves_the_earliest_deadline_first() {
    let fixture = Fixture::new("deadline", 48_000, 2.0);
    let cache = SampleCache::new();
    let pool = ReaderPool::stepped();
    let (mut s, _t) = sampler(fixture.kit(&cache, 32_768), &pool, RenderMode::Realtime);
    let params = DrumParams::default();
    let mut ports = Ports::new(128);
    let hit = |note| {
        [Hit {
            frame: 0,
            note,
            velocity: 1.0,
        }]
    };
    // An open hat on the lowest rings, a kick on the next ones.
    ports.render(&mut s, 128, &params, &hit(drum_map::HIHAT_OPEN));
    ports.render(&mut s, 128, &params, &hit(drum_map::KICK));
    pool.pump();
    let rings_of = |s: &DrumSampler, note: u8| -> Vec<u8> {
        s.voices
            .iter()
            .filter(|v| v.active && v.note == note)
            .map(|v| v.ring)
            .collect()
    };
    let kick_rings = rings_of(&s, drum_map::KICK);
    assert_eq!(kick_rings.len(), 3);
    // The hat is choked and its rings handed back; the kick plays on,
    // to within a ring of its tail, with the reader stalled.
    s.choke_note(drum_map::HIHAT_OPEN);
    for _ in 0..20 {
        ports.render(&mut s, 128, &params, &[]);
    }
    pool.pump();
    for _ in 20..200 {
        ports.render(&mut s, 128, &params, &[]);
    }
    // A snare takes the lowest rings (the hat's): a fresh claim, nothing
    // buffered — but its whole head ahead of it.
    ports.render(&mut s, 128, &params, &hit(drum_map::SNARE));
    let snare_rings = rings_of(&s, drum_map::SNARE);
    assert!(snare_rings.iter().max() < kick_rings.iter().min());
    let published = |s: &DrumSampler, rings: &[u8]| -> u64 {
        rings
            .iter()
            .map(|&r| s.stream_set().published_frames(r))
            .sum()
    };
    let kick_before = published(&s, &kick_rings);
    assert!(pool.step(), "one read");
    assert!(
        published(&s, &kick_rings) > kick_before,
        "the kick, near its tail, is read first"
    );
    assert_eq!(published(&s, &snare_rings), 0);
}

/// A panic in a read fails that one stream; the reader carries on with
/// the others.
#[test]
fn a_reader_panic_fails_one_stream_and_the_reader_carries_on() {
    let fixture = Fixture::new("panic", 48_000, 1.0);
    let cache = SampleCache::new();
    let path = &fixture.overhead[0];
    let pool = ReaderPool::stepped();
    let (mut s, _t) = sampler(
        single_voice_kit(&cache, path, 8_192),
        &pool,
        RenderMode::Realtime,
    );
    let params = DrumParams::default();
    let mut ports = Ports::new(128);
    let hit = |note| {
        [Hit {
            frame: 0,
            note,
            velocity: 1.0,
        }]
    };
    s.stream_set().panic_next_reads(1);
    ports.render(&mut s, 128, &params, &hit(drum_map::KICK));
    pool.pump();
    assert_eq!(s.stream_set().rings_failed(), 1, "the panicking read failed its stream");
    assert_eq!(s.stream_set().frames_buffered(), 0);
    ports.render(&mut s, 128, &params, &hit(drum_map::SNARE));
    pool.pump();
    assert_eq!(s.stream_set().rings_failed(), 1);
    assert!(
        s.stream_set().frames_buffered() > 0,
        "the next stream is read"
    );
}

/// One hit on `KICK` at block 0, then `blocks - 1` more blocks of 128
/// frames, `between` run after each: the left mix. A silent block goes
/// first, unrecorded, so a fresh sampler's master volume ramp (from
/// unity to the param's value) is behind it.
fn one_hit(
    s: &mut DrumSampler,
    blocks: usize,
    mut between: impl FnMut(usize, &mut DrumSampler),
) -> Vec<f32> {
    let params = DrumParams::default();
    let mut ports = Ports::new(128);
    let hit = [Hit {
        frame: 0,
        note: drum_map::KICK,
        velocity: 1.0,
    }];
    let mut out = Vec::new();
    ports.render(s, 128, &params, &[]);
    for b in 0..blocks {
        ports.render(s, 128, &params, if b == 0 { &hit } else { &[] });
        out.extend(ports.mix_left(128));
        between(b, s);
    }
    out
}

/// `got` is `reference` up to the fade before `end`, no louder than it
/// through the fade, nearly silent at its end, and silent from `end` on.
fn assert_fades_out_at(tag: &str, got: &[f32], reference: &[f32], end: usize) {
    // RELEASE_FADE_MS (25 ms) at 48 kHz.
    let fade = 1_200;
    assert_eq!(&got[..end - fade], &reference[..end - fade], "{tag}: before the fade");
    let peak = reference[end - fade..end]
        .iter()
        .fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak > 0.05, "{tag}: the reference is silent there");
    for i in end - fade..end {
        assert!(got[i].abs() <= reference[i].abs() + 1e-6, "{tag}: louder at {i}");
    }
    let tail = got[end - 24..end]
        .iter()
        .fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(tail < 0.05 * peak, "{tag}: not faded out at its end ({tail} of {peak})");
    assert!(
        got[end..].iter().all(|&x| x == 0.0),
        "{tag}: sound past the end"
    );
    assert!(
        reference[end..].iter().any(|&x| x != 0.0),
        "{tag}: the reference ends there too"
    );
}

/// A hit that finds every ring taken plays its head and fades out before
/// the head ends — not a cut — and counts once.
#[test]
fn a_hit_with_no_ring_fades_out_before_its_head_ends() {
    const PRELOAD: usize = 8_192;
    let fixture = Fixture::new("no-ring", 48_000, 1.0);
    let cache = SampleCache::new();
    let path = &fixture.overhead[0];
    let pool = ReaderPool::stepped();
    let (mut a, _ta) = sampler(single_voice_kit(&cache, path, 0), &pool, RenderMode::Realtime);
    let reference = one_hit(&mut a, 100, |_, _| {});
    let (mut s, _t) = sampler(
        single_voice_kit(&cache, path, PRELOAD as u32),
        &pool,
        RenderMode::Realtime,
    );
    // Every ring holds a request the (never pumped) reader has not taken.
    for _ in 0..NUM_RINGS {
        one_hit(&mut s, 1, |_, _| {});
    }
    s.reset();
    assert_eq!(s.stream_ring_misses(), 0);
    // (Voices of those hits that reached their tails underran.)
    let before = s.stream_underruns();
    let got = one_hit(&mut s, 100, |_, _| {});
    assert_eq!(s.stream_ring_misses(), 1);
    assert_eq!(s.stream_underruns() - before, 1, "counted once");
    assert_fades_out_at("no ring", &got, &reference, PRELOAD);
    assert!(s.voices.iter().all(|v| !v.active));
}

/// A stream that fails at once (its file is gone) fades its voice out
/// before the head ends, and counts once — not once a block.
#[test]
fn a_failed_stream_fades_out_and_counts_once() {
    const PRELOAD: usize = 8_192;
    let fixture = Fixture::new("failed", 48_000, 1.0);
    let cache = SampleCache::new();
    let path = &fixture.overhead[0];
    let pool = ReaderPool::stepped();
    let (mut a, _ta) = sampler(single_voice_kit(&cache, path, 0), &pool, RenderMode::Realtime);
    let reference = one_hit(&mut a, 120, |_, _| {});
    let pads = single_voice_kit(&cache, path, PRELOAD as u32);
    std::fs::remove_file(path).unwrap();
    let (mut s, _t) = sampler(pads, &pool, RenderMode::Realtime);
    let got = one_hit(&mut s, 120, |_, _| {
        pool.pump();
    });
    assert_eq!(s.stream_underruns(), 1, "counted once");
    assert_fades_out_at("failed at once", &got, &reference, PRELOAD);
}

/// A stream that fails midway (here, a read panics) fades its voice out
/// where its delivered frames end.
#[test]
fn a_stream_failing_midway_fades_out_where_its_frames_end() {
    const PRELOAD: usize = 8_192;
    let fixture = Fixture::new("failed-midway", 48_000, 1.0);
    let cache = SampleCache::new();
    let path = &fixture.overhead[0];
    let pool = ReaderPool::stepped();
    let (mut a, _ta) = sampler(single_voice_kit(&cache, path, 0), &pool, RenderMode::Realtime);
    let reference = one_hit(&mut a, 300, |_, _| {});
    let (mut s, _t) = sampler(
        single_voice_kit(&cache, path, PRELOAD as u32),
        &pool,
        RenderMode::Realtime,
    );
    let mut end = None;
    let got = one_hit(&mut s, 300, |b, s| {
        if b == 70 {
            s.stream_set().panic_next_reads(1);
        }
        pool.pump();
        if end.is_none() && s.stream_set().rings_failed() == 1 {
            let ring = s.voices.iter().find(|v| v.active).unwrap().ring;
            end = Some(PRELOAD + s.stream_set().published_frames(ring) as usize);
        }
    });
    let end = end.expect("the read panicked");
    // A whole ring was read before the panicking read; the take is 48 k
    // frames.
    assert!(
        (PRELOAD + 16_384..40_000).contains(&end),
        "it failed midway: {end}"
    );
    assert_eq!(s.stream_underruns(), 1, "counted once");
    assert_fades_out_at("failed midway", &got, &reference, end);
}

/// A file rewritten after the kit loaded — same length, other samples, a
/// later modification time — is another file: its tail is not read, and
/// the stream fails instead.
#[test]
fn a_file_rewritten_at_the_same_length_is_not_streamed() {
    let fixture = Fixture::new("rewritten", 48_000, 1.0);
    let cache = SampleCache::new();
    let path = fixture.overhead[0].clone();
    let pads = single_voice_kit(&cache, &path, 8_192);
    let len = std::fs::metadata(&path).unwrap().len();
    write_wav(&path, 2, 48_000, 48_000, 99);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), len, "same length");
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + Duration::from_secs(10))
        .unwrap();

    let pool = ReaderPool::stepped();
    let (mut s, _t) = sampler(pads, &pool, RenderMode::Realtime);
    let mut ports = Ports::new(128);
    let hit = [Hit {
        frame: 0,
        note: drum_map::KICK,
        velocity: 1.0,
    }];
    ports.render(&mut s, 128, &DrumParams::default(), &hit);
    pool.pump();
    assert_eq!(s.stream_set().rings_failed(), 1, "the stream failed");
    assert_eq!(s.stream_set().frames_buffered(), 0, "nothing was read");
}

/// The reader pool is shut down mid-render with voices on their tails:
/// every block still returns at once, through new hits (whose rings are
/// never handed back, until none is free), steals, chokes, a kit swap
/// and a reset — and the sampler drops without hanging.
#[test]
fn killing_the_reader_never_blocks_the_audio_thread() {
    const FRAMES: usize = 128;
    let fixture = Fixture::new("kill", 48_000, 1.0);
    let cache = SampleCache::new();
    let pads = fixture.kit(&cache, 4_096);
    let other = fixture.kit(&cache, 4_096);
    let pool = ReaderPool::new(2);
    let (s, tx) = sampler(pads, &pool, RenderMode::Realtime);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    // The audio thread: renders, and reports how long its slowest
    // block took. The watchdog is the timeout on `recv` below.
    let mut other = Some(other);
    let audio = std::thread::spawn(move || {
        let mut s = s;
        let params = DrumParams::default();
        let mut ports = Ports::new(FRAMES);
        let mut slowest = Duration::ZERO;
        for b in 0..1_200 {
            if b == 300 {
                // The reader is dead from block 200 (below); a kit swap
                // now retires every voice onto the old kit's rings.
                tx.send(other.take().unwrap()).unwrap();
            }
            if b == 600 {
                s.reset();
            }
            s.try_swap_kit();
            let t = Instant::now();
            // Six hits a block: steals, the hat choke group, and more
            // ring claims than there are rings.
            ports.render(&mut s, FRAMES, &params, &pattern(b, FRAMES, 6, 1_200));
            slowest = slowest.max(t.elapsed());
            if b == 150 {
                done_tx.send(None).unwrap();
            }
            std::thread::sleep(Duration::from_micros(200));
        }
        let underruns = s.stream_underruns();
        let rings = s.stream_set().rings_in_use();
        drop(s);
        done_tx.send(Some((slowest, underruns, rings))).unwrap();
    });
    // Let it start streaming, then kill the reader under it.
    done_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("audio thread started");
    pool.shutdown();
    let (slowest, underruns, rings) = done_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("the audio thread is blocked")
        .unwrap();
    audio.join().unwrap();
    // Blocking on the dead reader would never return (the watchdog
    // above); a block held off the CPU of a loaded test machine for a
    // while is not that.
    assert!(slowest < Duration::from_secs(2), "a block took {slowest:?}");
    assert!(underruns > 0, "the dead reader shows as underruns");
    // Nothing handed rings back once the reader died, so they ran out.
    assert_eq!(rings, NUM_RINGS);
}

// ---------------------------------------------------------------------------
// Memory.
// ---------------------------------------------------------------------------

/// Only the heads are resident: a streamed take holds `preload` frames,
/// and the cache's resident bytes are exactly the heads'.
#[test]
fn only_heads_are_resident() {
    const PRELOAD: u32 = 32_768;
    let fixture = Fixture::new("memory", 48_000, 3.0);
    let whole = SampleCache::new();
    let streamed = SampleCache::new();
    let _keep_whole = fixture.kit(&whole, 0);
    let keep_streamed = fixture.kit(&streamed, PRELOAD);
    let mut heads = 0u64;
    let mut seen = std::collections::HashSet::new();
    for take in keep_streamed
        .iter()
        .flat_map(|p| p.close_mics.iter().chain(p.overhead.iter()))
        .flat_map(|b| b.layers.iter().flat_map(|l| l.round_robins.iter()))
    {
        assert_eq!(take.resident_frames(), PRELOAD as usize);
        assert!(take.frames() > take.resident_frames());
        if seen.insert(Arc::as_ptr(take.shared())) {
            heads += (PRELOAD as usize * take.channels() * 4) as u64;
        }
    }
    let whole_bytes = whole.stats().resident_bytes;
    let streamed_bytes = streamed.stats().resident_bytes;
    assert_eq!(streamed_bytes, heads, "resident bytes are the heads'");
    // 32 k of ~144 k frames: well under a quarter.
    assert!(
        streamed_bytes * 4 < whole_bytes,
        "{streamed_bytes} vs {whole_bytes}"
    );
}

/// Ring storage is allocated by the reader for the rings it serves, and
/// only those: a sampler that streams nothing holds none, and one hit's
/// three voices hold three rings' worth.
#[test]
fn only_rings_in_use_hold_storage() {
    use resonance_drums::stream::RING_BYTES;
    let fixture = Fixture::new("ring-memory", 48_000, 0.5);
    let cache = SampleCache::new();
    let pool = ReaderPool::stepped();
    let (mut s, _t) = sampler(fixture.kit(&cache, 4_096), &pool, RenderMode::Realtime);
    let params = DrumParams::default();
    let mut ports = Ports::new(128);
    ports.render(&mut s, 128, &params, &[]);
    pool.pump();
    assert_eq!(s.stream_set().ring_bytes(), 0, "no stream, no storage");
    let hit = [Hit {
        frame: 0,
        note: drum_map::KICK,
        velocity: 1.0,
    }];
    ports.render(&mut s, 128, &params, &hit);
    assert_eq!(s.stream_rings_claimed(), 3);
    assert_eq!(s.stream_set().ring_bytes(), 0, "claimed, not served yet");
    pool.pump();
    assert_eq!(s.stream_set().rings_allocated(), 3);
    assert_eq!(s.stream_set().ring_bytes(), 3 * RING_BYTES as u64);
}

// ---------------------------------------------------------------------------
// Through the plugin: loader, state, `process`.
// ---------------------------------------------------------------------------

/// A manifest kit over `fixture`'s files: kick (two close mics and an
/// overhead) and snare (close and overhead).
fn manifest_kit(fixture: &Fixture) -> PathBuf {
    let name = |p: &PathBuf| p.file_name().unwrap().to_string_lossy().into_owned();
    let setup = |pos: &str, file: String| {
        format!(
            r#"{{"brand":"t","channel":"1","mic":"m","position":"{pos}","rounds":{{"RR1":{{"Vel01":"{file}"}}}}}}"#
        )
    };
    let manifest = format!(
        r#"{{
  "SD Kick mit Teppich": {{
    "01_KickIn_e901": {kin},
    "03_KickOut": {kout},
    "23_OHsAB_e914": {koh}
  }},
  "SD Snare Normal": {{
    "04_SNTop": {sn},
    "23_OHsAB_e914": {snoh}
  }}
}}"#,
        kin = setup("KickIn", name(&fixture.close[0])),
        kout = setup("KickOut", name(&fixture.close[1])),
        koh = setup("OHsAB", name(&fixture.overhead[0])),
        sn = setup("SNTop", name(&fixture.close[2])),
        snoh = setup("OHsAB", name(&fixture.overhead[1])),
    );
    let path = fixture.dir.join("drum_samples.json");
    std::fs::write(&path, manifest).unwrap();
    path
}

fn saver_for(plugin: &resonance_drums::ResonanceDrums) -> resonance_drums::DrumsExtraState {
    resonance_drums::DrumsExtraState {
        kit_path: plugin.bridge.kit_path.clone(),
        overhead_setup_key: plugin.bridge.overhead_setup_key.clone(),
        mic_banks: plugin.bridge.mic_banks.clone(),
        pad_choices: plugin.bridge.pad_choices.clone(),
        params: plugin.bridge.params.clone(),
        reload: Some(plugin.bridge.clone()),
    }
}

/// Wait for the plugin's load to land on the audio side.
fn settle(plugin: &mut resonance_drums::ResonanceDrums) {
    use resonance_drums::kit_loader::KitStatus;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let pending = plugin.bridge.pending_kit.lock().is_some();
        match plugin.bridge.kit_status.lock().clone() {
            KitStatus::Loaded { .. } if !pending => break,
            KitStatus::Error { message } if !pending => panic!("kit failed: {message}"),
            _ => {}
        }
        assert!(Instant::now() < deadline, "load never settled");
        std::thread::sleep(Duration::from_millis(2));
    }
    // One block takes the kit from the mailbox.
    plugin_block(plugin, &[]);
}

fn plugin_block(
    plugin: &mut resonance_drums::ResonanceDrums,
    events: &[resonance_plugin::NoteEvent],
) -> Vec<u32> {
    use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};
    const FRAMES: usize = 256;
    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0; FRAMES], vec![0.0; FRAMES]))
        .collect();
    {
        let mut ports: Vec<OutputBuffer<'_>> = bufs
            .iter_mut()
            .map(|(l, r)| OutputBuffer {
                left: l.as_mut_slice(),
                right: r.as_mut_slice(),
            })
            .collect();
        let mut iter = EventIterator::new(events);
        plugin.process(&mut ports, FRAMES, &mut iter, None);
    }
    bufs.iter()
        .flat_map(|(l, r)| l.iter().chain(r))
        .map(|s| s.to_bits())
        .collect()
}

/// The preload is plugin state: saved, restored, and what the loader
/// splits takes at. A plugin streaming its kit plays it bit-identically
/// to one holding it whole, with a fraction of the memory.
#[test]
fn the_plugin_streams_its_kit_and_keeps_the_preload_in_its_state() {
    use resonance_drums::stream::{DEFAULT_PRELOAD, PRELOAD_STATE_KEY};
    use resonance_drums::ResonanceDrums;
    use resonance_plugin::plugin::ExtraStateSaver;
    use resonance_plugin::{NoteEvent, ResonancePlugin};

    let fixture = Fixture::new("plugin", 44_100, 2.0);
    let manifest = manifest_kit(&fixture);

    // State: the default is saved; a saved preload is restored; an
    // unknown one is ignored.
    let first = ResonanceDrums::new();
    assert_eq!(
        saver_for(&first).save().get(PRELOAD_STATE_KEY),
        Some(&serde_json::json!(DEFAULT_PRELOAD))
    );
    let mut whole = ResonanceDrums::new();
    saver_for(&whole).load(&serde_json::json!({ PRELOAD_STATE_KEY: 0, "kit_path": manifest }));
    assert_eq!(
        whole
            .bridge
            .stream_preload
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    saver_for(&whole).load(&serde_json::json!({ PRELOAD_STATE_KEY: 12_345 }));
    assert_eq!(
        whole
            .bridge
            .stream_preload
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert_eq!(
        saver_for(&whole).save().get(PRELOAD_STATE_KEY),
        Some(&serde_json::json!(0))
    );

    let mut streamed = ResonanceDrums::new();
    saver_for(&streamed).load(&serde_json::json!({ "kit_path": manifest }));
    // The render below runs as fast as it can: the host says so, as a
    // bounce would (CLAP `render`), so the comparison does not hang on
    // how fast the reader threads get the CPU.
    streamed.bridge.host_render_mode.store(
        resonance_drums::stream::HOST_RENDER_OFFLINE,
        std::sync::atomic::Ordering::Relaxed,
    );
    assert!(whole.initialize(HOST, 256));
    assert!(streamed.initialize(HOST, 256));
    settle(&mut whole);
    settle(&mut streamed);
    // The kick and snare takes stream; the rest of the pads are the
    // built-in kit's short takes, whole either way.
    let kick = |p: &ResonanceDrums| -> Arc<resonance_drums::kit::SampleData> {
        let built = p.bridge.built_kit.lock().clone().unwrap();
        built.pads[0].close_mics[0].layers[0].round_robins[0]
            .shared()
            .clone()
    };
    assert!(kick(&streamed).tail().is_some());
    assert_eq!(kick(&streamed).resident_frames(), DEFAULT_PRELOAD as usize);
    assert!(kick(&whole).tail().is_none());
    let bytes = |p: &ResonanceDrums| {
        p.bridge
            .kit_bytes
            .load(std::sync::atomic::Ordering::Relaxed)
    };
    assert!(bytes(&streamed) < bytes(&whole));

    let mut a = Vec::new();
    let mut b = Vec::new();
    for blk in 0..(2.2 * HOST) as usize / 256 {
        let events: Vec<NoteEvent> = match blk {
            0 => vec![NoteEvent::NoteOn {
                note: drum_map::KICK,
                velocity: 1.0,
                timing: 3,
            }],
            40 => vec![NoteEvent::NoteOn {
                note: drum_map::SNARE,
                velocity: 0.8,
                timing: 100,
            }],
            _ => Vec::new(),
        };
        a.extend(plugin_block(&mut whole, &events));
        b.extend(plugin_block(&mut streamed, &events));
    }
    assert!(loud(&a) > 0.05);
    assert_eq!(
        streamed
            .bridge
            .stream_underruns
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert!(
        a == b,
        "the streamed plugin's output differs from the whole one's"
    );

    // Changing the preload reloads the kit at the new size.
    assert!(resonance_drums::stream::set_preload(
        &streamed.bridge,
        65_536
    ));
    settle(&mut streamed);
    let built = streamed.bridge.built_kit.lock().clone().unwrap();
    assert_eq!(built.preload, 65_536);
}

/// The preload is a param (`stream_preload`: Off / 32k / 64k / 128k, not
/// automatable): moving it — from a host, the control API or the editor
/// — reloads the kit at the new size, through the instance's watcher
/// (`selection::watch`). It is not in the params state; the state keeps
/// the preload in frames under its own key, which sets the param on load.
#[test]
fn the_stream_preload_param_reloads_the_kit() {
    use resonance_drums::stream::{
        preload_frames, set_preload, DEFAULT_PRELOAD, PRELOAD_STATE_KEY,
    };
    use resonance_drums::ResonanceDrums;
    use resonance_plugin::plugin::ExtraStateSaver;
    use resonance_plugin::{Param, ResonancePlugin};
    use std::sync::atomic::Ordering;

    let fixture = Fixture::new("preload-param", 44_100, 2.0);
    let manifest = manifest_kit(&fixture);
    let mut drums = ResonanceDrums::new();
    {
        let p = &drums.bridge.params.stream_preload;
        assert_eq!(p.id(), "stream_preload");
        assert!(!p.is_automatable());
        assert!(p.state_excluded());
        assert_eq!(preload_frames(p.value()), DEFAULT_PRELOAD);
        assert_eq!(p.display(p.get_plain()), "32k");
        assert_eq!(p.display(0.0), "Off");
        assert_eq!(p.parse("128k"), Some(3.0));
    }
    saver_for(&drums).load(&serde_json::json!({ "kit_path": manifest }));
    assert!(drums.initialize(HOST, 256));
    settle(&mut drums);
    let built_preload = |d: &ResonanceDrums| d.bridge.built_kit.lock().clone().unwrap().preload;
    assert_eq!(built_preload(&drums), DEFAULT_PRELOAD);

    // The host moves the param: the watcher reloads at 64k.
    let generation = drums.bridge.load_generation.load(Ordering::Acquire);
    drums.bridge.params.stream_preload.set_plain(2.0);
    resonance_drums::selection::watch(&drums.bridge);
    settle(&mut drums);
    assert_eq!(built_preload(&drums), 65_536);
    assert_eq!(drums.bridge.stream_preload.load(Ordering::Relaxed), 65_536);
    assert!(drums.bridge.load_generation.load(Ordering::Acquire) > generation);

    // Saved in frames under its key, and not among the params.
    assert_eq!(
        saver_for(&drums).save().get(PRELOAD_STATE_KEY),
        Some(&serde_json::json!(65_536))
    );
    let state: serde_json::Value = serde_json::from_slice(&drums.save_state()).unwrap();
    assert!(state["params"].get("stream_preload").is_none());

    // A state's preload sets the param.
    let other = ResonanceDrums::new();
    saver_for(&other).load(&serde_json::json!({ PRELOAD_STATE_KEY: 131_072 }));
    assert_eq!(other.bridge.params.stream_preload.value(), 3);
    saver_for(&other).load(&serde_json::json!({ PRELOAD_STATE_KEY: 0 }));
    assert_eq!(other.bridge.params.stream_preload.value(), 0);

    // `set_preload` moves the param with it, so the watcher leaves it be.
    assert!(set_preload(&drums.bridge, 0));
    assert_eq!(drums.bridge.params.stream_preload.value(), 0);
    settle(&mut drums);
    let generation = drums.bridge.load_generation.load(Ordering::Acquire);
    resonance_drums::selection::watch(&drums.bridge);
    assert_eq!(
        drums.bridge.load_generation.load(Ordering::Acquire),
        generation,
        "no reload for a param that already matches"
    );
    assert_eq!(built_preload(&drums), 0);
}

/// A host that hands `process` fewer ports than the plugin declared gets
/// silence, not a panic — and the block still runs in full: the hit in
/// it starts at its frame and plays on in time, so the next full block
/// is exactly what it would have been.
#[test]
fn a_block_with_too_few_ports_still_runs_in_time() {
    use resonance_drums::ResonanceDrums;
    use resonance_plugin::{EventIterator, NoteEvent, OutputBuffer, ResonancePlugin};
    let mut a = ResonanceDrums::new();
    let mut b = ResonanceDrums::new();
    assert!(a.initialize(HOST, 256));
    assert!(b.initialize(HOST, 256));
    let hit = [NoteEvent::NoteOn {
        note: drum_map::SNARE,
        velocity: 1.0,
        timing: 40,
    }];
    // `a` gets one port for the first block, `b` all of them.
    let (mut l, mut r) = (vec![0.0f32; 256], vec![0.0f32; 256]);
    {
        let mut one = [OutputBuffer {
            left: l.as_mut_slice(),
            right: r.as_mut_slice(),
        }];
        a.process(&mut one, 256, &mut EventIterator::new(&hit), None);
    }
    assert!(l.iter().chain(&r).all(|&s| s == 0.0));
    plugin_block(&mut b, &hit);
    let mut heard = 0.0f32;
    for _ in 0..8 {
        let got = plugin_block(&mut a, &[]);
        let want = plugin_block(&mut b, &[]);
        heard = heard.max(loud(&want));
        assert!(got == want, "the hit is out of time after the short block");
    }
    assert!(heard > 0.01, "the snare is heard");
}

// ---------------------------------------------------------------------------
// The real library (opt-in).
// ---------------------------------------------------------------------------

/// `RESONANCE_DRUMMICA_PATH`: the kit's `drum_samples.json`, or the
/// folder holding it. Read only.
fn drummica_manifest() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var("RESONANCE_DRUMMICA_PATH").ok()?);
    Some(if path.is_dir() {
        path.join("drum_samples.json")
    } else {
        path
    })
}

fn drummica_request(manifest: &Path, preload: u32) -> resonance_drums::kit_loader::KitRequest {
    resonance_drums::kit_loader::KitRequest {
        path: manifest.to_path_buf(),
        overhead_setup_key: resonance_drums::kit_loader::DEFAULT_OVERHEAD_SETUP.to_string(),
        pad_choices: std::array::from_fn(|_| Default::default()),
        articulations: [false; NUM_PADS],
        preload,
        banks: Default::default(),
    }
}

/// The default Drummica setup: resident memory at every preload (and
/// whole, for comparison), under 1 GiB at the default; and a streamed
/// render of long-ringing pads bit-identical to the whole kit's.
/// Skipped without `RESONANCE_DRUMMICA_PATH`; run it in release with
/// `--nocapture` to see the figures.
#[test]
fn drummica_default_setup_memory_and_bit_identity() {
    let Some(manifest) = drummica_manifest() else {
        eprintln!("RESONANCE_DRUMMICA_PATH not set; skipping the real-library test");
        return;
    };
    let load = |preload: u32| {
        let cache = SampleCache::new();
        let began = Instant::now();
        let kit = resonance_drums::kit_loader::load_kit(
            &drummica_request(&manifest, preload),
            HOST,
            None,
            None,
            &cache,
            &|| {},
            &|_| {},
            &|| false,
        )
        .expect("drummica loads");
        let bytes = cache.stats().resident_bytes;
        let (streamed, takes) = streamed_takes(&kit.pads);
        eprintln!(
            "drummica preload {preload:>6}: {:>7.1} MiB resident, {streamed}/{takes} takes streamed, loaded in {:.1?}",
            bytes as f64 / (1024.0 * 1024.0),
            began.elapsed()
        );
        (kit.pads, bytes)
    };
    let (whole, whole_bytes) = load(0);
    let mut default_kit = None;
    for preload in resonance_drums::stream::PRELOAD_CHOICES {
        let (pads, bytes) = load(preload);
        assert!(bytes < whole_bytes);
        if preload == resonance_drums::stream::DEFAULT_PRELOAD {
            assert!(bytes < 1 << 30, "the default setup holds {bytes} bytes");
            default_kit = Some(pads);
        }
    }
    let streamed = default_kit.unwrap();

    // Long tails: crashes, ride, china, toms, kick, an open hat — every
    // hit on its own, offline (the render runs far faster than real
    // time), loud and then soft.
    let pool = ReaderPool::new(2);
    let notes: Vec<u8> = [0usize, 1, 9, 11, 12, 15, 18, 21, 3]
        .iter()
        .map(|&p| PAD_MAPPINGS[p].note)
        .collect();
    let hits_at = |block: usize| -> Vec<Hit> {
        if block.is_multiple_of(40) && block / 40 < notes.len() * 2 {
            let n = block / 40;
            vec![Hit {
                frame: 17,
                note: notes[n % notes.len()],
                velocity: if n < notes.len() { 1.0 } else { 0.55 },
            }]
        } else {
            Vec::new()
        }
    };
    let render_kit = |pads: Vec<LoadedPad>, mode: RenderMode| -> (Vec<u32>, u64) {
        let (mut s, _t) = sampler(pads, &pool, mode);
        let params = DrumParams::default();
        let mut ports = Ports::new(512);
        let mut out = Vec::new();
        for b in 0..(12.0 * HOST) as usize / 512 {
            ports.render(&mut s, 512, &params, &hits_at(b));
            ports.append_bits(512, &mut out);
        }
        // Ring storage is memory too: what the streamed render's rings
        // hold, on top of the heads.
        eprintln!(
            "drummica ring storage: {} rings, {:.1} MiB",
            s.stream_set().rings_allocated(),
            s.stream_set().ring_bytes() as f64 / (1024.0 * 1024.0)
        );
        (out, s.stream_underruns())
    };
    let (reference, _) = render_kit(whole, RenderMode::Realtime);
    let (got, underruns) = render_kit(streamed, RenderMode::Offline);
    assert!(loud(&reference) > 0.05);
    assert_eq!(underruns, 0);
    if let Some((block, port, frame)) = first_difference(&reference, &got, 512) {
        panic!("drummica: streamed render differs at block {block}, port {port}, frame {frame}");
    }
    pool.shutdown();
}
