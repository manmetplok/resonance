//! Retiring a kit never allocates, frees or drops on the audio thread.
//!
//! The janitor's channel used to be crossbeam's `unbounded()`, whose list
//! flavour allocates a fresh block on the *sending* thread every 31
//! sends — and the sender is the audio thread, in `try_swap_kit` and
//! `end_block`. It is a preallocated `bounded` ring now, and a full ring
//! leaves the outgoing kit parked in its retired slot instead of dropping
//! it where it is.
//!
//! A standalone binary because it installs a counting
//! `#[global_allocator]` (process-global). The count is per thread, so
//! the janitor's own frees on its own thread do not show up.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use crossbeam_channel::bounded;
use resonance_drums::drum_map::{self, PAD_MAPPINGS};
use resonance_drums::dsp::{DrumSampler, PortBuffers};
use resonance_drums::kit::{
    LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer, NUM_OUTPUT_PORTS,
};
use resonance_drums::kit_loader::KitLoadProgress;
use resonance_drums::params::DrumParams;

struct Counting;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static EVENTS: Cell<usize> = const { Cell::new(0) };
}

fn note_event() {
    // `try_with`: the allocator also runs while thread-locals are being
    // torn down.
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

const BLOCK: usize = 64;

/// A kit whose every pad holds one short DC take; `tag` goes into the
/// pad names so a test can tell kits apart.
fn kit(tag: &str) -> Vec<LoadedPad> {
    PAD_MAPPINGS
        .iter()
        .map(|m| LoadedPad {
            name: format!("{tag}:{}", m.name),
            choke_group: None,
            output_group: m.output_group,
            close_mics: vec![LoadedMicBank {
                position: "test".to_string(),
                setup_key: String::new(),
                layers: vec![VelocityLayer::new(vec![LoadedSample::from_data(vec![0.1; 2 * 48_000])])],
            }],
            overhead: None,
        })
        .collect()
}

/// [`kit`] with mono takes, all sharing one `Arc`'d sample — the shape
/// the shared sample cache produces (E5).
fn mono_kit(tag: &str) -> Vec<LoadedPad> {
    let take = LoadedSample::mono(vec![0.1; 48_000]);
    PAD_MAPPINGS
        .iter()
        .map(|m| LoadedPad {
            name: format!("{tag}:{}", m.name),
            choke_group: None,
            output_group: m.output_group,
            close_mics: vec![LoadedMicBank {
                position: "test".to_string(),
                setup_key: String::new(),
                layers: vec![VelocityLayer::new(vec![take.clone()])],
            }],
            overhead: Some(LoadedMicBank {
                position: "OH".to_string(),
                setup_key: String::new(),
                layers: vec![VelocityLayer::new(vec![LoadedSample::from_data(vec![0.05; 2 * 48_000])])],
            }),
        })
        .collect()
}

struct Bufs(Vec<(Vec<f32>, Vec<f32>)>);

impl Bufs {
    fn new() -> Self {
        Self(
            (0..NUM_OUTPUT_PORTS)
                .map(|_| (vec![0.0; BLOCK], vec![0.0; BLOCK]))
                .collect(),
        )
    }
}

/// Render one block. The port views live in a fixed array on the stack,
/// so the render itself is the only thing that could touch the heap.
fn render(sampler: &mut DrumSampler, bufs: &mut Bufs, params: &DrumParams) {
    let mut it = bufs.0.iter_mut();
    let mut ports: [PortBuffers<'_>; NUM_OUTPUT_PORTS] = std::array::from_fn(|_| {
        let (l, r) = it.next().unwrap();
        PortBuffers {
            left: l.as_mut_slice(),
            right: r.as_mut_slice(),
        }
    });
    sampler.render_block(&mut ports, BLOCK, params, &[]);
}

#[test]
fn kit_swaps_never_touch_the_heap_on_the_audio_thread() {
    let (kit_tx, kit_rx) = bounded(1);
    let mut sampler = DrumSampler::new(kit_rx);
    sampler.set_sample_rate(48_000.0);
    sampler.pads = kit("boot");
    let params = DrumParams::default();
    let mut bufs = Bufs::new();
    render(&mut sampler, &mut bufs, &params);

    // Well past 31 sends, the list flavour's block size, so the old
    // channel would have allocated at least three times.
    let mut events = 0;
    for i in 0..200 {
        // Built and queued off the measured region: the loader's side.
        let _ = kit_tx.try_send(kit(&format!("k{i}")));
        events += heap_events(|| {
            // Something sounding every other swap, so both the "fading
            // voices" and the "nothing reads it" paths are taken.
            if i % 2 == 0 {
                sampler.note_on(drum_map::KICK, 1.0);
            }
            sampler.try_swap_kit();
            render(&mut sampler, &mut bufs, &params);
        });
    }
    assert_eq!(
        events, 0,
        "the audio thread allocated or freed {events} times across 200 kit swaps"
    );
}

