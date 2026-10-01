//! Resonance Drums - A drum sampler instrument CLAP plugin.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};
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
/// Test-only: which kit the header's ◀/▶ step from. `kit_browser` is
/// private outside this crate — same shape as `test_draw_pad_inspector`
/// above. Does not touch the shared library, so it needs no isolation
/// from the user's data dir and is not gated by `test-hooks`.
#[cfg(feature = "editor")]
#[doc(hidden)]
pub use editor::test_kit_path_for_stepping;
/// Test-only: run one full `EditorApp::ui` frame of the editor, or drive
/// a whole editor (Library overlay included) frame by frame.
/// `DrumsEditorApp` is otherwise private (drums-plugin-rework.md §9) —
/// same shape as `test_draw_pad_inspector` above.
///
/// Both of these isolate the process from the user's data dir
/// ([`library::isolate_for_tests`]) before touching the shared library, so
/// they are gated by `test-hooks` like that hook is: a release build of
/// the cdylib (`--release`, no `test-hooks`) and a headless one
/// (`--no-default-features`, no `editor` either) both carry none of this.
#[cfg(all(feature = "editor", feature = "test-hooks"))]
#[doc(hidden)]
pub use editor::{test_render_editor_frame, EditorFrameProbe, ProbedRect, ProbedText, TestEditor};
pub mod kit;
pub mod kit_loader;
pub mod kit_info_ext;
pub mod last_hit;
/// The process-wide kit library and its download worker.
#[cfg(feature = "editor")]
#[doc(hidden)]
pub mod library;
pub mod level;
pub mod mic_catalog;
pub mod pad_map;
pub mod params;
pub mod reload;
pub mod rr_display;
pub mod sample_info;
pub mod selection;
pub mod stream;
pub mod velocity;
pub mod voice;

