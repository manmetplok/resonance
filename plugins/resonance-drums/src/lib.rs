//! Resonance Drums - A drum sampler instrument CLAP plugin.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use drum_map::NUM_PADS;

use crossbeam_channel::{bounded, Receiver, Sender};
use resonance_plugin::*;

pub mod articulation;
pub mod choice;
#[cfg(feature = "editor")]
#[doc(hidden)]
pub mod download;
pub mod drum_map;
pub mod dsp;
#[cfg(feature = "editor")]
mod editor;
/// Test-only: draw the pad inspector at whatever width the caller gives
/// it. The editor module is otherwise private (ba todo #1377).
#[cfg(feature = "editor")]
#[doc(hidden)]
pub use editor::test_draw_pad_inspector;
/// Test-only: run one full `EditorApp::ui` frame of the editor, or drive
/// a whole editor (Library overlay included) frame by frame.
/// `DrumsEditorApp` is otherwise private (drums-plugin-rework.md §9) —
/// same shape as `test_draw_pad_inspector` above.
#[cfg(feature = "editor")]
#[doc(hidden)]
pub use editor::{
    test_kit_path_for_stepping, test_render_editor_frame, EditorFrameProbe, ProbedRect,
    ProbedText, TestEditor,
};
pub mod kit;
pub mod kit_loader;
/// The process-wide kit library and its download worker.
#[cfg(feature = "editor")]
#[doc(hidden)]
pub mod library;
mod mic_catalog;
pub mod params;
pub mod reload;
pub mod rr_display;
pub mod sample_info;
pub mod velocity;
pub mod voice;

use articulation::ArticulationWatcher;
use kit::LoadedPad;
use kit_loader::{
    BuiltKit, HandedOffKit, KitLoadProgress, KitRequest, KitStatus, LoadStats, PadMicChoices,
    DEFAULT_OVERHEAD_SETUP,
};
use mic_catalog::ManifestMicCatalog;
use params::{DrumParams, GLOBAL_PARAMS, PARAMS_PER_PAD};
use resonance_plugin::plugin::ExtraStateSaver;
use dsp::DrumSampler;

