//! The core audio engine. `AudioEngine::new` wires up the cpal output
//! stream and spawns the engine control thread; the control thread's
//! command dispatch and per-concern handlers live in the submodules
//! (`thread`, `transport`, `tracks`, `clips`, `midi`, `plugins`,
//! `busses`, plus `scan` and `bounce`).

/// Ring buffer size for recording input: ~10 seconds at 96kHz stereo.
/// Sized as a safety margin between the cpal input callback (producer)
/// and the engine control thread's drain-to-WAV loop (consumer); the
/// engine thread wakes at ~60 Hz, so even a pathological scheduling
/// gap fits inside this.
pub(crate) const RECORDING_RING_SIZE: usize = 96000 * 2 * 10;
pub(crate) use crate::limits::MAX_BUSSES;

use indexmap::IndexMap;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use ringbuf::traits::Split;

use crate::clap_host::SyncClapInstance;
use crate::midi_clock::MidiClockEvent;
use crate::midi_hardware::{LiveControlEvent, LiveMidiEvent};
use crate::mixer;
use crate::platform::{self, DeviceDirection};
use crate::stream_errors::{format_underrun_line, UnderrunRateLimiter};
use crate::types::*;

mod bounce;
pub use bounce::{
    export_stems, render_stem, stem_filter, stem_project_range, to_audio_clip, to_wav,
    try_lock_with_backoff, write_stem_wav, StemFilter,
};
mod bounce_common;
pub use bounce_common::midi_render_range;

/// Copy-on-write helper for the `ArcSwap<TempoMap>` shared with the
/// audio thread. The audio side does wait-free `load()`s; this helper
/// is the single-writer mutation path used by every engine-thread
/// site that previously held a `RwLock<TempoMap>::write()`.
pub(crate) fn rcu_tempo<F: FnOnce(&mut TempoMap)>(
    map: &arc_swap::ArcSwap<TempoMap>,
    f: F,
) {
    let mut new = (**map.load()).clone();
    f(&mut new);
    map.store(Arc::new(new));
}

pub(crate) mod audition;
pub use audition::{
    compute_sync_ratio, load_audition_source, set_audition_options_in_place,
    start_audition_in_place, stop_audition_in_place, AuditionSource,
};
mod bounce_realtime;
mod busses;
mod clips;
pub use clips::transcode_to_wav;
pub use clips::{
    detect_clip_tempo_in_place, set_clip_fade_in_place, set_clip_gain_in_place,
    set_clip_warp_in_place, set_clip_warp_markers_in_place, MAX_CLIP_GAIN_DB, MIN_CLIP_GAIN_DB,
};
mod import_pool;
pub use import_pool::{import_one_to_pool, run_pool_import, PoolImportOutcome};
mod master;
pub(crate) mod midi;
mod midi_map;
mod plugins;
pub(crate) mod reference;
mod scan;
mod thread;
mod tracks;
mod transport;