use articulation::ArticulationWatcher;
use kit::LoadedPad;
use kit_loader::{
    BankRequest, BuiltKit, HandedOffKit, HeldKit, KitLoadProgress, KitRequest, KitStatus,
    LoadStats, MicBankSetups, PadMicChoices, DEFAULT_OVERHEAD_SETUP,
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
    /// the loader on success; persisted in `save_state` as a `kit_ref`
    /// ([`selection::KitRef`]).
    pub kit_path: Arc<Mutex<Option<PathBuf>>>,
    /// Which piece each pad of the current kit plays, its name and
    /// articulation labels, and the kit's port / choke hints (E10).
    /// Published with the hand-off of the kit it describes (loader,
    /// `play_builtin`, the built-in install in `initialize`), so it always
    /// describes the kit playing ([`pad_map::KitPadsHandle`]).
    pub kit_pads: pad_map::KitPadsHandle,
    /// The kit a load is in flight for, with that load's generation stamp.
    /// Recorded by [`kit_loader::spawn_loader`] when it starts, cleared
    /// when that load finishes (loaded or failed). While it is set it,
    /// not `kit_path`, is the kit the user wants — see
    /// [`KitBridge::wanted_kit_path`].
    pub pending_kit: Arc<Mutex<Option<(u64, PathBuf)>>>,
    /// The last kit that loaded, as a project saved mid-pick recorded it
    /// (`kit_ref_fallback`): if the kit the project wants fails to load
    /// on reopen, the loader loads this one instead of leaving the
    /// built-in kit. Consumed by the first load that finishes.
    pub kit_fallback: Arc<Mutex<Option<PathBuf>>>,
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
    /// The E15 bank setups: overhead slots 2 and 3 and the room setup
    /// (slot 1 is `overhead_setup_key`). Persists via plugin state
    /// (`mic_banks`). Change it with [`KitBridge::set_overhead_slot`] /
    /// [`KitBridge::set_room_setup`], which reload the kit.
    pub mic_banks: Arc<Mutex<MicBankSetups>>,
    /// `bleed_on` / `room_on` as the kit in memory (or the load in flight)
    /// was built with them, like `loaded_articulations`: what tells the
    /// watcher a bank param moved. Written by [`kit_loader::spawn_loader`].
    pub loaded_bank_flags: Arc<Mutex<(bool, bool)>>,
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
    /// Block peak of each output port (the louder channel), in port
    /// order, as `f32::to_bits`. Written by the sampler beside `out_peak`;
    /// read (and decayed) by the editor's Mix-tab strips.
    pub port_peak: Arc<[AtomicU32; kit::NUM_OUTPUT_PORTS]>,
    /// Every pad's last hit and the latest hit on any pad — pad,
    /// velocity, layer, take — written by the sampler with one atomic
    /// store per slot ([`last_hit`]). The editor's cell lights, its
    /// "last played take" and the status bar's last hit read it.
    pub last_hits: Arc<last_hit::LastHits>,
    /// Bytes of decoded sample data currently held in memory. Published by
    /// whoever built the live kit — the loader thread on a successful load,
    /// `initialize` for the embedded fallback. Sentinel `0` = nothing loaded.
    pub kit_bytes: Arc<AtomicU64>,
    /// Per-pad identity of the sample a full-velocity hit plays, measured
    /// from the decoded takes. Published alongside every kit build; read by
    /// the inspector's SAMPLE stage. Empty until the first kit is built.
    pub pad_samples: Arc<Mutex<Vec<Option<sample_info::PadSampleInfo>>>>,
    /// Of `kit_bytes`, the bytes another instance already held when this
    /// one got them (through the shared sample cache, E5) — memory this
    /// kit costs nothing extra for. Over the whole kit, the built-in one
    /// included; see [`kit_loader::is_shared`]. The process-wide total is
    /// `kit_loader::cache::global().stats().resident_bytes`.
    pub kit_shared_bytes: Arc<AtomicU64>,
    /// The last kit a loader built, kept so the next load of the same kit
    /// at the same rate rebuilds only the pads that changed (E4).
    ///
    /// It holds its takes alive by itself: while the sampler plays this
    /// build the two share the memory, but once the sampler moves on
    /// (to the built-in kit, at a re-activation) the build alone keeps
    /// that kit resident. So `initialize` drops it whenever it cannot
    /// donate to the next load — another rate, another kit, or none —
    /// and a direct [`kit_loader::hand_off_kit`] drops it too.
    pub built_kit: Arc<Mutex<Option<BuiltKit>>>,
    /// The built-in kit `initialize` installed, while the sampler plays
    /// it (cleared once a loaded kit is handed off), with which of its
    /// takes another instance already held. Lets a load — and the next
    /// `initialize` — tell this instance's own takes from shared ones
    /// ([`Self::kit_shared_bytes`]).
    pub builtin_kit: Arc<Mutex<Option<HeldKit>>>,
    /// What the last successful load did: files decoded vs found in the
    /// cache, pads reused, unreadable files. Read by tests as the decode
    /// counter.
    pub load_stats: Arc<Mutex<LoadStats>>,
    /// Decode progress, complete only once the audio thread has taken the
    /// kit (§5.4). Lock-free on every side; the sampler marks the take.
    pub load_progress: Arc<KitLoadProgress>,
    /// The host, once the CLAP bridge hands it over (`set_host`): told to
    /// re-read the params when the plugin moves `kit_select` itself.
    pub host: Arc<Mutex<Option<Arc<HostHandle>>>>,
    /// What this instance asked the host for, counted whether or not a
    /// host is attached — so a test can tell a user's kit pick (one
    /// undoable edit) from a state load (a rescan, never an edit).
    #[doc(hidden)]
    pub host_asks: Arc<HostAsks>,

    // --- Disk streaming (E14, slice K6b) -------------------------------
    /// Frames of each take kept in memory; the rest of a longer take
    /// streams from disk. One of [`stream::PRELOAD_CHOICES`] (0 keeps
    /// every take whole). Read by each load; persisted as plugin state
    /// (`stream_preload`). Change it with [`stream::set_preload`], which
    /// reloads the kit.
    pub stream_preload: Arc<AtomicU32>,
    /// Stream underruns since the instance started: voice-blocks that
    /// played silence for tail frames the disk reader had not delivered
    /// yet. Written by the audio thread.
    pub stream_underruns: Arc<AtomicU64>,
    /// Bytes of ring storage the streams hold (only rings that have been
    /// used hold any: [`stream::StreamSet::ring_bytes`]), on top of the
    /// kit's heads in `kit_bytes`. Written by the audio thread once a
    /// block.
    pub stream_ring_bytes: Arc<AtomicU64>,
    /// How the host renders, as it declared it (CLAP `render`):
    /// [`stream::HOST_RENDER_UNKNOWN`] (0, the default: the sampler tells
    /// from the block timing), [`stream::HOST_RENDER_REALTIME`] (1: never
    /// wait for the disk reader) or [`stream::HOST_RENDER_OFFLINE`] (2: a
    /// bounce — wait for it, up to seconds a block). Store it from the
    /// main thread whenever the host sets the mode; the audio thread
    /// reads it once a block, so it applies from the next block on.
    pub host_render_mode: Arc<AtomicU8>,
}