/// Shared state the loader thread, editor, and audio-thread plugin all need
/// handles to. Cheap to clone (all Arcs + one channel sender clone).
#[doc(hidden)]
#[derive(Clone)]
pub struct KitBridge {
    /// Path to the currently loaded (or last-loaded) kit manifest. Set by
    /// the loader on success; persisted in `save_state`.
    pub kit_path: Arc<Mutex<Option<PathBuf>>>,
    /// The kit a load is in flight for, with that load's generation stamp.
    /// Recorded by [`kit_loader::spawn_loader`] when it starts, cleared
    /// when that load finishes (loaded or failed). While it is set it,
    /// not `kit_path`, is the kit the user wants — see
    /// [`KitBridge::wanted_kit_path`].
    pub pending_kit: Arc<Mutex<Option<(u64, PathBuf)>>>,
    /// What the last loader hand-off put in the mailbox, and at what rate.
    /// Lets `initialize` tell whether the kit the sampler holds is still
    /// the right one. Written under `kit_handoff`.
    pub handed_off: Arc<Mutex<Option<HandedOffKit>>>,
    /// Test hook: while set, every loader waits for one message on it
    /// before decoding, so a test can hold a load "mid-decode".
    #[doc(hidden)]
    pub decode_gate: Arc<Mutex<Option<Receiver<()>>>>,
    /// Status reported by the loader, rendered by the editor.
    pub kit_status: Arc<Mutex<KitStatus>>,
    /// Host sample rate, captured in `initialize()`. Stored as `f32::to_bits`.
    /// Sentinel `0` means "not yet initialized — no audio rate is known".
    pub sample_rate: Arc<AtomicU32>,
    /// Audio-thread kit handoff: a one-slot, latest-wins mailbox. Clones
    /// go to the editor and loader thread. Send through
    /// [`kit_loader::hand_off_kit`], never directly — a plain `try_send`
    /// into a full slot drops the *newer* kit.
    pub kit_sender: Sender<Vec<LoadedPad>>,
    /// The mailbox's other end, held by the loaders so a newer kit can
    /// take a stale one back out of the slot (E3). The stale kit is
    /// dropped on the loader thread, never on the audio thread.
    pub kit_reclaim: Receiver<Vec<LoadedPad>>,
    /// Serialises the generation check and the hand-off, so an older
    /// loader that passed its check can never send after a newer one and
    /// evict it.
    pub kit_handoff: Arc<Mutex<()>>,
    /// Monotonic load stamp. Incremented each time a new loader is spawned;
    /// in-flight loaders check this before writing status/kit_path so a
    /// stale load can't clobber a newer one.
    pub load_generation: Arc<AtomicU64>,
    /// Index of mic setups available in the currently-loaded manifest.
    /// Rebuilt on each successful load and read by the editor to populate
    /// per-pad mic pickers.
    pub catalog: Arc<Mutex<ManifestMicCatalog>>,
    /// User-chosen setup keys per pad (key = position, value = setup_key).
    /// Wrapped in a Mutex so the editor can edit from the UI thread while
    /// the loader thread reads a snapshot when building a new kit.
    pub pad_choices: Arc<Mutex<[PadMicChoices; drum_map::NUM_PADS]>>,
    /// User-chosen global overhead setup key. Defaults to
    /// `DEFAULT_OVERHEAD_SETUP` and persists via plugin state.
    pub overhead_setup_key: Arc<Mutex<String>>,
    /// The plugin's parameters. Shared here so everything off the audio
    /// thread — the editor, the loader, the articulation watcher — reads
    /// a pad's articulation from the one place that holds it (see
    /// [`articulation`]), instead of from a private mirror.
    pub params: Arc<params::DrumParams>,
    /// The articulation set the kit currently in memory (or the load in
    /// flight) was built from. Not a second source of truth: it is a
    /// record of what was decoded, which is what tells the watcher a
    /// parameter has moved since. Written by [`kit_loader::spawn_loader`].
    pub loaded_articulations: Arc<Mutex<[bool; drum_map::NUM_PADS]>>,
    /// Wakes the articulation watcher so an editor click reloads without
    /// waiting for its poll. Bounded and best-effort: a full channel
    /// already has a pending wake, which is all a ping means.
    pub articulation_wake: Sender<()>,
    /// Hits the editor has asked to hear, drained by `process()` on the
    /// audio thread (ba todo #1328). Bounded, so sending never allocates
    /// and never blocks the UI thread; a full queue means the audio
    /// thread has not run since 16 clicks ago, and the extra clicks are
    /// dropped rather than queued into a burst.
    pub audition_sender: Sender<AuditionHit>,
    /// Last-played round-robin display state. Written by the audio thread
    /// after each `note_on`, read by the editor for per-pad "take N of M"
    /// indicators. Packed and unpacked by [`rr_display`]; zero means
    /// "never triggered".
    pub last_rr: Arc<[AtomicU32; NUM_PADS]>,
    /// Frames in the last block the host asked us to render. Written by
    /// `process` on the audio thread, read by the editor's status bar.
    /// Sentinel `0` means "no block has been processed yet".
    pub block_frames: Arc<AtomicU32>,
    /// Block peak of the plugin's output, across every port, as
    /// `f32::to_bits`: `[left, right]`. Written by the sampler at the end
    /// of every render, read (and decayed) by the editor's OUT meter.
    pub out_peak: Arc<[AtomicU32; 2]>,
    /// Bytes of decoded sample data currently held in memory. Published by
    /// whoever built the live kit — the loader thread on a successful load,
    /// `initialize` for the embedded fallback. Sentinel `0` = nothing loaded.
    pub kit_bytes: Arc<AtomicU64>,
    /// Per-pad identity of the sample a full-velocity hit plays, measured
    /// from the decoded takes. Published alongside every kit build; read by
    /// the inspector's SAMPLE stage. Empty until the first kit is built.
    pub pad_samples: Arc<Mutex<Vec<Option<sample_info::PadSampleInfo>>>>,
    /// Of `kit_bytes`, the bytes the last load found already decoded by
    /// another instance (through the shared sample cache, E5) — memory
    /// this kit costs nothing extra for. The process-wide total is
    /// `kit_loader::cache::global().stats().resident_bytes`.
    pub kit_shared_bytes: Arc<AtomicU64>,
    /// The last kit a loader built, kept so the next load of the same kit
    /// at the same rate rebuilds only the pads that changed (E4). Shares
    /// its sample memory with the kit the sampler plays.
    pub built_kit: Arc<Mutex<Option<BuiltKit>>>,
    /// What the last successful load did: files decoded vs found in the
    /// cache, pads reused, unreadable files. Read by tests as the decode
    /// counter.
    pub load_stats: Arc<Mutex<LoadStats>>,
    /// Decode progress, complete only once the audio thread has taken the
    /// kit (§5.4). Lock-free on every side; the sampler marks the take.
    pub load_progress: Arc<KitLoadProgress>,
}

