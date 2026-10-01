//! Disk streaming (drums-plugin-rework.md E14) never allocates or frees
//! on the audio thread: claiming a ring, reading it, underrunning,
//! stealing a streaming voice into a tail slot, choking it, swapping the
//! kit under it, resetting, and waiting for the reader offline.
//!
//! A standalone binary because it installs a counting
//! `#[global_allocator]` (process-global). The count is per thread, so
//! the reader threads' own allocations do not show up.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::io::Write;
use std::path::Path;

use crossbeam_channel::bounded;
use resonance_drums::drum_map::{self, NUM_PADS, PAD_MAPPINGS};
use resonance_drums::dsp::{DrumSampler, Hit, PortBuffers};
use resonance_drums::kit::{
    LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer, NUM_OUTPUT_PORTS,
};
use resonance_drums::kit_loader::cache::SampleCache;
use resonance_drums::params::DrumParams;
use resonance_drums::stream::RenderMode;

struct Counting;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static EVENTS: Cell<usize> = const { Cell::new(0) };
}

fn note_event() {
    let _ = ARMED.try_with(|armed| {
        if armed.get() {
            let _ = EVENTS.try_with(|e| e.set(e.get() + 1));
        }
    });
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_event();
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note_event();
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note_event();
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Run `f` and return how many heap events it caused on this thread.
fn heap_events(f: impl FnOnce()) -> usize {
    EVENTS.with(|e| e.set(0));
    ARMED.with(|a| a.set(true));
    f();
    ARMED.with(|a| a.set(false));
    EVENTS.with(|e| e.get())
}

const BLOCK: usize = 128;
const HOST: f32 = 48_000.0;
const PRELOAD: u32 = 4_096;

fn write_wav(path: &Path, channels: u16, frames: usize, seed: usize) {
    let mut data = Vec::with_capacity(frames * channels as usize * 2);
    for f in 0..frames {
        for ch in 0..channels as usize {
            let v = ((f as f64 * (0.01 + seed as f64 * 0.003) + ch as f64).sin() * 12_000.0) as i16;
            data.extend_from_slice(&v.to_le_bytes());
        }
    }
    let block_align = channels as u32 * 2;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&44_100u32.to_le_bytes());
    out.extend_from_slice(&(44_100 * block_align).to_le_bytes());
    out.extend_from_slice(&(block_align as u16).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    std::fs::File::create(path)
        .unwrap()
        .write_all(&out)
        .unwrap();
}

/// A kit of streamed takes (44.1 kHz files, so the tails resample):
/// two close banks (mono, stereo) and a stereo overhead per pad, choke
/// groups as mapped (the hats).
fn kit(cache: &SampleCache, dir: &Path, tag: &str) -> Vec<LoadedPad> {
    let take = |name: &str| {
        let (data, _) = cache
            .get_or_decode_preload(&dir.join(name), HOST, PRELOAD)
            .unwrap();
        assert!(data.tail().is_some(), "{name} streams");
        LoadedSample::from_shared(data)
    };
    let bank = |name: &str| LoadedMicBank {
        position: name.to_string(),
        setup_key: String::new(),
        layers: vec![VelocityLayer::new(vec![take(name)])],
    };
    PAD_MAPPINGS
        .iter()
        .map(|m| LoadedPad {
            name: format!("{tag}:{}", m.name),
            choke_group: m.choke_group,
            output_group: m.output_group,
            close_mics: vec![bank("a.wav"), bank("b.wav")],
            overhead: Some(bank("oh.wav")),
        })
        .collect()
}

struct Bufs(Vec<(Vec<f32>, Vec<f32>)>);

/// Render one block. The port views live in a fixed array on the stack.
fn render(sampler: &mut DrumSampler, bufs: &mut Bufs, params: &DrumParams, hits: &[Hit]) {
    let mut it = bufs.0.iter_mut();
    let mut ports: [PortBuffers<'_>; NUM_OUTPUT_PORTS] = std::array::from_fn(|_| {
        let (l, r) = it.next().unwrap();
        PortBuffers {
            left: l.as_mut_slice(),
            right: r.as_mut_slice(),
        }
    });
    sampler.render_block(&mut ports, BLOCK, params, hits);
}