/// Shared state between the engine control thread and the audio callback.
/// `pub` (not `pub(crate)`) only so `__test_support` can re-export it for
/// integration tests; the `engine` module itself stays private.
pub struct SharedState {
    /// Current playhead position in sample frames.
    pub playhead: AtomicU64,
    /// Whether playback is active.
    pub playing: AtomicBool,
    /// Whether recording is active.
    pub recording: AtomicBool,
    /// Whether any track is monitoring input.
    pub monitoring: AtomicBool,
    /// Master volume as linear gain (AtomicU32 bit-punned f32).
    pub master_volume_bits: AtomicU32,
    /// Master volume the previous audio block ended on (bit-punned
    /// f32). Audio thread only; the master pass ramps from this to
    /// `master_volume_bits` per sample to avoid zipper noise.
    pub master_last_volume_bits: AtomicU32,
    /// Master peak level L (AtomicU32 bit-punned f32), for VU meters.
    pub master_peak_l_bits: AtomicU32,
    /// Master peak level R (AtomicU32 bit-punned f32), for VU meters.
    pub master_peak_r_bits: AtomicU32,
    /// When true, the mixer skips the master FX chain (everything in
    /// `MasterBus::plugin_ids`). Fader + peak metering are unaffected.
    pub master_fx_bypassed: AtomicBool,
    /// Flag: recording ring buffer overflowed (samples were dropped).
    pub recording_overflow: AtomicBool,
    /// Channel count of the currently-active input stream, or 0 when
    /// no stream is open. Used by the mix callback to de-interleave
    /// per-track monitor audio from a multi-channel input device.
    pub input_channels: AtomicU16,
    /// Loop (cycle) playback enabled: when true, the audio callback
    /// wraps the playhead from `loop_out` back to `loop_in`.
    pub loop_enabled: AtomicBool,
    pub loop_in: AtomicU64,
    pub loop_out: AtomicU64,
    /// True while a record-with-count-in is in flight. The mixer uses
    /// this to pick its count-in branch (hold the playhead, skip
    /// track/clip rendering, render metronome ticks and monitoring).
    /// The engine control thread clears it after opening the
    /// recording stream so normal playback can resume on the next
    /// buffer.
    pub count_in_active: AtomicBool,
    /// Count-in frames remaining before the last click fires. When
    /// this hits zero while `count_in_active` is still set, the mixer
    /// stops emitting metronome ticks but keeps holding the playhead
    /// until the control thread has opened the recording stream.
    pub count_in_remaining: AtomicU64,
    /// Total count-in frames at the moment count-in was armed. Used
    /// with `count_in_remaining` to derive elapsed frames for beat
    /// alignment inside the mixer's count-in branch.
    pub count_in_total: AtomicU64,
    /// Cooperative cancel flag for the offline bounce renderer
    /// (`bounce::to_audio_clip`). The renderer polls this between
    /// chunks and aborts when it flips to true. The realtime bounce
    /// path doesn't need it — its cancel goes through `handle_pause`
    /// directly — but stays in shared state so the offline renderers
    /// running on their worker threads can be aborted from the same
    /// `CancelBounce` command without threading another channel.
    pub bounce_cancel: AtomicBool,
    /// Reference A/B monitor snapshot. Published by the control thread
    /// (`reference::ReferencePlayer::publish`) and read lock-free by the
    /// audio callback to replace the post-master output with the active
    /// reference's PCM. Never consulted by any offline/realtime bounce
    /// path, so exports always render the processed mix.
    pub reference: reference::ReferenceMonitor,
    /// Latest processed-mix loudness/peak/range snapshot, published by the
    /// audio callback's mix metering tap each block and read lock-free by
    /// the control thread to answer `PollABMeters`. Holds its last value
    /// while the mix isn't playing (e.g. while auditioning a reference).
    pub mix_meter: resonance_metering::AtomicMeterSnapshot,
    /// Latest active-reference loudness/peak/range snapshot, published by
    /// the audio callback's reference metering tap while a reference is
    /// auditioned (post loudness-match/trim gain). Forwarded to the UI only
    /// when a reference is active; see `reference::handle_poll_ab_meters`.
    pub ref_meter: resonance_metering::AtomicMeterSnapshot,
    /// Lock-free snapshot of the engine's aux-send table, published by
    /// the control thread on every send add/remove/clear and read once
    /// per block by the live mixer and the offline bounce renderer. The
    /// authoritative table lives on the control thread
    /// (`HandlerState::aux_sends`); this is the audio-thread-visible copy
    /// so the render path needs no lock. Empty until the first send is
    /// created, so projects without sends pay nothing.
    pub aux_sends: arc_swap::ArcSwap<Vec<AuxSend>>,