/// Counts of [`KitBridge`]'s requests to the host. Test surface.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct HostAsks {
    /// `announce_param_change("kit_select")`: user edits of the kit.
    pub kit_select_edits: AtomicU64,
    /// `request_params_rescan` (values).
    pub value_rescans: AtomicU64,
    /// `request_params_text_rescan` (values and text).
    pub text_rescans: AtomicU64,
    /// `announce_param_change` for an editor edit of any param other than
    /// `kit_select` ([`KitBridge::announce_param_edit`]), by id, in order:
    /// one entry per undoable edit the host is told about.
    pub param_edits: Mutex<Vec<String>>,
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
    /// poll. Best-effort by design — see [`Self::articulation_wake`]. The
    /// same thread acts on `kit_select`.
    pub fn wake_articulation_watcher(&self) {
        let _ = self.articulation_wake.try_send(());
    }

    /// Have the host re-read the params' values: the plugin moved
    /// `kit_select` itself without the user asking (a state load), or
    /// `kit_load_progress` moved. Never an undoable edit. A no-op before
    /// the bridge handed the host over (and in tests).
    pub fn request_params_rescan(&self) {
        self.host_asks.value_rescans.fetch_add(1, Ordering::Relaxed);
        if let Some(host) = self.host.lock().as_ref() {
            host.request_params_rescan();
        }
    }

    /// Have the host re-read the params' values **and text**: what an
    /// unchanged `kit_select` value names changed (a kit renamed in its
    /// slot, another parked kit).
    pub fn request_params_text_rescan(&self) {
        self.host_asks.text_rescans.fetch_add(1, Ordering::Relaxed);
        if let Some(host) = self.host.lock().as_ref() {
            host.request_params_text_rescan();
        }
    }

    /// The user changed `kit_select` from the plugin's own UI (a Library
    /// Load, the kit dropdown, ◀/▶, a relink): the host records it as one
    /// undoable edit. Call it after the value is set — and only for a
    /// user's pick: a value derived from a state load is
    /// [`Self::request_params_rescan`], or an undo would record an edit.
    pub fn announce_kit_select(&self) {
        self.host_asks
            .kit_select_edits
            .fetch_add(1, Ordering::Relaxed);
        if let Some(host) = self.host.lock().as_ref() {
            host.announce_param_change("kit_select");
        }
    }

    /// The user changed param `id` from the editor (a knob drag that just
    /// ended, a click, a pick): the host records it as one undoable edit
    /// (`HostHandle::announce_param_change`). Call it after the value is
    /// set, once per gesture. Counted in [`HostAsks::param_edits`] whether
    /// or not a host is attached.
    pub fn announce_param_edit(&self, id: &str) {
        self.host_asks.param_edits.lock().push(id.to_string());
        if let Some(host) = self.host.lock().as_ref() {
            host.announce_param_change(id);
        }
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
        // One lock at a time (see `overhead_slots`): a guard taken inside
        // the struct literal would live until the whole request is built.
        let overhead_setup_key = self.overhead_setup_key.lock().clone();
        let pad_choices = self.pad_choices.lock().clone();
        Some(KitRequest {
            path,
            overhead_setup_key,
            pad_choices,
            articulations: self.articulations(),
            preload: self.stream_preload.load(Ordering::Relaxed),
            banks: self.bank_request(),
        })
    }

    /// The E15 banks a load now builds: the setups (state) and the
    /// `bleed_on` / `room_on` params.
    pub fn bank_request(&self) -> BankRequest {
        BankRequest {
            setups: self.mic_banks.lock().clone(),
            bleed: self.params.bleed_enabled(),
            room: self.params.room_enabled(),
        }
    }

    /// Whether `bleed_on` / `room_on` moved since the kit in memory (or
    /// the load in flight) was built.
    pub fn bank_flags_moved(&self) -> bool {
        *self.loaded_bank_flags.lock() != (self.params.bleed_enabled(), self.params.room_enabled())
    }

    /// The kit-wide overhead setups, slot 1 first (`""`: an empty slot).
    ///
    /// Lock order: this (like every reader of `overhead_setup_key`,
    /// `mic_banks` and `pad_choices`) never holds two of them at once —
    /// each is cloned in its own statement, so its guard is gone before
    /// the next is taken. Holding `mic_banks` while taking
    /// `overhead_setup_key` here, against the state load's opposite
    /// order, was an ABBA deadlock with the editor polling this.
    pub fn overhead_slots(&self) -> [String; kit::MAX_OVERHEAD_SLOTS] {
        let first = self.overhead_setup_key.lock().clone();
        let extra = self.mic_banks.lock().extra_overheads.clone();
        std::array::from_fn(|slot| match slot {
            0 => first.clone(),
            n => extra[n - 1].clone(),
        })
    }

    /// Put `setup` in overhead slot `slot` (0-based: 0 is slot 1, the
    /// `overhead_setup_key`; `""` empties slots 2 and 3) and reload the
    /// kit — only the pads whose overheads change are rebuilt, and only
    /// the newly chosen setup's files decoded (E4). Returns whether a
    /// load started (none without a kit or a sample rate; the choice is
    /// kept for the next load either way). For the editor's Setup tab.
    ///
    /// Every slot plays only an overhead setup: a key naming another
    /// kind (a close mic, a room) leaves slot 1 on the piece's first
    /// overhead setup and slots 2 and 3 silent
    /// ([`kit_loader::banks::resolve_extra_banks`]).
    pub fn set_overhead_slot(&self, slot: usize, setup: &str) -> bool {
        match slot {
            0 => *self.overhead_setup_key.lock() = setup.to_string(),
            n if n < kit::MAX_OVERHEAD_SLOTS => {
                self.mic_banks.lock().extra_overheads[n - 1] = setup.to_string()
            }
            _ => return false,
        }
        reload::reload_kit_acting(self)
    }

    /// Choose the room setup (`""`: the kit's first) and reload the kit.
    /// Heard only with `room_on`.
    pub fn set_room_setup(&self, setup: &str) -> bool {
        self.mic_banks.lock().room = setup.to_string();
        reload::reload_kit_acting(self)
    }

    /// Abandon an in-flight load unless it is for `keep`: a state load
    /// that names another kit (or none) supersedes it. Bumping the
    /// generation turns the loader into a no-op when it lands.
    pub(crate) fn supersede_pending(&self, keep: Option<&std::path::Path>) {
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
    /// The host, for the audio thread's `kit_load_progress` reports
    /// (a clone of the bridge's, so `process` takes no lock).
    host: Option<Arc<HostHandle>>,
    /// `kit_load_progress` as the host sees it: served in the
    /// parameter's place ([`selection::ProgressParam`]).
    progress_param: selection::ProgressParam,
}