/// One editor-requested hit on its way to the audio thread. `Copy` and
/// two words wide, so the queue is a plain preallocated ring — nothing
/// on the audio side allocates, locks or blocks to receive one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuditionHit {
    /// MIDI note of the pad to strike — the same note a host would send,
    /// so an audition takes the identical path through the sampler.
    pub note: u8,
    /// 0..1 trigger velocity.
    pub velocity: f32,
}

/// Velocity an editor audition strikes at: firm, but short of the top
/// layer, so the button demonstrates the pad rather than its loudest
/// sample. Roughly MIDI 102.
pub const AUDITION_VELOCITY: f32 = 0.8;

/// How many un-drained auditions the queue holds. One block's worth of
/// frantic clicking; beyond that the clicks are dropped.
const AUDITION_QUEUE_DEPTH: usize = 16;

impl KitBridge {
    /// Ask the audio thread to strike `note` at the standard audition
    /// velocity. Called from the editor (UI) thread.
    ///
    /// Best-effort by design: if the queue is full the click is dropped
    /// rather than queued, because a burst of stacked hits some seconds
    /// later is not what the user asked for. Nothing here can block the
    /// UI thread or the audio thread.
    pub fn audition(&self, note: u8) {
        self.audition_at(note, AUDITION_VELOCITY);
    }

    /// Audition at an explicit velocity.
    pub fn audition_at(&self, note: u8, velocity: f32) {
        let _ = self.audition_sender.try_send(AuditionHit { note, velocity });
    }

    /// Per-pad articulation, derived from the parameters: false = the
    /// pad's primary piece, true = its alternate one. This is what the
    /// kit loader is built from, so a parameter write — from the editor,
    /// host automation, or `set_plugin_param` — is what selects samples.
    pub fn articulations(&self) -> [bool; drum_map::NUM_PADS] {
        self.params.articulations()
    }

    /// Ask the articulation watcher to look now rather than at its next
    /// poll. Best-effort by design — see [`Self::articulation_wake`].
    pub fn wake_articulation_watcher(&self) {
        let _ = self.articulation_wake.try_send(());
    }

    /// The kit the user last asked for: the one a load is in flight for,
    /// else the one last loaded. A reload (re-activation, mic or
    /// articulation change) reloads this — `kit_path` alone would revert
    /// a pick still decoding to the kit before it.
    pub fn wanted_kit_path(&self) -> Option<PathBuf> {
        let pending = self.pending_kit.lock().as_ref().map(|(_, p)| p.clone());
        pending.or_else(|| self.kit_path.lock().clone())
    }

    /// The full request a reload would decode now: the wanted kit with
    /// the current mic and articulation choices. `None` with no kit.
    pub fn wanted_request(&self) -> Option<KitRequest> {
        let path = self.wanted_kit_path()?;
        Some(KitRequest {
            path,
            overhead_setup_key: self.overhead_setup_key.lock().clone(),
            pad_choices: self.pad_choices.lock().clone(),
            articulations: self.articulations(),
        })
    }

    /// Abandon an in-flight load unless it is for `keep`: a state load
    /// that names another kit (or none) supersedes it. Bumping the
    /// generation turns the loader into a no-op when it lands.
    fn supersede_pending(&self, keep: Option<&std::path::Path>) {
        let mut pending = self.pending_kit.lock();
        let Some((_, path)) = pending.as_ref() else {
            return;
        };
        if Some(path.as_path()) == keep {
            return;
        }
        let generation = self.load_generation.fetch_add(1, Ordering::AcqRel) + 1;
        *pending = None;
        // No load is outstanding any more; one the state names is
        // started by the caller (and restarts the progress).
        self.load_progress.idle(generation);
        let mut status = self.kit_status.lock();
        if matches!(*status, KitStatus::Loading { .. }) {
            *status = KitStatus::Empty;
        }
    }
}