    // -- Audition preview (doc #175) --
    /// Decoded preview source, published wait-free by the engine thread and
    /// read by the audio callback. `None` when no preview is loaded. See
    /// [`audition`].
    pub audition_source: arc_swap::ArcSwapOption<audition::AuditionSource>,
    /// Whether a preview is currently playing. The audio callback checks this
    /// first each block; it clears the flag itself when a non-looping preview
    /// reaches the end.
    pub audition_playing: AtomicBool,
    /// Audition playhead in source frames, stored as bit-punned `f64` (it can
    /// be fractional under sync-to-tempo varispeed). Sole writer is the audio
    /// callback; the engine thread reads it for `AuditionPosition` events.
    pub audition_pos_bits: AtomicU64,
    /// Loop the preview when it reaches the end (vs. stopping).
    pub audition_loop: AtomicBool,
    /// Sync-to-tempo (varispeed) enabled for the preview.
    pub audition_sync: AtomicBool,
    /// Playback ratio (source frames per output frame) as bit-punned `f32`,
    /// computed by the engine thread; `1.0` is natural speed.
    pub audition_ratio_bits: AtomicU32,
    /// Latched by the audio callback when a non-looping preview reaches its
    /// end; consumed by the engine thread to emit `AuditionStopped` once.
    pub audition_finished: AtomicBool,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            playhead: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            recording: AtomicBool::new(false),
            monitoring: AtomicBool::new(false),
            master_volume_bits: AtomicU32::new(1.0f32.to_bits()),
            master_last_volume_bits: AtomicU32::new(1.0f32.to_bits()),
            master_peak_l_bits: AtomicU32::new(0),
            master_peak_r_bits: AtomicU32::new(0),
            master_fx_bypassed: AtomicBool::new(false),
            recording_overflow: AtomicBool::new(false),
            input_channels: AtomicU16::new(0),
            loop_enabled: AtomicBool::new(false),
            loop_in: AtomicU64::new(0),
            loop_out: AtomicU64::new(0),
            count_in_active: AtomicBool::new(false),
            count_in_remaining: AtomicU64::new(0),
            count_in_total: AtomicU64::new(0),
            bounce_cancel: AtomicBool::new(false),
            reference: reference::ReferenceMonitor::default(),
            mix_meter: resonance_metering::AtomicMeterSnapshot::new(),
            ref_meter: resonance_metering::AtomicMeterSnapshot::new(),
            aux_sends: arc_swap::ArcSwap::from_pointee(Vec::new()),
            audition_source: arc_swap::ArcSwapOption::empty(),
            audition_playing: AtomicBool::new(false),
            audition_pos_bits: AtomicU64::new(0),
            audition_loop: AtomicBool::new(false),
            audition_sync: AtomicBool::new(false),
            audition_ratio_bits: AtomicU32::new(1.0f32.to_bits()),
            audition_finished: AtomicBool::new(false),
        }
    }
}

/// Error returned by [`AudioEngine::send`] when the engine thread's
/// command channel has been dropped. Wraps the original command so the
/// caller can retry, log, or surface a "engine disconnected" message
/// to the user.
///
/// In practice this happens after `AudioEngine::shutdown` (or `Drop`)
/// has joined the engine thread, or — in pathological cases — if the
/// engine thread panicked. Either way the command will not be acted
/// on and the caller should treat it as a fatal-ish state.
#[derive(Debug)]
pub struct EngineSendError(pub AudioCommand);

impl std::fmt::Display for EngineSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "audio engine command channel disconnected; dropped command: {:?}",
            self.0
        )
    }
}

impl std::error::Error for EngineSendError {}

/// One-shot eprintln when the command channel first disconnects, as a
/// safety net for call sites that intentionally `let _ =` the
/// `EngineSendError` returned by `AudioEngine::send`. Uses an atomic
/// latch so a stuck app doesn't flood stderr. Lives at module scope
/// so the test-only `for_test_disconnected` path can reset it (see
/// `__test_support::__reset_engine_disconnect_latch_for_test`).
fn report_engine_disconnect_once() {
    use std::sync::atomic::Ordering;
    if !ENGINE_DISCONNECT_REPORTED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "audio: engine command channel disconnected — subsequent send() calls will return EngineSendError"
        );
    }
}

static ENGINE_DISCONNECT_REPORTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Test-only hook: clear the one-shot disconnect-reported latch so a
/// regression test can drive `send` through the disconnect branch
/// without depending on prior test ordering.
#[doc(hidden)]
pub fn __reset_engine_disconnect_latch_for_test() {
    ENGINE_DISCONNECT_REPORTED.store(false, std::sync::atomic::Ordering::Relaxed);
}