#[test]
fn streaming_never_touches_the_heap_on_the_audio_thread() {
    let dir = std::env::temp_dir().join(format!("drums-streaming-rt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    write_wav(&dir.join("a.wav"), 1, 44_100, 1);
    write_wav(&dir.join("b.wav"), 2, 40_000, 2);
    write_wav(&dir.join("oh.wav"), 2, 44_100, 3);
    let cache = SampleCache::new();

    let (kit_tx, kit_rx) = bounded(1);
    let mut sampler = DrumSampler::new(kit_rx);
    sampler.set_sample_rate(HOST);
    sampler.set_render_mode(RenderMode::Realtime);
    sampler.pads = kit(&cache, &dir, "boot");
    let params = DrumParams::default();
    let mut bufs = Bufs(
        (0..NUM_OUTPUT_PORTS)
            .map(|_| (vec![0.0; BLOCK], vec![0.0; BLOCK]))
            .collect(),
    );
    // Warm up: one block, so lazily-initialised statics (the clock) are
    // not counted.
    render(&mut sampler, &mut bufs, &params, &[]);

    let mut events = 0;
    let mut peak = 0.0f32;
    for b in 0..1_500usize {
        // The loader's side, off the measured region.
        if b % 300 == 150 {
            let _ = kit_tx.try_send(kit(&cache, &dir, &format!("k{b}")));
        }
        if b == 700 {
            // A stalled reader: underruns, still no heap.
            sampler.stream_set().set_paused(true);
        }
        if b == 760 {
            sampler.stream_set().set_paused(false);
        }
        let hits: Vec<Hit> = (0..4)
            .map(|k| {
                let n = b * 4 + k;
                Hit {
                    frame: k * 31,
                    note: PAD_MAPPINGS[(n * 7) % NUM_PADS].note,
                    velocity: 0.8,
                }
            })
            .collect();
        events += heap_events(|| {
            sampler.try_swap_kit();
            if b % 50 == 0 {
                sampler.choke_note(drum_map::HIHAT_OPEN);
            }
            if b == 1_000 {
                sampler.reset();
            }
            render(&mut sampler, &mut bufs, &params, &hits);
        });
        peak = bufs
            .0
            .iter()
            .flat_map(|(l, r)| l.iter().chain(r))
            .fold(peak, |m, s| m.max(s.abs()));
        std::thread::sleep(std::time::Duration::from_micros(300));
    }
    assert!(peak > 0.0, "silent: the test proves nothing");
    assert!(sampler.stream_underruns() > 0, "the stall was exercised");
    assert!(sampler.tail_voices_active() > 0 || sampler.stream_rings_claimed() > 0);

    // Offline, waiting for a stalled reader (sleeping, bounded) — for
    // frames, and for a ring whose request it has not taken — allocates
    // nothing either.
    sampler.set_render_mode(RenderMode::Offline);
    sampler.stream_set().set_paused(true);
    for mapping in PAD_MAPPINGS.iter().take(3) {
        let hits = [Hit {
            frame: 0,
            note: mapping.note,
            velocity: 1.0,
        }];
        events += heap_events(|| render(&mut sampler, &mut bufs, &params, &hits));
    }
    sampler.stream_set().set_paused(false);
    assert_eq!(
        events, 0,
        "the audio thread allocated or freed {events} times while streaming"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A kit whose banks each have two velocity layers 30 dB apart: a quiet
/// resident one and a loud streamed one — so the E7 level pick has
/// levels to tell apart, and tuned voices read heads and rings both.
fn layered_kit(cache: &SampleCache, dir: &Path) -> Vec<LoadedPad> {
    let streamed = |name: &str| {
        let (data, _) = cache
            .get_or_decode_preload(&dir.join(name), HOST, PRELOAD)
            .unwrap();
        LoadedSample::from_shared(data)
    };
    let quiet = LoadedSample::mono(
        (0..PRELOAD as usize * 2)
            .map(|i| (i as f32 * 0.03).sin() * 0.012)
            .collect(),
    );
    let bank = |name: &str| LoadedMicBank {
        position: name.to_string(),
        setup_key: String::new(),
        layers: vec![
            VelocityLayer::new(vec![quiet.clone()]),
            VelocityLayer::new(vec![streamed(name)]),
        ],
    };
    PAD_MAPPINGS
        .iter()
        .map(|m| LoadedPad {
            name: m.name.to_string(),
            choke_group: m.choke_group,
            output_group: m.output_group,
            close_mics: vec![bank("a.wav"), bank("b.wav")],
            overhead: Some(bank("oh.wav")),
        })
        .collect()
}

/// The K7 playing features (drums-plugin-rework.md §7 E7, E8, E9, E11,
/// E12) add nothing to the heap on the audio thread either: the per-block
/// settings snapshot, the velocity level pick and humanize, tuned voices
/// (Hermite reads over heads and rings, rate-scaled deadlines), hold /
/// decay envelopes, sample starts, dB levels and trims, Stereo and Multi
/// routing, param choke groups — live, through a stalled reader, and
/// offline.
#[test]
fn the_playing_features_never_touch_the_heap_on_the_audio_thread() {
    use resonance_drums::params::{OUTPUT_MODE_MULTI, OUTPUT_MODE_STEREO};

    let dir = std::env::temp_dir().join(format!("drums-k7-rt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    write_wav(&dir.join("a.wav"), 1, 44_100, 1);
    write_wav(&dir.join("b.wav"), 2, 40_000, 2);
    write_wav(&dir.join("oh.wav"), 2, 44_100, 3);
    let cache = SampleCache::new();

    let (_kit_tx, kit_rx) = bounded::<Vec<LoadedPad>>(1);
    let mut sampler = DrumSampler::new(kit_rx);
    sampler.set_sample_rate(HOST);
    sampler.set_render_mode(RenderMode::Realtime);
    sampler.pads = layered_kit(&cache, &dir);
    let params = DrumParams::default();
    params.velocity_humanize.set_value(10.0);
    params.velocity_curve.set_value(0.3);
    params.master_volume.set_value(-3.0);
    for (i, pad) in params.pads.iter().enumerate() {
        pad.tune.set_value([12.0, -7.0, 0.0, 24.0, 3.5][i % 5]);
        pad.volume.set_value(-(i as f32) * 0.5);
        pad.trims[1].set_value(-4.0);
        pad.trims[2].set_value(2.0);
        if i % 3 == 0 {
            pad.hold.set_value(5.0);
            pad.decay.set_value(60.0);
        }
        if i % 4 == 1 {
            pad.start.set_value(8.0);
        }
        if (9..=11).contains(&i) {
            pad.choke.set_value(2);
        }
        pad.output.set_value((i % NUM_OUTPUT_PORTS) as i32);
    }
    let mut bufs = Bufs(
        (0..NUM_OUTPUT_PORTS)
            .map(|_| (vec![0.0; BLOCK], vec![0.0; BLOCK]))
            .collect(),
    );
    sampler.update_global_settings(&params);
    render(&mut sampler, &mut bufs, &params, &[]);

    let mut events = 0;
    let mut peak = 0.0f32;
    for b in 0..1_200usize {
        // The host's side, off the measured region: params move.
        if b % 200 == 0 {
            params.output_mode.set_value(if b % 400 == 0 {
                OUTPUT_MODE_MULTI
            } else {
                OUTPUT_MODE_STEREO
            });
        }
        params.pads[0].tune.set_value(((b % 49) as f32 - 24.0) * 0.5);
        if b == 500 {
            sampler.stream_set().set_paused(true);
        }
        if b == 560 {
            sampler.stream_set().set_paused(false);
        }
        let hits: Vec<Hit> = (0..4)
            .map(|k| {
                let n = b * 4 + k;
                Hit {
                    frame: k * 31,
                    note: PAD_MAPPINGS[(n * 7) % NUM_PADS].note,
                    velocity: 0.1 + 0.9 * ((n * 13) % 10) as f32 / 10.0,
                }
            })
            .collect();
        events += heap_events(|| {
            sampler.update_global_settings(&params);
            render(&mut sampler, &mut bufs, &params, &hits);
        });
        peak = bufs
            .0
            .iter()
            .flat_map(|(l, r)| l.iter().chain(r))
            .fold(peak, |m, s| m.max(s.abs()));
        std::thread::sleep(std::time::Duration::from_micros(300));
    }
    assert!(peak > 0.0, "silent: the test proves nothing");
    assert!(sampler.stream_underruns() > 0, "the stall was exercised");

    // Offline, waiting for a stalled reader with tuned voices.
    sampler.set_render_mode(RenderMode::Offline);
    sampler.stream_set().set_paused(true);
    for mapping in PAD_MAPPINGS.iter().take(3) {
        let hits = [Hit {
            frame: 0,
            note: mapping.note,
            velocity: 1.0,
        }];
        events += heap_events(|| {
            sampler.update_global_settings(&params);
            render(&mut sampler, &mut bufs, &params, &hits);
        });
    }
    sampler.stream_set().set_paused(false);
    assert_eq!(
        events, 0,
        "the audio thread allocated or freed {events} times while playing"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