pub struct ResonanceDrums {
    /// Parameters — shared with the editor thread via `Arc` so the UI can
    /// read and write from a separate thread. All `FloatParam` / `BoolParam`
    /// fields use atomic storage internally, so `&DrumParams` is safe to use
    /// concurrently from audio + UI.
    params: Arc<DrumParams>,
    /// Which preset is loaded and whether it has been edited since,
    /// chained in front of this plugin's own `DrumsExtraState` so the kit
    /// path and the preset identity both ride along (ba todo #1358).
    presets: Arc<resonance_plugin::presets::PresetSession>,
    sampler: DrumSampler,
    #[doc(hidden)]
    pub bridge: KitBridge,
    /// Keeps the articulation watcher thread running for as long as this
    /// plugin instance lives. Never read — dropping it stops the thread.
    _articulation_watcher: ArticulationWatcher,
    /// Receiving end of the editor's audition queue. The audio thread is
    /// the sole consumer; drained at the top of every `process()`.
    audition_receiver: Receiver<AuditionHit>,
}

impl ResonancePlugin for ResonanceDrums {
    const CLAP_ID: &'static str = "com.resonance.drums";
    const NAME: &'static str = "Resonance Drums";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str = "A drum sampler instrument";
    // `drum-machine` is the kit-level CLAP category (the old `drum` was
    // translated to it by hand; now it is declared directly).
    const FEATURES: &'static [&'static std::ffi::CStr] = &[
        features::INSTRUMENT,
        features::DRUM_MACHINE,
        features::SAMPLER,
        features::STEREO,
    ];

    const INPUT_CHANNELS: Option<u32> = None;
    const MIDI_INPUT: bool = true;

    fn new() -> Self {
        // SPSC-style handoff: audio thread is the sole consumer. Bound of 1
        // coalesces in-flight swaps: a loader finding the slot full takes
        // the stale kit back out (`kit_reclaim`) and puts its own in, so
        // if the user spams Load Kit only the newest kit reaches the audio
        // thread.
        let (kit_sender, kit_receiver): (Sender<Vec<LoadedPad>>, Receiver<Vec<LoadedPad>>) =
            bounded(1);
        // Articulation wake-ups carry no payload, so a depth of one is
        // enough: a queued ping already says "look again".
        let (articulation_wake, articulation_wake_rx): (Sender<()>, Receiver<()>) = bounded(1);
        // Editor auditions. Bounded so neither end ever allocates.
        let (audition_sender, audition_receiver): (Sender<AuditionHit>, Receiver<AuditionHit>) =
            bounded(AUDITION_QUEUE_DEPTH);
        let params = Arc::new(DrumParams::default());
        let bridge = KitBridge {
            kit_path: Arc::new(Mutex::new(None)),
            pending_kit: Arc::new(Mutex::new(None)),
            handed_off: Arc::new(Mutex::new(None)),
            decode_gate: Arc::new(Mutex::new(None)),
            kit_status: Arc::new(Mutex::new(KitStatus::Empty)),
            sample_rate: Arc::new(AtomicU32::new(0)),
            kit_reclaim: kit_receiver.clone(),
            kit_handoff: Arc::new(Mutex::new(())),
            kit_sender,
            load_generation: Arc::new(AtomicU64::new(0)),
            catalog: Arc::new(Mutex::new(ManifestMicCatalog::default())),
            pad_choices: Arc::new(Mutex::new(std::array::from_fn(|_| {
                PadMicChoices::default()
            }))),
            overhead_setup_key: Arc::new(Mutex::new(DEFAULT_OVERHEAD_SETUP.to_string())),
            params: params.clone(),
            loaded_articulations: Arc::new(Mutex::new(params.articulations())),
            articulation_wake,
            audition_sender,
            last_rr: Arc::new(std::array::from_fn(|_| AtomicU32::new(0))),
            block_frames: Arc::new(AtomicU32::new(0)),
            out_peak: Arc::new(std::array::from_fn(|_| AtomicU32::new(0))),
            kit_bytes: Arc::new(AtomicU64::new(0)),
            pad_samples: Arc::new(Mutex::new(Vec::new())),
            kit_shared_bytes: Arc::new(AtomicU64::new(0)),
            built_kit: Arc::new(Mutex::new(None)),
            load_stats: Arc::new(Mutex::new(LoadStats::default())),
            load_progress: Arc::new(KitLoadProgress::new()),
        };
        // Counted in the kit library's "used in N open drum instances".
        // The download worker is not per instance any more: the editor
        // factory opens the process-wide library (`library::shared`).
        #[cfg(feature = "editor")]
        library::register_instance(&bridge.kit_path);
        let mut sampler = DrumSampler::new(kit_receiver);
        sampler.set_load_progress(bridge.load_progress.clone());
        sampler.set_last_rr(bridge.last_rr.clone());
        sampler.set_out_peak(bridge.out_peak.clone());
        let watcher = articulation::spawn_watcher(&bridge, articulation_wake_rx);
        // The preset identity wraps the kit saver rather than replacing
        // it: chaining is why `with_extra` exists.
        let presets = resonance_plugin::presets::PresetSession::for_plugin_with_extra::<Self>(Arc::new(
            DrumsExtraState {
                kit_path: bridge.kit_path.clone(),
                overhead_setup_key: bridge.overhead_setup_key.clone(),
                pad_choices: bridge.pad_choices.clone(),
                params: params.clone(),
                reload: Some(bridge.clone()),
            },
        ));
        Self {
            params,
            presets,
            sampler,
            bridge,
            _articulation_watcher: watcher,
            audition_receiver,
        }
    }

    fn param_count(&self) -> usize {
        // master_volume + polyphony + velocity_curve + round_robin_mode,
        // then (volume, pan, mute, oh_blend, balance, articulation) per pad
        GLOBAL_PARAMS + drum_map::NUM_PADS * PARAMS_PER_PAD
    }

    fn param(&self, index: usize) -> &dyn Param {
        self.params.param_at(index)
    }

    fn output_layout(&self) -> Vec<resonance_plugin::OutputPortSpec> {
        // 7 stereo output ports: Main + 5 drum groups + Overhead, declared
        // unconditionally — the plugin has no stereo-only mode. See the pad
        // mapping in `drum_map.rs` for which pad feeds which port, and
        // `kit::OUTPUT_PORT_NAMES` for the shared name list the editor's KIT
        // card reads back.
        kit::OUTPUT_PORT_NAMES
            .iter()
            .map(|name| resonance_plugin::OutputPortSpec {
                name: std::borrow::Cow::Borrowed(name),
                channel_count: 2,
            })
            .collect()
    }

    /// Activation. The kit to load is the one the user last asked for
    /// ([`KitBridge::wanted_kit_path`]) — including a pick still decoding
    /// when the host deactivated — decoded at this rate.
    ///
    /// Under `kit_handoff`, so no loader hands off in between: the rate
    /// is published first (a loader still decoding for another rate then
    /// drops its kit), and a kit a loader handed off that the audio thread
    /// never took is taken out of the mailbox here — installed if it was
    /// decoded at this rate, freed (on this thread) if not.
    ///
    /// If the sampler then already holds the wanted kit, at this rate and
    /// with these mic and articulation choices, nothing is decoded again
    /// (E4's "`initialize` at an unchanged rate reuses the loaded kit").
    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        let wanted = self.bridge.wanted_request();
        let reuse = {
            let _handoff = self.bridge.kit_handoff.lock();
            self.bridge
                .sample_rate
                .store(sample_rate.to_bits(), Ordering::Release);
            let mut handed = self.bridge.handed_off.lock();
            if let Ok(pads) = self.bridge.kit_reclaim.try_recv() {
                match handed.as_ref() {
                    Some(h) if h.sample_rate == sample_rate => {
                        self.sampler.install_kit(pads);
                        self.bridge.load_progress.note_taken();
                    }
                    _ => {
                        drop(pads);
                        self.bridge.load_progress.note_reclaimed();
                        *handed = None;
                    }
                }
            }
            // A build at another rate cannot donate a single pad to the
            // next load; letting it go now frees its memory before that
            // load decodes, instead of after.
            let mut built = self.bridge.built_kit.lock();
            if built.as_ref().is_some_and(|b| b.sample_rate != sample_rate) {
                *built = None;
            }
            drop(built);
            let reuse = matches!(
                (handed.as_ref(), wanted.as_ref()),
                (Some(h), Some(w)) if h.sample_rate == sample_rate && h.request == *w
            );
            if !reuse {
                // The sampler goes back to the built-in kit below.
                *handed = None;
            }
            reuse
        };
        if reuse {
            self.sampler.set_sample_rate(sample_rate);
            return true;
        }

        self.sampler.load_defaults(sample_rate);
        // Publish what the fallback kit actually costs and what it holds,
        // so the status bar and the inspector's SAMPLE stage describe the
        // kit that is really loaded rather than a placeholder.
        self.publish_kit_facts(sample_rate);

        if wanted.is_none() {
            // The built-in kit is the wanted kit, and it is in place.
            self.bridge
                .load_progress
                .idle(self.bridge.load_generation.load(Ordering::Acquire));
        }
        if let Some(request) = wanted {
            kit_loader::spawn_loader(
                request.path,
                sample_rate,
                &self.bridge,
                request.overhead_setup_key,
                request.pad_choices,
                request.articulations,
            );
        }

        true
    }

    fn reset(&mut self) {
        self.sampler.reset();
    }

    /// Inactive, there is no rate to decode a kit at: a state load in this
    /// window (a full-state reload's deactivate → load → activate) only
    /// records the kit, and `initialize` loads it once.
    fn deactivate(&mut self) {
        self.bridge.sample_rate.store(0, Ordering::Release);
    }

    fn process(
        &mut self,
        outputs: &mut [resonance_plugin::OutputBuffer<'_>],
        frames: usize,
        events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
        resonance_dsp::flush_denormals();

        // Real block size for the editor's status bar.
        self.bridge
            .block_frames
            .store(frames as u32, Ordering::Relaxed);

        // Swap in a freshly loaded kit if one is waiting.
        self.sampler.try_swap_kit();

        // Snapshot the global trigger settings once per block, before any
        // note lands. Block-rate is the right granularity: they only
        // affect how a hit is *started*, and it keeps `note_on` off the
        // param objects.
        self.sampler.update_global_settings(&self.params);

        // Project the CLAP bridge's `OutputBuffer` slice into the sampler's
        // `PortBuffers` shape on the stack. The plugin declares exactly
        // `NUM_OUTPUT_PORTS` output ports in its layout so the bridge is
        // guaranteed to hand us at least that many; bail if it doesn't
        // rather than panic on the audio thread (still applying the
        // events, so no hit or choke is lost).
        if outputs.len() < kit::NUM_OUTPUT_PORTS {
            while let Some(event) = events.next_event() {
                self.apply_event(event);
            }
            self.drain_auditions();
            return;
        }
        let mut out_iter = outputs.iter_mut();
        let mut port_views: [dsp::PortBuffers<'_>; kit::NUM_OUTPUT_PORTS] =
            std::array::from_fn(|_| {
                let out = out_iter.next().expect("checked len above");
                dsp::PortBuffers {
                    left: &mut *out.left,
                    right: &mut *out.right,
                }
            });
        drop(out_iter);

        // Render the block in spans split at event times (DSP-01): every
        // event is applied at its own frame, so a hit starts where the
        // host put it, two hits on one pad in one block are two onsets,
        // and a choke cuts at its offset rather than at frame 0.
        //
        // Offsets are clamped into the block, and never move backwards:
        // an out-of-order event is applied at the frame already reached.
        // Editor auditions carry no time; they are applied just before
        // the first span, after any frame-0 host events, which is where
        // they have always landed.
        self.sampler
            .begin_block(&mut port_views, frames, &self.params);
        let last_frame = frames.saturating_sub(1);
        let mut cursor = 0usize;
        let mut auditions_drained = false;
        while let Some(event) = events.next_event() {
            let at = (event.timing() as usize).min(last_frame).max(cursor);
            if at > cursor {
                if !auditions_drained {
                    self.drain_auditions();
                    auditions_drained = true;
                }
                self.sampler.render_span(&mut port_views, cursor, at);
                cursor = at;
            }
            self.apply_event(event);
        }
        if !auditions_drained {
            self.drain_auditions();
        }
        self.sampler.render_span(&mut port_views, cursor, frames);
        self.sampler
            .end_block(&mut port_views, frames, &self.params);
    }

    fn extra_state_saver(&self) -> Option<Arc<dyn ExtraStateSaver>> {
        Some(self.presets.clone())
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::DrumsEditorFactory::new(
            self.params.clone(),
            self.bridge.clone(),
            self.presets.clone(),
        )))
    }
}