/// The audio engine.
#[allow(dead_code)]
pub struct AudioEngine {
    cmd_tx: Sender<AudioCommand>,
    event_rx: Receiver<AudioEvent>,
    _stream: Option<cpal::Stream>,
    /// Join handle for the engine control thread. `Drop` sends a
    /// `ShutDown` command (which breaks the thread's loop, since the
    /// thread's own `cmd_tx_retry` keeps the channel from ever
    /// returning `Disconnected`) and then joins.
    engine_thread: Option<std::thread::JoinHandle<()>>,
    // Shared state for live stream rebuilding (e.g. buffer size changes)
    shared: Arc<SharedState>,
    tracks: Arc<parking_lot::RwLock<IndexMap<TrackId, Track>>>,
    busses: Arc<parking_lot::RwLock<IndexMap<BusId, Bus>>>,
    master: Arc<parking_lot::RwLock<MasterBus>>,
    clips: Arc<parking_lot::RwLock<Vec<AudioClip>>>,
    midi_clips: Arc<parking_lot::RwLock<Vec<MidiClip>>>,
    plugins:
        Arc<parking_lot::RwLock<IndexMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    /// Monitor input ring buffer's producer. Wrapped in `Mutex` solely
    /// so the same `Arc` can be handed to successive input-stream
    /// builders across device changes — there is only ever one writer
    /// thread (the cpal/PipeWire input callback) at a time, the engine
    /// thread never `.lock()`s, and the callback always uses `try_lock`.
    /// In steady state the CAS is uncontended.
    monitor_prod: Arc<parking_lot::Mutex<ringbuf::HeapProd<f32>>>,
    sample_rate: u32,
    channels: usize,
    quantum: usize,
}

impl AudioEngine {
    /// Create and start the audio engine. Returns the engine handle.
    pub fn new() -> Result<Self, String> {
        // Replace ALSA's default stderr error handler before any cpal
        // / device enumeration so the startup PCM probing doesn't
        // spam "Cannot open device /dev/dsp" and friends. Idempotent.
        platform::silence_alsa_diagnostic_output();

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "No audio output device found".to_string())?;