#[test]
fn a_full_janitor_leaves_the_kit_parked_until_there_is_room() {
    let (kit_tx, kit_rx) = bounded(1);
    // A janitor channel nobody drains, already full.
    let (janitor_tx, janitor_rx) = bounded::<Vec<LoadedPad>>(1);
    janitor_tx.send(Vec::new()).unwrap();
    let mut sampler = DrumSampler::with_janitor(kit_rx, janitor_tx);
    sampler.set_sample_rate(48_000.0);
    sampler.pads = kit("a");
    let params = DrumParams::default();
    let mut bufs = Bufs::new();

    // Swap a → b with nothing sounding: `a` is unreferenced at once, but
    // the janitor has no room for it.
    kit_tx.send(kit("b")).unwrap();
    let events = heap_events(|| {
        sampler.try_swap_kit();
        render(&mut sampler, &mut bufs, &params);
        render(&mut sampler, &mut bufs, &params);
    });
    assert_eq!(
        events, 0,
        "a full janitor must not mean a free on the audio thread"
    );
    assert_eq!(sampler.retired_kits_parked(), 1, "kit a must stay parked");
    assert!(sampler.pads[0].name.starts_with("b:"));

    // Room again: the next block ships it.
    assert!(
        janitor_rx.recv().unwrap().is_empty(),
        "the filler goes first"
    );
    render(&mut sampler, &mut bufs, &params);
    assert_eq!(sampler.retired_kits_parked(), 0);
    let shipped = janitor_rx.try_recv().expect("kit a reaches the janitor");
    assert!(shipped[0].name.starts_with("a:"));
}

#[test]
fn every_slot_and_the_janitor_full_defers_the_swap() {
    let (kit_tx, kit_rx) = bounded(1);
    let (janitor_tx, janitor_rx) = bounded::<Vec<LoadedPad>>(1);
    janitor_tx.send(Vec::new()).unwrap();
    let mut sampler = DrumSampler::with_janitor(kit_rx, janitor_tx);
    sampler.set_sample_rate(48_000.0);
    sampler.pads = kit("k0");

    // Fill every retired slot (no render between, so nothing ships).
    let mut swaps = 0;
    for i in 1.. {
        sampler.note_on(drum_map::KICK, 1.0);
        kit_tx.send(kit(&format!("k{i}"))).unwrap();
        sampler.try_swap_kit();
        if !kit_rx_is_empty(&kit_tx) {
            break;
        }
        swaps = i;
        assert!(i < 16, "the sampler never ran out of retired slots");
    }
    let parked = sampler.retired_kits_parked();
    assert!(parked > 0);
    // The swap that found no room left both kits where they were.
    let current = format!("k{swaps}:");
    assert!(
        sampler.pads[0].name.starts_with(&current),
        "the live kit changed although nothing could be retired: {}",
        sampler.pads[0].name
    );

    // Drain the janitor and the deferred kit goes in.
    let _ = janitor_rx.recv().unwrap();
    sampler.try_swap_kit();
    assert!(
        kit_rx_is_empty(&kit_tx),
        "the deferred kit must be taken now"
    );
    let next = format!("k{}:", swaps + 1);
    assert!(sampler.pads[0].name.starts_with(&next));
}

fn kit_rx_is_empty(tx: &crossbeam_channel::Sender<Vec<LoadedPad>>) -> bool {
    tx.is_empty()
}

/// Swapping in kits of shared mono takes, marking each take on the load
/// progress, and rendering them (mono read onto both sides, next to a
/// stereo overhead) touches the heap no more than stereo kits do: never.
#[test]
fn mono_kit_swaps_and_the_progress_mark_never_touch_the_heap() {
    let (kit_tx, kit_rx) = bounded(1);
    let mut sampler = DrumSampler::new(kit_rx);
    let progress = std::sync::Arc::new(KitLoadProgress::new());
    sampler.set_load_progress(progress.clone());
    sampler.set_sample_rate(48_000.0);
    sampler.pads = mono_kit("boot");
    let params = DrumParams::default();
    let mut bufs = Bufs::new();
    render(&mut sampler, &mut bufs, &params);

    let mut events = 0;
    for i in 0..100 {
        let _ = kit_tx.try_send(mono_kit(&format!("m{i}")));
        events += heap_events(|| {
            sampler.note_on(drum_map::KICK, 1.0);
            sampler.note_on(drum_map::SNARE, 0.7);
            sampler.try_swap_kit();
            render(&mut sampler, &mut bufs, &params);
            render(&mut sampler, &mut bufs, &params);
        });
    }
    assert_eq!(
        events, 0,
        "the audio thread allocated or freed {events} times across 100 mono kit swaps"
    );
    assert_eq!(progress.kits_taken(), 100, "every take is marked");
    let peak = bufs
        .0
        .iter()
        .flat_map(|(l, r)| l.iter().chain(r))
        .fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.0, "the mono kit rendered silence: the test proves nothing");
}