impl ResonanceDrums {
    /// Apply one host note event to the sampler, at the frame the caller
    /// has rendered up to.
    fn apply_event(&mut self, event: NoteEvent) {
        match event {
            NoteEvent::NoteOn { note, velocity, .. } => {
                self.sampler.note_on(note, velocity);
            }
            NoteEvent::NoteOff { note, .. } => {
                self.sampler.note_off(note);
            }
            NoteEvent::Choke { note, .. } => {
                self.sampler.choke_note(note);
            }
        }
    }

    /// Editor auditions (ba todo #1328), fed through the same `note_on`
    /// a MIDI hit takes, so an auditioned pad sounds exactly like a
    /// played one — same velocity layer, same round robin, same choke
    /// group, same voice allocation. `try_recv` on a bounded channel
    /// neither allocates nor blocks, and this runs whether or not the
    /// transport is rolling: the host calls `process` for as long as the
    /// plugin is active.
    fn drain_auditions(&mut self) {
        while let Ok(hit) = self.audition_receiver.try_recv() {
            self.sampler.note_on(hit.note, hit.velocity);
        }
    }

    /// Measure the kit the sampler currently holds and publish the two
    /// facts the editor displays about it: how much decoded audio is in
    /// memory, and what sample each pad plays at full velocity.
    ///
    /// Called from `initialize` for the embedded fallback kit; the loader
    /// thread publishes the same two facts for kits it loads from disk
    /// (see `kit_loader::spawn_loader`). Both callers are off the audio
    /// thread — nothing here runs in `process`.
    fn publish_kit_facts(&self, sample_rate: f32) {
        self.bridge
            .kit_bytes
            .store(self.sampler.total_sample_bytes() as u64, Ordering::Relaxed);
        self.bridge.kit_shared_bytes.store(0, Ordering::Relaxed);
        *self.bridge.pad_samples.lock() =
            sample_info::infos_for_pads(&self.sampler.pads, sample_rate);
    }
}