/// The drums' state upgrade ([`resonance_plugin::StateUpgrade`], declared
/// as `STATE_UPGRADE`): a state from an older build, brought up to what
/// this one reads, in place. Idempotent.
///
/// - v1's linear levels become dB under the new ids
///   ([`params::upgrade_v1_levels`], E9);
/// - a state from before `output_mode` existed plays Multi, as it did
///   ([`params::upgrade_output_mode`], E11/D5: Stereo is a fresh
///   instance's default only);
/// - a state from before E15 with `polyphony` at that build's maximum
///   (64) gets today's (128, [`params::upgrade_polyphony`]).
pub fn upgrade_state(state: &mut serde_json::Value) {
    params::upgrade_v1_levels(state);
    params::upgrade_output_mode(state);
    params::upgrade_polyphony(state);
}

/// `kit_select`'s and `kit_load_progress`'s host-order indices in
/// `DrumParams::param_at` (the globals added after them follow).
const KIT_SELECT_INDEX: usize = 4;
const KIT_LOAD_PROGRESS_INDEX: usize = 5;

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
        let kit_path = Arc::new(Mutex::new(None));
        let kit_pads = pad_map::KitPadsHandle::default();
        // The articulation parameters read as the current kit's labels.
        articulation::attach_kit_labels(&params, &kit_pads);
        let bridge = KitBridge {
            kit_path,
            kit_pads,
            pending_kit: Arc::new(Mutex::new(None)),
            kit_fallback: Arc::new(Mutex::new(None)),
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
            mic_banks: Arc::new(Mutex::new(MicBankSetups::default())),
            loaded_bank_flags: Arc::new(Mutex::new((false, false))),
            params: params.clone(),
            loaded_articulations: Arc::new(Mutex::new(params.articulations())),
            articulation_wake,
            audition_sender,
            last_rr: Arc::new(std::array::from_fn(|_| AtomicU32::new(0))),
            block_frames: Arc::new(AtomicU32::new(0)),
            out_peak: Arc::new(std::array::from_fn(|_| AtomicU32::new(0))),
            port_peak: Arc::new(std::array::from_fn(|_| AtomicU32::new(0))),
            last_hits: Arc::new(last_hit::LastHits::default()),
            kit_bytes: Arc::new(AtomicU64::new(0)),
            pad_samples: Arc::new(Mutex::new(Vec::new())),
            kit_shared_bytes: Arc::new(AtomicU64::new(0)),
            built_kit: Arc::new(Mutex::new(None)),
            builtin_kit: Arc::new(Mutex::new(None)),
            load_stats: Arc::new(Mutex::new(LoadStats::default())),
            load_progress: Arc::new(KitLoadProgress::new()),
            host: Arc::new(Mutex::new(None)),
            host_asks: Arc::new(HostAsks::default()),
            stream_preload: Arc::new(AtomicU32::new(stream::DEFAULT_PRELOAD)),
            stream_underruns: Arc::new(AtomicU64::new(0)),
            stream_ring_bytes: Arc::new(AtomicU64::new(0)),
            host_render_mode: Arc::new(AtomicU8::new(stream::HOST_RENDER_UNKNOWN)),
        };
        // Counted in the kit library's "used in N open drum instances".
        // The download worker is not per instance any more: the editor
        // factory opens the process-wide library (`library::shared`).
        #[cfg(feature = "editor")]
        library::register_bridge(&bridge);
        let mut sampler = DrumSampler::new(kit_receiver);
        sampler.set_load_progress(bridge.load_progress.clone());
        sampler.set_last_rr(bridge.last_rr.clone());
        sampler.set_out_peak(bridge.out_peak.clone());
        sampler.set_port_peak(bridge.port_peak.clone());
        sampler.set_last_hits(bridge.last_hits.clone());
        sampler.set_underrun_counter(bridge.stream_underruns.clone());
        sampler.set_ring_bytes_counter(bridge.stream_ring_bytes.clone());
        sampler.set_host_render_mode(bridge.host_render_mode.clone());
        let watcher = articulation::spawn_watcher(&bridge, articulation_wake_rx);
        // The preset identity wraps the kit saver rather than replacing
        // it: chaining is why `with_extra` exists.
        let presets = resonance_plugin::presets::PresetSession::for_plugin_with_extra::<Self>(Arc::new(
            DrumsExtraState {
                kit_path: bridge.kit_path.clone(),
                overhead_setup_key: bridge.overhead_setup_key.clone(),
                mic_banks: bridge.mic_banks.clone(),
                pad_choices: bridge.pad_choices.clone(),
                params: params.clone(),
                reload: Some(bridge.clone()),
            },
        ));
        debug_assert_eq!(params.param_at(KIT_SELECT_INDEX).id(), "kit_select");
        debug_assert_eq!(
            params.param_at(KIT_LOAD_PROGRESS_INDEX).id(),
            "kit_load_progress"
        );
        let progress_param =
            selection::ProgressParam::new(params.clone(), bridge.load_progress.clone());
        Self {
            params,
            presets,
            sampler,
            bridge,
            _articulation_watcher: watcher,
            audition_receiver,
            host: None,
            progress_param,
        }
    }

    fn param_count(&self) -> usize {
        // The globals, then one block per pad — see `DrumParams::param_at`.
        GLOBAL_PARAMS + drum_map::NUM_PADS * PARAMS_PER_PAD
    }

    /// Every load path — this plugin's `load_state`, the CLAP bridge's
    /// while active, a preset — runs [`upgrade_state`] before reading a
    /// param.
    const STATE_UPGRADE: Option<resonance_plugin::StateUpgrade> = Some(upgrade_state);

    fn param(&self, index: usize) -> &dyn Param {
        if index == KIT_LOAD_PROGRESS_INDEX {
            return &self.progress_param;
        }
        self.params.param_at(index)
    }

    fn output_layout(&self) -> Vec<resonance_plugin::OutputPortSpec> {
        // 7 stereo output ports: Main + 5 drum groups + Overhead, declared
        // in both output modes — a host holds the port list, so it cannot
        // change with `output_mode`; Stereo (E11) leaves all but Main
        // silent. `pad_N_output` says which pad feeds which port in Multi
        // (defaults in `drum_map.rs`), and `kit::OUTPUT_PORT_NAMES` is the
        // shared name list the editor's KIT card reads back.
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
    ///
    /// A load still pending then is either decoding for this rate or one
    /// that gave up on the rate check while the plugin was inactive (it
    /// leaves no trace of which). When the sampler already holds the
    /// wanted kit, that load is started again rather than trusted to
    /// land: every pad comes from the last build (E4), so it decodes
    /// nothing, and it clears `pending_kit` and finishes the progress —
    /// which a load that gave up never would.
    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        // A render after activation proves itself offline afresh.
        self.sampler.restart_render_timing();
        let wanted = self.bridge.wanted_request();
        let (reuse, pending) = {
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
                        // The load that sent it is over, and its kit
                        // never plays: it must not read as complete once
                        // the reclaim count reaches its ordinal. Back to
                        // "loading" — the reload below (or `idle`, with
                        // no kit wanted) moves it on.
                        self.bridge
                            .load_progress
                            .begin(self.bridge.load_generation.load(Ordering::Acquire));
                        drop(pads);
                        self.bridge.load_progress.note_reclaimed();
                        *handed = None;
                    }
                }
            }
            // A build at another rate, or of a kit that is no longer
            // wanted (none: the built-in kit; or another kit), cannot
            // donate a single pad to the next load. The sampler is about
            // to drop it too, so letting it go now frees its memory
            // before that load decodes, instead of after — and at all,
            // should that load fail.
            let mut built = self.bridge.built_kit.lock();
            let donates = built.as_ref().is_some_and(|b| {
                b.sample_rate == sample_rate && wanted.as_ref().is_some_and(|w| w.path == b.path)
            });
            if !donates {
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
            (reuse, self.bridge.pending_kit.lock().is_some())
        };
        if reuse {
            self.sampler.set_sample_rate(sample_rate);
            if pending {
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
            }
            // The progress moved (an install above, a restarted load).
            self.publish_progress();
            return true;
        }

        // What this instance holds already, so the built-in kit's takes
        // it held before (a re-activation on it) are not taken for
        // another instance's. Both stay alive until replaced below.
        let mut held = kit_loader::HeldTakes::new();
        if let Some(builtin) = self.bridge.builtin_kit.lock().as_ref() {
            builtin.held_takes(&mut held);
        }
        if let Some(built) = self.bridge.built_kit.lock().as_ref() {
            built.held_takes(&mut held);
        }
        let sources = self.sampler.load_defaults_sourced(sample_rate);
        // The editor's pad view follows the kit now in the sampler.
        crate::pad_map::publish_builtin(&self.bridge);
        let (shared_bytes, builtin) =
            kit_loader::measure_builtin_kit(&self.sampler.pads, &sources, &held);
        *self.bridge.builtin_kit.lock() = Some(builtin);
        // Publish what the fallback kit actually costs and what it holds,
        // so the status bar and the inspector's SAMPLE stage describe the
        // kit that is really loaded rather than a placeholder.
        self.publish_kit_facts(sample_rate, shared_bytes);

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
        self.publish_progress();

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

        // `kit_load_progress` mirrors the load (atomics only: no lock, no
        // allocation), and reaches 1.0 in the block that took the kit. The
        // host is asked to re-read it at the start, half-way and end of a
        // load, not per file.
        self.publish_progress();

        // Snapshot the global trigger settings once per block, before any
        // note lands. Block-rate is the right granularity: they only
        // affect how a hit is *started*, and it keeps `note_on` off the
        // param objects.
        self.sampler.update_global_settings(&self.params);

        // Project the CLAP bridge's `OutputBuffer` slice into the sampler's
        // `PortBuffers` shape on the stack. The plugin declares exactly
        // `NUM_OUTPUT_PORTS` output ports in its layout so the bridge is
        // guaranteed to hand us at least that many. If it doesn't, render
        // into no ports rather than panic on the audio thread: the block
        // still runs in full — events applied at their frames, voices
        // moving on in time, rings swept, the render timing kept — only
        // unheard.
        if outputs.len() < kit::NUM_OUTPUT_PORTS {
            self.render_events(&mut [], frames, events);
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
        self.render_events(&mut port_views, frames, events);
    }

    fn extra_state_saver(&self) -> Option<Arc<dyn ExtraStateSaver>> {
        Some(self.presets.clone())
    }

    /// A bounce waits for the disk reader; realtime never does (E14).
    /// Called on the audio thread at the top of a block while active: one
    /// atomic store.
    fn set_render_mode(&mut self, offline: bool) {
        self.bridge.host_render_mode.store(
            if offline {
                stream::HOST_RENDER_OFFLINE
            } else {
                stream::HOST_RENDER_REALTIME
            },
            Ordering::Relaxed,
        );
    }

    fn set_host(&mut self, host: Arc<HostHandle>) {
        *self.bridge.host.lock() = Some(host.clone());
        self.host = Some(host);
    }

    fn kit_info_source(&self) -> Option<Arc<dyn resonance_plugin::KitInfoSource>> {
        kit_info_ext::source(&self.bridge)
    }

    fn param_text_source(&self) -> Option<Arc<dyn resonance_plugin::ParamTextSource>> {
        // The params are shared, so a live instance's `kit_select` still
        // reads as the kit's name — and a name still picks a kit — while
        // the plugin is in the audio processor (§5.1, §8).
        Some(Arc::new(DrumParamText {
            params: self.params.clone(),
            progress: self.progress_param.clone(),
        }))
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

/// Parameter text over the shared `DrumParams`, for the CLAP bridge while
/// the plugin is active — and the live values of the two the plugin moves
/// itself, which the bridge's mirror only catches up with per block.
struct DrumParamText {
    params: Arc<DrumParams>,
    progress: selection::ProgressParam,
}

impl DrumParamText {
    fn param(&self, index: usize) -> Option<&dyn Param> {
        match index {
            KIT_LOAD_PROGRESS_INDEX => Some(&self.progress),
            i if i < params::PARAM_COUNT => Some(self.params.param_at(i)),
            _ => None,
        }
    }
}

impl resonance_plugin::ParamTextSource for DrumParamText {
    fn display(&self, index: usize, value: f64) -> Option<String> {
        self.param(index).map(|p| p.display(value))
    }

    fn parse(&self, index: usize, text: &str) -> Option<f64> {
        self.param(index)?.parse(text)
    }

    /// `kit_select` (moved by a state load, the editor, a park) and
    /// `kit_load_progress` (moved by the loader and the audio thread).
    /// Atomics only: the main thread calls this while `process` runs.
    fn live_value(&self, index: usize) -> Option<f64> {
        match index {
            KIT_SELECT_INDEX => Some(self.params.kit_select.value() as f64),
            KIT_LOAD_PROGRESS_INDEX => Some(self.progress.value() as f64),
            _ => None,
        }
    }
}

impl ResonanceDrums {
    /// Render the block in spans split at event times (DSP-01): every
    /// event is applied at its own frame, so a hit starts where the
    /// host put it, two hits on one pad in one block are two onsets,
    /// and a choke cuts at its offset rather than at frame 0.
    ///
    /// Offsets are clamped into the block, and never move backwards:
    /// an out-of-order event is applied at the frame already reached.
    /// Editor auditions carry no time; they are applied just before
    /// the first span, after any frame-0 host events, which is where
    /// they have always landed.
    fn render_events(
        &mut self,
        ports: &mut [dsp::PortBuffers<'_>],
        frames: usize,
        events: &mut EventIterator<'_>,
    ) {
        self.sampler.begin_block(ports, frames, &self.params);
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
                self.sampler.render_span(ports, cursor, at);
                cursor = at;
            }
            self.apply_event(event);
        }
        if !auditions_drained {
            self.drain_auditions();
        }
        self.sampler.render_span(ports, cursor, frames);
        self.sampler.end_block(ports, frames, &self.params);
    }

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

    /// Publish `kit_load_progress` and tell the host of a stage change.
    /// Atomics only.
    fn publish_progress(&self) {
        selection::publish_progress(
            &self.params,
            &self.bridge.load_progress,
            self.host.as_deref(),
        );
    }

    /// Measure the kit the sampler currently holds and publish the facts
    /// the editor displays about it: how much decoded audio is in memory,
    /// how much of that another instance holds too (`shared_bytes`), and
    /// what sample each pad plays at full velocity.
    ///
    /// Called from `initialize` for the embedded fallback kit; the loader
    /// thread publishes the same facts for kits it loads from disk (see
    /// `kit_loader::spawn_loader`). Both callers are off the audio
    /// thread — nothing here runs in `process`.
    fn publish_kit_facts(&self, sample_rate: f32, shared_bytes: u64) {
        self.bridge
            .kit_bytes
            .store(self.sampler.total_sample_bytes() as u64, Ordering::Relaxed);
        self.bridge
            .kit_shared_bytes
            .store(shared_bytes, Ordering::Relaxed);
        *self.bridge.pad_samples.lock() =
            sample_info::infos_for_pads(&self.sampler.pads, sample_rate);
    }
}