        let device_name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| "<unnamed>".to_string());

        let config = device
            .default_output_config()
            .map_err(|e| format!("Failed to get default output config: {}", e))?;

        let channels = config.channels() as usize;
        let default_rate = config.sample_rate();

        // Prefer the PipeWire graph sample rate to avoid resampling.
        // cpal's default_output_config often returns 44100 via ALSA compat, but the
        // actual hardware/graph runs at a different rate -- causing PipeWire to resample
        // every buffer and inflating the quantum (e.g. 1102 frames instead of 128).
        let sample_rate = platform::pick_sample_rate(&device, &config, DeviceDirection::Output);

        // Query PipeWire quantum to size buffers relative to the actual period.
        let probed_quantum = platform::pipewire_quantum();
        let probed_max_quantum = platform::pipewire_max_quantum();
        let quantum = probed_quantum.unwrap_or(1024) as usize;
        let max_quantum = probed_max_quantum.unwrap_or(2048) as usize;
        let buf_frames = max_quantum.max(quantum * 2).max(256);

        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<AudioCommand>();
        let (event_tx, event_rx) = crossbeam_channel::unbounded::<AudioEvent>();
        // Bounded so a stuck engine thread can never let hardware
        // MIDI events queue without bound. 1024 fits a comfortable
        // burst at typical engine-thread cadence (~60 Hz wakeups).
        let (live_midi_tx, live_midi_rx) = crossbeam_channel::bounded::<LiveMidiEvent>(1024);
        // Separate channel for the dedicated control-surface input. Same
        // bound + rationale as the per-track live MIDI channel above.
        let (live_control_tx, live_control_rx) =
            crossbeam_channel::bounded::<LiveControlEvent>(1024);
        // MIDI clock arrives at 24 PPQN (≈48 msgs/sec at 120 BPM)
        // plus Start/Stop/Continue. 4096 covers seconds of bursty
        // input even if the engine thread stalls.
        let (clock_tx, clock_rx) = crossbeam_channel::bounded::<MidiClockEvent>(4096);

        let shared = Arc::new(SharedState::default());

        let shared_audio = Arc::clone(&shared);

        let tracks: Arc<parking_lot::RwLock<IndexMap<TrackId, Track>>> =
            Arc::new(parking_lot::RwLock::new(IndexMap::new()));
        let busses: Arc<parking_lot::RwLock<IndexMap<BusId, Bus>>> =
            Arc::new(parking_lot::RwLock::new(IndexMap::new()));
        let master: Arc<parking_lot::RwLock<MasterBus>> =
            Arc::new(parking_lot::RwLock::new(MasterBus::new()));
        let clips: Arc<parking_lot::RwLock<Vec<AudioClip>>> =
            Arc::new(parking_lot::RwLock::new(Vec::new()));
        let midi_clips: Arc<parking_lot::RwLock<Vec<MidiClip>>> =
            Arc::new(parking_lot::RwLock::new(Vec::new()));

        let tracks_audio = Arc::clone(&tracks);
        let busses_audio = Arc::clone(&busses);
        let master_audio = Arc::clone(&master);
        let clips_audio = Arc::clone(&clips);
        let midi_clips_audio = Arc::clone(&midi_clips);

        let tempo_map: Arc<arc_swap::ArcSwap<TempoMap>> =
            Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default()));
        let tempo_audio = Arc::clone(&tempo_map);

        // Plugin instances shared between engine thread and audio callback
        let plugins: Arc<
            parking_lot::RwLock<IndexMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>>,
        > = Arc::new(parking_lot::RwLock::new(IndexMap::new()));
        let plugins_audio = Arc::clone(&plugins);

        // Plugin-delay-compensation table: published by the engine
        // thread on topology changes, loaded wait-free by the audio
        // callback. See `crate::latency` for the compensation model.
        let latency_comp: Arc<arc_swap::ArcSwap<crate::latency::LatencyComp>> = Arc::new(
            arc_swap::ArcSwap::from_pointee(crate::latency::LatencyComp::empty()),
        );
        let latency_comp_audio = Arc::clone(&latency_comp);

        let mut stream_config: cpal::StreamConfig = config.into();
        stream_config.sample_rate = sample_rate;
        stream_config.buffer_size = cpal::BufferSize::Fixed(quantum as cpal::FrameCount);
        let audio_sample_rate = sample_rate;

        let audio_buf_frames = buf_frames;
        let audio_quantum = quantum;
        // Rate-limited counter for `StreamError::BufferUnderrun` events.
        // cpal 0.17 surfaces ALSA/JACK underruns through `err_fn`
        // (previously they went to cpal-internal stderr). On a busy
        // desktop with PipeWire that can fire many times a second
        // under normal UI load, so we coalesce into one summary line
        // per `UNDERRUN_REPORT_INTERVAL` instead of spamming.
        let underrun_limiter = Arc::new(UnderrunRateLimiter::new());
        let build_stream = |config: &cpal::StreamConfig| {
            // Clone captures that the closure needs to own
            let shared_audio = Arc::clone(&shared_audio);
            let tracks_audio = Arc::clone(&tracks_audio);
            let busses_audio = Arc::clone(&busses_audio);
            let master_audio = Arc::clone(&master_audio);
            let clips_audio = Arc::clone(&clips_audio);
            let midi_clips_audio = Arc::clone(&midi_clips_audio);
            let plugins_audio = Arc::clone(&plugins_audio);
            let tempo_audio = Arc::clone(&tempo_audio);
            let latency_comp_audio = Arc::clone(&latency_comp_audio);
            let underrun_limiter = Arc::clone(&underrun_limiter);
            let mut track_buf_l = vec![0.0f32; audio_buf_frames];
            let mut track_buf_r = vec![0.0f32; audio_buf_frames];
            // Pre-allocate MAX_BUSSES stereo buffers so adding a bus at
            // runtime never allocates on the audio thread. mix_audio only
            // uses the first N slots where N = current bus count.
            let mut bus_bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..MAX_BUSSES)
                .map(|_| {
                    (
                        vec![0.0f32; audio_buf_frames],
                        vec![0.0f32; audio_buf_frames],
                    )
                })
                .collect();
            // Per-plugin-output-port scratch used for multi-output
            // instruments (resonance-drums declares 7 ports; this pool
            // carries room for a couple more).
            let mut port_scratch: Vec<(Vec<f32>, Vec<f32>)> = (0
                ..crate::mixer::MAX_PLUGIN_OUTPUT_PORTS)
                .map(|_| {
                    (
                        vec![0.0f32; audio_buf_frames],
                        vec![0.0f32; audio_buf_frames],
                    )
                })
                .collect();
            let mut note_event_buf: Vec<PendingNoteEvent> =
                Vec::with_capacity(mixer::MAX_MIDI_EVENTS_PER_BUFFER);
            // Pre-sized stash for MIDI events that couldn't be delivered
            // because the UI thread held a plugin's mutex; replayed on
            // the next successful lock so notes don't stick or vanish.
            let mut midi_stash = mixer::MidiStash::new();
            // Monitor scratch + ring are sized for the widest multi-channel
            // interleaved input we're likely to see (e.g. an 18-in audio
            // interface). 32 channels × a few blocks of headroom covers
            // everything reasonable without leaking meaningful RAM.
            use crate::limits::MAX_INPUT_CHANNELS;
            let mut monitor_temp = vec![0.0f32; audio_buf_frames * MAX_INPUT_CHANNELS];
            let monitor_ring = ringbuf::HeapRb::<f32>::new(audio_quantum * MAX_INPUT_CHANNELS * 4);
            let (prod, mut monitor_cons) = monitor_ring.split();
            // A/B metering taps (mix + reference). Pre-size their
            // de-interleave scratch to the callback buffer so the realtime
            // feed path never allocates.
            let mut ab_meters = reference::ABMeters::new(audio_sample_rate as f32);
            ab_meters.reserve(audio_buf_frames);

            // Pre-fault every page of the audio-thread scratch so the cpal
            // callback isn't the first writer. `vec![0.0f32; N]` and
            // `HeapRb::new(N)` both come from anonymous mmap / calloc,
            // which hands back lazy zero-fill pages — the kernel only
            // commits a physical page on first *write*. Doing those
            // writes from inside the realtime callback fires minor page
            // faults under cpal's deadline and cpal 0.17 reports each as
            // `StreamError::BufferUnderrun`, flooding the log (and
            // glitching audio) for the first second or two after
            // `stream.play()`. Same pattern as `DelayLine::new` in
            // resonance-dsp (commit f0de785); see `prefault.rs`.
            use crate::prefault::prefault_f32;
            prefault_f32(&mut track_buf_l);
            prefault_f32(&mut track_buf_r);
            for (l, r) in bus_bufs.iter_mut() {
                prefault_f32(l);
                prefault_f32(r);
            }
            for (l, r) in port_scratch.iter_mut() {
                prefault_f32(l);
                prefault_f32(r);
            }
            prefault_f32(&mut monitor_temp);
            let result = device.build_output_stream(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    mixer::mix_audio(
                        data,
                        channels,
                        &shared_audio,
                        &tracks_audio,
                        &busses_audio,
                        &master_audio,
                        &clips_audio,
                        &midi_clips_audio,
                        &plugins_audio,
                        &tempo_audio,
                        &latency_comp_audio,
                        audio_sample_rate,
                        &mut track_buf_l,
                        &mut track_buf_r,
                        &mut bus_bufs,
                        &mut port_scratch,
                        &mut note_event_buf,
                        &mut midi_stash,
                        &mut monitor_cons,
                        &mut monitor_temp,
                        audio_buf_frames,
                        audio_quantum,
                        &mut ab_meters,
                    );
                },
                move |err| match err {
                    cpal::StreamError::BufferUnderrun => {
                        if let Some(report) =
                            underrun_limiter.record(std::time::Instant::now())
                        {
                            eprintln!("{}", format_underrun_line("output", &report));
                        }
                    }
                    other => {
                        eprintln!("Audio stream error: {}", other);
                    }
                },
                None,
            );
            result.map(|stream| (stream, prod))
        };

        let (stream, monitor_prod_raw, used_fixed_buffer) = match build_stream(&stream_config) {
            Ok((stream, prod)) => (stream, prod, true),
            Err(fixed_err) => {
                // Fall back to default buffer size if fixed quantum was rejected.
                let mut fallback_config = stream_config.clone();
                fallback_config.buffer_size = cpal::BufferSize::Default;
                match build_stream(&fallback_config) {
                    Ok((stream, prod)) => {
                        eprintln!(
                                "audio: Fixed({}) rejected ({}) — falling back to BufferSize::Default (HIGH LATENCY)",
                                quantum, fixed_err
                            );
                        (stream, prod, false)
                    }
                    Err(e) => {
                        return Err(format!("Failed to build output stream: {}", e));
                    }
                }
            }
        };

        // One-line negotiation summary so latency regressions are diagnosable
        // from stderr alone. `probed_*` being None means the pw-metadata
        // subprocess failed and we're running on the conservative fallback
        // numbers, which is usually the cause of "why is latency higher than
        // the pipewire quantum".
        eprintln!(
            "audio: device={:?} sample_rate={} (cpal_default={}) quantum={} (probed={:?}) max_quantum={} (probed={:?}) buf_frames={} fixed_buffer={}",
            device_name,
            sample_rate,
            default_rate,
            quantum,
            probed_quantum,
            max_quantum,
            probed_max_quantum,
            buf_frames,
            used_fixed_buffer,
        );

        let monitor_prod = Arc::new(parking_lot::Mutex::new(monitor_prod_raw));
        let monitor_prod_audio = Arc::clone(&monitor_prod);

        stream
            .play()
            .map_err(|e| format!("Failed to start stream: {}", e))?;

        // Spawn the engine control thread
        let shared_ctrl = Arc::clone(&shared);
        let tracks_ctrl = Arc::clone(&tracks);
        let busses_ctrl = Arc::clone(&busses);
        let master_ctrl = Arc::clone(&master);
        let clips_ctrl = Arc::clone(&clips);
        let midi_clips_ctrl = Arc::clone(&midi_clips);
        let tempo_ctrl = Arc::clone(&tempo_map);
        let plugins_ctrl = Arc::clone(&plugins);
        let latency_comp_ctrl = Arc::clone(&latency_comp);

        let cmd_tx_retry = cmd_tx.clone();
        let engine_thread = std::thread::Builder::new()
            .name("resonance-engine".into())
            .spawn(move || {
                thread::engine_thread(
                    cmd_rx,
                    cmd_tx_retry,
                    event_tx,
                    shared_ctrl,
                    tracks_ctrl,
                    busses_ctrl,
                    master_ctrl,
                    clips_ctrl,
                    midi_clips_ctrl,
                    tempo_ctrl,
                    plugins_ctrl,
                    latency_comp_ctrl,
                    monitor_prod_audio,
                    live_midi_tx,
                    live_midi_rx,
                    live_control_tx,
                    live_control_rx,
                    clock_tx,
                    clock_rx,
                    sample_rate,
                    buf_frames,
                    quantum,
                );
            })
            .map_err(|e| format!("Failed to spawn engine thread: {}", e))?;

        Ok(Self {
            cmd_tx,
            event_rx,
            _stream: Some(stream),
            engine_thread: Some(engine_thread),
            shared,
            tracks,
            busses,
            master,
            clips,
            midi_clips,
            plugins,
            tempo_map,
            monitor_prod,
            sample_rate,
            channels,
            quantum,
        })
    }

    /// Send a command to the audio engine.
    ///
    /// Returns `Err(EngineSendError)` if the engine thread's command
    /// channel has been dropped (post-shutdown or panic). The returned
    /// error carries the original command so the caller can choose to
    /// retry, surface a UI message, or log and move on. The first
    /// disconnect of the process lifetime is also reported once on
    /// stderr so call sites that ignore the result (via `let _ =`)
    /// don't fail completely silently.
    #[must_use = "ignoring an engine send failure swallows a user-visible command (Play, SetVolume, …); use `let _ = …` only after deciding the loss is acceptable"]
    pub fn send(&self, cmd: AudioCommand) -> Result<(), EngineSendError> {
        match self.cmd_tx.send(cmd) {
            Ok(()) => Ok(()),
            Err(e) => {
                report_engine_disconnect_once();
                Err(EngineSendError(e.0))
            }
        }
    }

    /// Best-effort synchronous shutdown handshake.
    ///
    /// Sends `Stop` (which silences every CLAP instrument and emits
    /// `All Notes Off` on every connected hardware MIDI output), waits
    /// for the engine to ack with `AudioEvent::Stopped`, then sends
    /// `ShutDown` and joins the engine thread. Returns once the thread
    /// has exited or `timeout` elapses. Other events that arrive in the
    /// meantime are drained and discarded — the caller is shutting
    /// down anyway.
    ///
    /// Call this before dropping `AudioEngine` (or before closing the
    /// app window) so a hardware synth doesn't sustain notes that were
    /// playing at quit time. `Drop` calls this with a short timeout if
    /// the user didn't.
    pub fn shutdown(&mut self, timeout: std::time::Duration) {
        let _ = self.cmd_tx.send(AudioCommand::Stop);
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let now = std::time::Instant::now();
            if now >= deadline {
                break;
            }
            match self.event_rx.recv_timeout(deadline - now) {
                Ok(AudioEvent::Stopped) => break,
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        let _ = self.cmd_tx.send(AudioCommand::ShutDown);
        if let Some(handle) = self.engine_thread.take() {
            // Spawn a watchdog thread to enforce the deadline since
            // std::thread::JoinHandle has no timed join. The handle
            // itself is moved into the watchdog so this function
            // returns promptly even if the engine thread is wedged.
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let watchdog = std::thread::spawn(move || {
                let _ = handle.join();
            });
            // Best effort: poll the watchdog until the deadline.
            let poll_until = std::time::Instant::now() + remaining;
            while !watchdog.is_finished() && std::time::Instant::now() < poll_until {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }

    /// Try to receive an event from the audio engine (non-blocking).
    pub fn try_recv(&self) -> Option<AudioEvent> {
        self.event_rx.try_recv().ok()
    }

    /// Test-only constructor that builds an `AudioEngine` with no spawned
    /// engine thread, no cpal stream, and a command channel whose receiver
    /// has already been dropped. Calling [`AudioEngine::send`] on the
    /// returned handle therefore always exercises the disconnect branch
    /// and returns `Err(EngineSendError)`.
    ///
    /// Exposed via `__test_support` so the disconnect regression test in
    /// `tests/` can run without bringing up a real audio device.
    #[doc(hidden)]
    pub fn for_test_disconnected() -> Self {
        // Build a command channel and immediately drop the receiver so
        // every send hits `SendError`.
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<AudioCommand>();
        drop(cmd_rx);
        let (_event_tx, event_rx) = crossbeam_channel::unbounded::<AudioEvent>();

        let shared = Arc::new(SharedState::default());

        // A zero-capacity ringbuf is fine — the test never drives audio
        // through it.
        let monitor_ring = ringbuf::HeapRb::<f32>::new(1);
        let (prod, _cons) = monitor_ring.split();

        Self {
            cmd_tx,
            event_rx,
            _stream: None,
            engine_thread: None,
            shared,
            tracks: Arc::new(parking_lot::RwLock::new(IndexMap::new())),
            busses: Arc::new(parking_lot::RwLock::new(IndexMap::new())),
            master: Arc::new(parking_lot::RwLock::new(MasterBus::new())),
            clips: Arc::new(parking_lot::RwLock::new(Vec::new())),
            midi_clips: Arc::new(parking_lot::RwLock::new(Vec::new())),
            plugins: Arc::new(parking_lot::RwLock::new(IndexMap::new())),
            tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
            monitor_prod: Arc::new(parking_lot::Mutex::new(prod)),
            sample_rate: 48_000,
            channels: 2,
            quantum: 128,
        }
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        // If `shutdown` was already called the JoinHandle is `None` and
        // this is a no-op. Otherwise send `ShutDown` and let the thread
        // exit; the cpal stream drops afterward as the struct unwinds.
        if self.engine_thread.is_some() {
            self.shutdown(std::time::Duration::from_millis(500));
        }
    }
}