/// Persists the drum plugin's kit path, the globally selected overhead
/// setup, and per-pad close-mic picks alongside the plugin's params.
/// The saver holds only shared Arcs so the CLAP bridge can call save/load
/// from the main thread while the plugin is in the audio processor
/// without touching audio-thread state.
///
/// Articulations are **not** state of their own any more: they are
/// parameters, so `params_to_json` already carries them. The legacy
/// `articulations` array is still written for older builds to read, and
/// still read back for pads whose parameter is missing from the file —
/// see [`DrumsExtraState::load`].
#[doc(hidden)]
pub struct DrumsExtraState {
    pub kit_path: Arc<Mutex<Option<PathBuf>>>,
    pub overhead_setup_key: Arc<Mutex<String>>,
    pub pad_choices: Arc<Mutex<[PadMicChoices; drum_map::NUM_PADS]>>,
    /// Read-only here: the saver mirrors the articulation params into the
    /// legacy key on save, and migrates the legacy key into them on load.
    pub params: Arc<DrumParams>,
    /// The plugin's bridge, when the saver belongs to a live plugin: a
    /// load that changes the kit or its mic choices reloads the kit here,
    /// once a sample rate is known (a preset load while active — before,
    /// the new kit was only written into the state). `None` for a bare
    /// saver (tests), which then only records.
    pub reload: Option<KitBridge>,
}