/// Persists the drum plugin's kit reference, the globally selected overhead
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
    /// The E15 bank setups (`mic_banks`): overhead slots 2 and 3, room.
    pub mic_banks: Arc<Mutex<MicBankSetups>>,
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

/// The mic choices a state load compares before and after.
type MicSnapshot = (String, MicBankSetups, [PadMicChoices; drum_map::NUM_PADS]);

impl DrumsExtraState {
    /// The mic choices as they stand, each lock taken and dropped on its
    /// own (see [`KitBridge::overhead_slots`] for why none is held while
    /// the next is taken).
    fn mic_snapshot(&self) -> MicSnapshot {
        let overhead = self.overhead_setup_key.lock().clone();
        let banks = self.mic_banks.lock().clone();
        let pads = self.pad_choices.lock().clone();
        (overhead, banks, pads)
    }
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
        // But that pick may yet fail, and a project that reopens on a
        // kit that will not load reopens on the built-in kit. So while it
        // is pending, the kit that last loaded goes along as the fallback
        // the reopen tries next (a reopen whose fallback has not been
        // used up yet passes its own on).
        let fallback = self.reload.as_ref().and_then(|bridge| {
            let other = |path: Option<PathBuf>| path.filter(|p| Some(p) != wanted.as_ref());
            other(self.kit_path.lock().clone()).or_else(|| other(bridge.kit_fallback.lock().clone()))
        });
        // State v2: the kit as a reference (id, name, paths), so it is
        // found again after a rename, a move or on another machine
        // (§5.2). A kit the state asked for that is not here is written
        // back verbatim, so opening and saving a project does not lose it.
        let selection = &self.params.selection;
        let kit_ref = match &wanted {
            Some(path) => Some(selection.ref_for_manifest(path)),
            None => selection.missing(),
        };
        map.insert(
            selection::KIT_REF_KEY.to_string(),
            kit_ref.map_or(serde_json::Value::Null, |r| r.to_json()),
        );
        if let Some(fallback) = fallback {
            map.insert(
                selection::KIT_REF_FALLBACK_KEY.to_string(),
                selection.ref_for_manifest(&fallback).to_json(),
            );
        }
        map.insert(
            "overhead_setup_key".to_string(),
            serde_json::Value::String(self.overhead_setup_key.lock().clone()),
        );
        // E15: overhead slots 2 and 3 and the room setup. (Bleed and room
        // on/off and every bank level are params.)
        map.insert(
            kit_loader::MIC_BANKS_STATE_KEY.to_string(),
            self.mic_banks.lock().to_json(),
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
        // Disk streaming (E14): the preload. Instance state, not sound,
        // so not a preset key.
        if let Some(bridge) = &self.reload {
            map.insert(
                stream::PRELOAD_STATE_KEY.to_string(),
                serde_json::Value::from(bridge.stream_preload.load(Ordering::Relaxed)),
            );
        }
        map
    }

