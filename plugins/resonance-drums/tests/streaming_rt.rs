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
        layers: vec![VelocityLayer {
            round_robins: vec![take(name)],
        }],
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