impl ExtraStateSaver for DrumsExtraState {
    fn save(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut map = serde_json::Map::new();
        // The kit the user wants — a pick still decoding counts — not only
        // the last one that finished: saving mid-decode must not write the
        // kit the user just moved away from. (A pick that then fails
        // leaves `kit_path` on the previous kit, which is what plays.)
        let wanted = match &self.reload {
            Some(bridge) => bridge.wanted_kit_path(),
            None => self.kit_path.lock().clone(),
        };
        let path = wanted.map(|p| p.to_string_lossy().into_owned());
        map.insert(
            "kit_path".to_string(),
            match path {
                Some(s) => serde_json::Value::String(s),
                None => serde_json::Value::Null,
            },
        );
        map.insert(
            "overhead_setup_key".to_string(),
            serde_json::Value::String(self.overhead_setup_key.lock().clone()),
        );
        // Per-pad close-mic choices as an array of `{position: setup_key}` maps.
        let choices = self.pad_choices.lock();
        let pads_array: Vec<serde_json::Value> = choices
            .iter()
            .map(|pc| {
                let entries: serde_json::Map<String, serde_json::Value> = pc
                    .close_setups
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect();
                serde_json::Value::Object(entries)
            })
            .collect();
        map.insert(
            "pad_mic_choices".to_string(),
            serde_json::Value::Array(pads_array),
        );
        // Per-pad articulation toggles as an array of booleans. Derived
        // from the params (which `params_to_json` also saves under
        // `pad_N_articulation`); written so a build older than ba todo
        // #1325 can still open the project.
        let arts_array: Vec<serde_json::Value> = self
            .params
            .articulations()
            .iter()
            .map(|&v| serde_json::Value::Bool(v))
            .collect();
        map.insert(
            "articulations".to_string(),
            serde_json::Value::Array(arts_array),
        );
        map
    }