    /// The kit and its mic choices are the sound (the articulation
    /// toggles are params already). The kit goes by reference, so a
    /// preset recalls it by content id after a rename or a move (§5.2).
    fn preset_keys(&self) -> &'static [&'static str] {
        &[
            selection::KIT_REF_KEY,
            "overhead_setup_key",
            kit_loader::MIC_BANKS_STATE_KEY,
            "pad_mic_choices",
        ]
    }

    fn load(&self, state: &serde_json::Value) {
        // A v1 state names its kit by path (`kit_path`); read it as the
        // v2 reference it converts to. The library root is only needed
        // (and the library only opened) when there is one to convert.
        let selection = &self.params.selection;
        let upgraded;
        let state = if state.get(selection::V1_KIT_PATH_KEY).is_some()
            || state.get(selection::V1_KIT_PATH_FALLBACK_KEY).is_some()
        {
            let mut v2 = state.clone();
            selection::upgrade_v1_state(&mut v2, selection.library.root().as_deref());
            upgraded = v2;
            &upgraded
        } else {
            state
        };
        // The kit the user wants, not merely the last one that finished:
        // a pick still decoding counts.
        let wanted_path = |kit_path: &Arc<Mutex<Option<PathBuf>>>| match &self.reload {
            Some(bridge) => bridge.wanted_kit_path(),
            None => kit_path.lock().clone(),
        };
        let preload = || {
            self.reload
                .as_ref()
                .map(|b| b.stream_preload.load(Ordering::Relaxed))
        };
        // No act on `kit_select` interleaves with this load's: from here
        // to the load it starts, the watcher waits (it would otherwise act
        // on a host write the state supersedes, clearing the state's
        // fallback kit and starting a second load).
        let _acting = selection.acting();
        let before = (
            wanted_path(&self.kit_path),
            self.mic_snapshot(),
            preload(),
        );
        // Disk streaming (E14): the preload, when the state has one.
        if let (Some(bridge), Some(frames)) = (
            &self.reload,
            stream::preload_from_state(state.get(stream::PRELOAD_STATE_KEY)),
        ) {
            bridge.stream_preload.store(frames, Ordering::Relaxed);
            // The param follows (it is not in the params state: the
            // preload travels under its own key, in frames).
            self.params
                .stream_preload
                .set_value(stream::preload_param_value(frames));
        }
        // An explicit `kit_ref: null` clears the remembered kit (a project
        // saved with none always writes it); a document without the key —
        // a params-only preset — keeps the current kit, as the IR keeps
        // its impulse. A reference that resolves to nothing is a missing
        // kit: the built-in kit plays and the reference is kept. Before
        // the plugin is active the loader is spawned from `initialize()`,
        // where the sample rate is known.
        if let Some(kit) = selection::resolve_state(state, selection) {
            // A load in flight for another kit is superseded by this one.
            if let Some(bridge) = &self.reload {
                bridge.supersede_pending(kit.path.as_deref());
                // The kit to load should this one fail; absent in states
                // saved with nothing pending.
                *bridge.kit_fallback.lock() = kit.fallback.clone();
            }
            *self.kit_path.lock() = kit.path.clone();
            // `kit_select` follows the reference: the kit's slot here. The
            // host re-reads it — a rescan, never an edit: a state load
            // (an undo's included) is no user's pick.
            let text_changed = selection::adopt_state_kit(&self.params, &kit);
            if let Some(bridge) = &self.reload {
                if text_changed {
                    bridge.request_params_text_rescan();
                } else {
                    bridge.request_params_rescan();
                }
            }
        }

        if let Some(s) = state.get("overhead_setup_key").and_then(|v| v.as_str()) {
            *self.overhead_setup_key.lock() = s.to_string();
        }
        // The bank setups travel with the mic choices: every state and
        // preset that names `overhead_setup_key` has carried `mic_banks`
        // since E15. One that names the mic choices without it predates
        // the banks and meant none — slots 2 and 3 empty, the default
        // room — whether it is a project reopened or a preset recalled
        // over an instance that has banks set. A document with neither (a
        // params-only preset) leaves the instance's as they are, like
        // every other key it leaves out.
        if let Some(banks) = state.get(kit_loader::MIC_BANKS_STATE_KEY) {
            *self.mic_banks.lock() = MicBankSetups::from_json(banks);
        } else if state.get("overhead_setup_key").is_some() {
            *self.mic_banks.lock() = MicBankSetups::default();
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
            self.mic_snapshot(),
            preload(),
        );
        if let Some(bridge) = &self.reload {
            let rate = f32::from_bits(bridge.sample_rate.load(Ordering::Acquire));
            if rate > 0.0 && before != after {
                let (path, (overhead_setup_key, _, pad_choices), _) = after;
                match path {
                    Some(path) => kit_loader::spawn_loader(
                        path,
                        rate,
                        bridge,
                        overhead_setup_key,
                        pad_choices,
                        bridge.articulations(),
                    ),
                    // No kit (or a missing one) while running: the
                    // built-in kit replaces whatever played (D7).
                    None => selection::play_builtin(bridge),
                }
            }
        }
    }
}

resonance_plugin::export_clap!(ResonanceDrums);