    /// The kit and its mic choices are the sound (the articulation
    /// toggles are params already). Path-only until kits become a library
    /// kind (plugin-preset-library.md §9.3).
    fn preset_keys(&self) -> &'static [&'static str] {
        &["kit_path", "overhead_setup_key", "pad_mic_choices"]
    }

    fn load(&self, state: &serde_json::Value) {
        // The kit the user wants, not merely the last one that finished:
        // a pick still decoding counts.
        let wanted_path = |kit_path: &Arc<Mutex<Option<PathBuf>>>| match &self.reload {
            Some(bridge) => bridge.wanted_kit_path(),
            None => kit_path.lock().clone(),
        };
        let before = (
            wanted_path(&self.kit_path),
            self.overhead_setup_key.lock().clone(),
            self.pad_choices.lock().clone(),
        );
        // An explicit `kit_path: null` clears the remembered kit (a project
        // saved with none always writes it); a document without the key —
        // a params-only preset — keeps the current kit, as the IR keeps
        // its impulse. Before the plugin is active the loader is spawned
        // from `initialize()`, where the sample rate is known.
        if let Some(v) = state.get("kit_path") {
            let path = v.as_str().map(PathBuf::from);
            // A load in flight for another kit is superseded by this one.
            if let Some(bridge) = &self.reload {
                bridge.supersede_pending(path.as_deref());
            }
            *self.kit_path.lock() = path;
        }

        if let Some(s) = state.get("overhead_setup_key").and_then(|v| v.as_str()) {
            *self.overhead_setup_key.lock() = s.to_string();
        }

        if let Some(arr) = state.get("pad_mic_choices").and_then(|v| v.as_array()) {
            let mut guard = self.pad_choices.lock();
            for (i, pad_val) in arr.iter().enumerate().take(drum_map::NUM_PADS) {
                let mut choices = PadMicChoices::default();
                if let Some(obj) = pad_val.as_object() {
                    for (k, v) in obj {
                        if let Some(s) = v.as_str() {
                            choices.close_setups.insert(k.clone(), s.to_string());
                        }
                    }
                }
                guard[i] = choices;
            }
        }

        // Legacy migration only. A project saved by any build that had
        // the `pad_N_articulation` param carries the value there, and
        // `load_params_from_json` has already applied it; re-applying the
        // array would be a second source of truth for the same fact. So
        // adopt an entry only for a pad whose param the file does not
        // have.
        if let Some(arr) = state.get("articulations").and_then(|v| v.as_array()) {
            let saved_params = state.get("params").and_then(|v| v.as_object());
            for (i, val) in arr.iter().enumerate().take(drum_map::NUM_PADS) {
                let Some(b) = val.as_bool() else { continue };
                let has_param = saved_params
                    .map(|m| m.contains_key(&format!("pad_{i}_articulation")))
                    .unwrap_or(false);
                if !has_param {
                    self.params.pads[i].articulation.set_value(if b {
                        articulation::ARTICULATION_ALT
                    } else {
                        articulation::ARTICULATION_PRIMARY
                    });
                }
            }
        }

        // The kit is the sound: a load that changed it (or its mics) while
        // a sample rate is known reloads it now.
        let after = (
            wanted_path(&self.kit_path),
            self.overhead_setup_key.lock().clone(),
            self.pad_choices.lock().clone(),
        );
        if let (Some(bridge), Some(path)) = (&self.reload, after.0.clone()) {
            let rate = f32::from_bits(bridge.sample_rate.load(Ordering::Acquire));
            if rate > 0.0 && before != after {
                kit_loader::spawn_loader(
                    path,
                    rate,
                    bridge,
                    after.1,
                    after.2,
                    bridge.articulations(),
                );
            }
        }
    }
}

resonance_plugin::export_clap!(ResonanceDrums);

