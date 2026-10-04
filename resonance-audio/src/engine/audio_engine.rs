//! The engine handle. [`AudioEngine`] is the one surface the app sees:
//! commands in (`send`), events out (`try_recv`), lifecycle (`new` /
//! `with_options` / `shutdown` / `Drop`). Start-up runs in steps — probe
//! the output device, open the channels and shared snapshots, build the
//! mix callback, open the output backend, spawn the control thread — each
//! its own function below `impl AudioEngine`. Split out of `engine/mod.rs`
//! so that file re-exports and nothing else (ARCH2-12).

use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use ringbuf::traits::Split;
use thiserror::Error;

use crate::midi_clock::MidiClockEvent;
use crate::midi_hardware::{LiveControlEvent, LiveMidiEvent};
use crate::mixer;
use crate::platform::{self, DeviceDirection};
use crate::types::*;

use super::{reference, thread, AutomationSnapshot, SharedState, MAX_BUSSES};

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

/// One-shot error log when the command channel first disconnects, as a
/// safety net for call sites that intentionally `let _ =` the
/// `EngineSendError` returned by `AudioEngine::send`. Uses an atomic
/// latch so a stuck app doesn't flood stderr. Lives at module scope
/// so the test-only `for_test_disconnected` path can reset it (see
/// `test_support::__reset_engine_disconnect_latch_for_test`).
fn report_engine_disconnect_once() {
    use std::sync::atomic::Ordering;
    if !ENGINE_DISCONNECT_REPORTED.swap(true, Ordering::Relaxed) {
        tracing::error!(
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
pub struct AudioEngine {
    cmd_tx: Sender<AudioCommand>,
    event_rx: Receiver<AudioEvent>,
    /// The engine's shared atomics, kept on the handle so app-facing
    /// accessors ([`AudioEngine::output_stream_lost`]) can read state
    /// the output-backend callbacks publish without a command round
    /// trip through an engine thread that may be busy.
    shared: Arc<SharedState>,
    _stream: Option<cpal::Stream>,
    /// Native PipeWire output stream (the default backend). `None` when
    /// running on the cpal fallback (`_stream` is `Some` then) or in
    /// test constructors. Never read (its accessor,
    /// [`output_pipewire::PipeWireOutputHandle::with_stream`], has no
    /// caller yet — doc #260 finding #13's follow-up latency/time-info
    /// work hasn't landed), but it must stay a field: its `Drop` stops
    /// the PipeWire realtime thread and tears down the stream/listener/
    /// core, so dropping it early (e.g. at the end of `new()`, where the
    /// local `pw_output` binding would otherwise fall out of scope)
    /// would silence output the moment construction finished.
    #[cfg(target_os = "linux")]
    #[allow(dead_code)]
    pw_output: Option<crate::output_pipewire::PipeWireOutputHandle>,
    /// Join handle for the engine control thread. `Drop` sends a
    /// `ShutDown` command (which breaks the thread's loop, since the
    /// thread's own `cmd_tx_retry` keeps the channel from ever
    /// returning `Disconnected`) and then joins.
    engine_thread: Option<std::thread::JoinHandle<()>>,
    /// Holds the PipeWire graph at the engine's rate for the lifetime
    /// of the engine (`None` when the force was rejected and we follow
    /// the graph instead). Declared last so its `Drop` — which hands
    /// the graph back by restoring the `clock.force-rate` we found —
    /// runs after the output stream has been torn down.
    _graph_force: Option<platform::GraphRateForce>,
}

/// Failure starting the audio engine ([`AudioEngine::new`]): probing the
/// output device, building the cpal stream (with the buffer-size
/// fallback), starting it, or spawning the engine control thread.
/// Message text matches the historical `format!()` / literal strings.
#[derive(Debug, Error)]
pub enum EngineInitError {
    #[error("No audio output device found")]
    NoOutputDevice,
    #[error("Failed to get default output config: {0}")]
    DefaultConfig(#[source] cpal::DefaultStreamConfigError),
    #[error("Failed to build output stream: {0}")]
    BuildStream(#[source] cpal::BuildStreamError),
    #[error("Failed to start stream: {0}")]
    PlayStream(#[source] cpal::PlayStreamError),
    #[error("Failed to spawn engine thread: {0}")]
    SpawnThread(#[source] std::io::Error),
}

impl From<EngineInitError> for EngineError {
    fn from(e: EngineInitError) -> Self {
        let kind = match &e {
            EngineInitError::NoOutputDevice => EngineErrorKind::NotFound,
            EngineInitError::DefaultConfig(_)
            | EngineInitError::BuildStream(_)
            | EngineInitError::PlayStream(_)
            | EngineInitError::SpawnThread(_) => EngineErrorKind::Io,
        };
        EngineError::new(kind, e.to_string())
    }
}

/// Startup options for [`AudioEngine::with_options`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineOptions {
    /// Threads the live render pool spreads track jobs over, the audio
    /// thread included (realtime-multithreading.md §4.3); 1 renders
    /// serially. `None` = physical cores − 1 workers plus the audio
    /// thread. The `RESONANCE_RENDER_THREADS` environment variable
    /// overrides it.
    pub render_threads: Option<usize>,
}

impl AudioEngine {
    /// Create and start the audio engine with default options. Returns
    /// the engine handle.
    pub fn new() -> Result<Self, EngineInitError> {
        Self::with_options(EngineOptions::default())
    }

    /// Create and start the audio engine. Returns the engine handle.
    ///
    /// In order: probe the default output device and the graph's rate and
    /// quantum ([`probe_output`]), open the command / event / live-MIDI
    /// channels ([`EngineChannels::open`]) and the wait-free snapshots the
    /// two threads share ([`Snapshots::new`]), size the render pool, open
    /// the output backend with a mix callback built for it
    /// ([`open_output_backend`]), start the stream, then spawn the engine
    /// control thread ([`spawn_engine_thread`]).
    pub fn with_options(options: EngineOptions) -> Result<Self, EngineInitError> {
        let probe = probe_output()?;
        let channels = probe.config.channels() as usize;
        let ch = EngineChannels::open();
        let snapshots = Snapshots::new();

        let mut stream_config: cpal::StreamConfig = probe.config.clone().into();
        stream_config.sample_rate = probe.sample_rate;
        stream_config.buffer_size = cpal::BufferSize::Fixed(probe.quantum as cpal::FrameCount);

        // The render pool's size, resolved once: the live callback's pool
        // and every offline render's use the same thread count.
        let live_pool_config =
            crate::render_pool::PoolConfig::live(probe.buf_frames, options.render_threads);
        crate::render_pool::configure_threads(live_pool_config.workers + 1);
        let mixer = MixerFactory {
            snapshots: &snapshots,
            live_midi_rx: &ch.live_midi_rx,
            live_midi_fwd_tx: &ch.live_midi_fwd_tx,
            sample_rate: probe.sample_rate,
            buf_frames: probe.buf_frames,
            quantum: probe.quantum,
            pool_config: live_pool_config,
        };

        let backend =
            open_output_backend(&probe, &stream_config, channels, &snapshots.shared, &mixer)?;

        // One-line negotiation summary so latency regressions are diagnosable
        // from stderr alone. `probed_*` being None means the pw-metadata
        // subprocess failed and we're running on the conservative fallback
        // numbers, which is usually the cause of "why is latency higher than
        // the pipewire quantum".
        tracing::info!(
            "audio: backend={} device={:?} sample_rate={} (cpal_default={}, graph_forced={:?}) quantum={} (probed={:?}) max_quantum={} (probed={:?}) buf_frames={} fixed_buffer={}",
            backend.name,
            probe.device_name,
            probe.sample_rate,
            probe.default_rate,
            probe.graph_force.as_ref().map(|f| f.rate()),
            probe.quantum,
            probe.probed_quantum,
            probe.max_quantum,
            probe.probed_max_quantum,
            probe.buf_frames,
            backend.used_fixed_buffer,
        );

        let monitor_prod_audio = Arc::new(parking_lot::Mutex::new(backend.monitor_prod));

        if let Some(stream) = &backend.stream {
            stream.play().map_err(EngineInitError::PlayStream)?;
        }

        // Spawn the engine control thread
        let cmd_tx_retry = ch.cmd_tx.clone();
        let engine_thread = spawn_engine_thread(thread::EngineThreadParams {
            cmd_rx: ch.cmd_rx,
            cmd_tx_retry,
            event_tx: ch.event_tx,
            shared: Arc::clone(&snapshots.shared),
            tempo_map: Arc::clone(&snapshots.tempo_map),
            latency_comp: Arc::clone(&snapshots.latency_comp),
            automation: Arc::clone(&snapshots.automation),
            monitor_prod: monitor_prod_audio,
            live_midi_tx: ch.live_midi_tx,
            live_midi_fwd_rx: ch.live_midi_fwd_rx,
            live_control_tx: ch.live_control_tx,
            live_control_rx: ch.live_control_rx,
            clock_tx: ch.clock_tx,
            clock_rx: ch.clock_rx,
            sample_rate: probe.sample_rate,
            buf_frames: probe.buf_frames,
            quantum: probe.quantum,
        })?;

        Ok(Self {
            cmd_tx: ch.cmd_tx,
            event_rx: ch.event_rx,
            shared: snapshots.shared,
            _stream: backend.stream,
            #[cfg(target_os = "linux")]
            pw_output: backend.pw_output,
            engine_thread: Some(engine_thread),
            _graph_force: probe.graph_force,
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

    /// Whether a `send` has ever observed the engine thread's command
    /// channel disconnected. Reads the same one-shot latch
    /// [`report_engine_disconnect_once`] sets, so it goes true the first
    /// time any `send` call anywhere hits the disconnect branch — not
    /// just one made through this handle.
    ///
    /// A disconnected channel never reconnects (the engine thread is
    /// gone for good — post-shutdown or a panic), so this is safe to
    /// poll from a long-lived caller like the app's tick handler to
    /// latch a one-time "engine stopped responding" banner instead of
    /// matching on every individual `send`'s `Result`.
    pub fn is_disconnected(&self) -> bool {
        ENGINE_DISCONNECT_REPORTED.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Whether the *output stream* is currently lost while the engine
    /// thread is still alive — the sink vanished (USB interface
    /// unplugged) or the audio server restarted, so nothing is audible
    /// and recording captures nothing even though commands still ack.
    ///
    /// Distinct from [`AudioEngine::is_disconnected`], which only
    /// reports the engine *thread* being gone. Published by the output
    /// backends' error callbacks (`output_pipewire::on_state_changed`
    /// and the cpal `err_fn`); per-engine rather than process-global,
    /// and — unlike the disconnect latch — it clears again when the
    /// PipeWire backend observes the stream come back, so poll it every
    /// tick rather than latching the first `true`.
    pub fn output_stream_lost(&self) -> bool {
        self.shared
            .output_stream_lost
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The realtime mix callback's load as `(smoothed, window_peak)`
    /// fractions of the cycle budget, as [`crate::cycle_load`] publishes
    /// them every mix call — for the app's CPU readout. `None` until a
    /// mix call has published one (no audio callback has run yet).
    pub fn dsp_load(&self) -> Option<(f32, f32)> {
        use std::sync::atomic::Ordering::Relaxed;
        let smoothed = self.shared.dsp_load_ema_bits.load(Relaxed);
        let peak = self.shared.dsp_load_peak_bits.load(Relaxed);
        if smoothed == 0 && peak == 0 {
            return None;
        }
        Some((f32::from_bits(smoothed), f32::from_bits(peak)))
    }

    /// Test-only hook: publish a DSP load as the mix callback would, so
    /// app tests can drive the CPU readout without an audio stream.
    #[doc(hidden)]
    pub fn __set_dsp_load_for_test(&self, smoothed: f32, peak: f32) {
        use std::sync::atomic::Ordering::Relaxed;
        self.shared.dsp_load_ema_bits.store(smoothed.to_bits(), Relaxed);
        self.shared.dsp_load_peak_bits.store(peak.to_bits(), Relaxed);
    }

    /// Test-only hook: force the output-stream-lost flag so app tests
    /// can drive the banner logic without a real backend callback.
    #[doc(hidden)]
    pub fn __set_output_stream_lost_for_test(&self, lost: bool) {
        self.shared
            .output_stream_lost
            .store(lost, std::sync::atomic::Ordering::Relaxed);
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

    /// Shared body for the test-only constructors below: every field that
    /// doesn't vary between them (no real device, no engine thread, an
    /// empty project, a throwaway single-slot monitor ring). Callers pass
    /// in just the pieces that differ — the command channel ends and the
    /// event receiver — so a field added to `AudioEngine` needs an
    /// initializer here plus one in [`AudioEngine::new`], not one in each
    /// test constructor too.
    fn for_test_with(cmd_tx: Sender<AudioCommand>, event_rx: Receiver<AudioEvent>) -> Self {
        Self {
            cmd_tx,
            event_rx,
            shared: Arc::new(SharedState::default()),
            _stream: None,
            #[cfg(target_os = "linux")]
            pw_output: None,
            engine_thread: None,
            _graph_force: None,
        }
    }

    /// Test-only constructor that builds an `AudioEngine` with no spawned
    /// engine thread, no cpal stream, and a command channel whose receiver
    /// has already been dropped. Calling [`AudioEngine::send`] on the
    /// returned handle therefore always exercises the disconnect branch
    /// and returns `Err(EngineSendError)`.
    ///
    /// Exposed via `test_support` so the disconnect regression test in
    /// `tests/` can run without bringing up a real audio device.
    #[doc(hidden)]
    pub fn for_test_disconnected() -> Self {
        // Build a command channel and immediately drop the receiver so
        // every send hits `SendError`.
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<AudioCommand>();
        drop(cmd_rx);
        let (_event_tx, event_rx) = crossbeam_channel::unbounded::<AudioEvent>();

        Self::for_test_with(cmd_tx, event_rx)
    }

    /// Test-only constructor that builds an `AudioEngine` with no spawned
    /// engine thread and no cpal stream, but whose command channel's
    /// receiver is handed back to the caller. Commands sent via
    /// [`AudioEngine::send`] therefore queue on the returned `Receiver`
    /// instead of being processed, so a test can assert *exactly* which
    /// commands an update handler emitted — without bringing up a real
    /// audio device or racing an engine thread.
    ///
    /// The engine never processes the queued commands, so it emits no
    /// echo events: a test simulating a round trip feeds the resulting
    /// `AudioEvent`s back in itself (mirroring the live engine's echo).
    #[doc(hidden)]
    pub fn for_test_capture() -> (Self, Receiver<AudioCommand>) {
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<AudioCommand>();
        let (_event_tx, event_rx) = crossbeam_channel::unbounded::<AudioEvent>();

        let engine = Self::for_test_with(cmd_tx, event_rx);
        (engine, cmd_rx)
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

// ---------------------------------------------------------------------------
// Start-up steps of `AudioEngine::with_options`
// ---------------------------------------------------------------------------

/// What probing the default output device settled on: the device, the
/// rate the graph will run at, and the quantum the buffers are sized by.
struct OutputProbe {
    device: cpal::Device,
    device_name: String,
    config: cpal::SupportedStreamConfig,
    /// cpal's default rate, for the start-up log.
    default_rate: cpal::SampleRate,
    /// Holds the PipeWire graph at `sample_rate` for the engine's lifetime
    /// (`None` when the force was rejected and we follow the graph).
    graph_force: Option<platform::GraphRateForce>,
    sample_rate: u32,
    quantum: usize,
    max_quantum: usize,
    buf_frames: usize,
    probed_quantum: Option<u32>,
    probed_max_quantum: Option<u32>,
}

fn probe_output() -> Result<OutputProbe, EngineInitError> {
    // Replace ALSA's default stderr error handler before any cpal
    // / device enumeration so the startup PCM probing doesn't
    // spam "Cannot open device /dev/dsp" and friends. Idempotent.
    platform::silence_alsa_diagnostic_output();

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or(EngineInitError::NoOutputDevice)?;

    let device_name = device
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_else(|_| "<unnamed>".to_string());

    let config = device
        .default_output_config()
        .map_err(EngineInitError::DefaultConfig)?;

    let default_rate = config.sample_rate();

    // Assert the graph rate rather than follow it: another client
    // (e.g. a 44.1 kHz music stream on an idle graph) may have
    // dragged PipeWire off the project rate, and following it would
    // adopt that rate for the whole session. Force the preferred
    // rate (canonical 48 kHz when the device supports it) via the
    // settings metadata; when the environment rejects the force,
    // fall back to following the graph as before. cpal's
    // default_output_config often returns 44100 via ALSA compat
    // while the actual graph runs at a different rate — following
    // that blindly makes PipeWire resample every buffer and inflate
    // the quantum (e.g. 1102 frames instead of 128).
    let graph_force = platform::assert_graph_rate(&device, DeviceDirection::Output);
    let sample_rate = match &graph_force {
        Some(force) => force.rate(),
        None => platform::pick_sample_rate(&device, &config, DeviceDirection::Output),
    };

    // Query PipeWire quantum to size buffers relative to the actual period.
    let probed_quantum = platform::pipewire_quantum();
    let probed_max_quantum = platform::pipewire_max_quantum();
    let quantum = probed_quantum.unwrap_or(1024) as usize;
    let max_quantum = probed_max_quantum.unwrap_or(2048) as usize;
    let buf_frames = max_quantum.max(quantum * 2).max(256);

    Ok(OutputProbe {
        device,
        device_name,
        config,
        default_rate,
        graph_force,
        sample_rate,
        quantum,
        max_quantum,
        buf_frames,
        probed_quantum,
        probed_max_quantum,
    })
}

/// The channels between the app, the engine control thread and the
/// audio callback.
struct EngineChannels {
    cmd_tx: Sender<AudioCommand>,
    cmd_rx: Receiver<AudioCommand>,
    event_tx: Sender<AudioEvent>,
    event_rx: Receiver<AudioEvent>,
    live_midi_tx: Sender<LiveMidiEvent>,
    live_midi_rx: Receiver<LiveMidiEvent>,
    live_midi_fwd_tx: Sender<LiveMidiEvent>,
    live_midi_fwd_rx: Receiver<LiveMidiEvent>,
    live_control_tx: Sender<LiveControlEvent>,
    live_control_rx: Receiver<LiveControlEvent>,
    clock_tx: Sender<MidiClockEvent>,
    clock_rx: Receiver<MidiClockEvent>,
}

impl EngineChannels {
    fn open() -> Self {
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<AudioCommand>();
        let (event_tx, event_rx) = crossbeam_channel::unbounded::<AudioEvent>();
        // Bounded so a stuck engine thread can never let hardware
        // MIDI events queue without bound. 1024 fits a comfortable
        // burst at typical engine-thread cadence (~60 Hz wakeups).
        let (live_midi_tx, live_midi_rx) = crossbeam_channel::bounded::<LiveMidiEvent>(1024);
        // Audio-thread live-MIDI pickup (doc #260 finding #16): the mix
        // callback drains `live_midi_rx` (instrument delivery within one
        // quantum) and forwards each event on this channel for the
        // engine thread's recording / MIDI-thru bookkeeping.
        let (live_midi_fwd_tx, live_midi_fwd_rx) =
            crossbeam_channel::bounded::<LiveMidiEvent>(1024);
        // Separate channel for the dedicated control-surface input. Same
        // bound + rationale as the per-track live MIDI channel above.
        let (live_control_tx, live_control_rx) =
            crossbeam_channel::bounded::<LiveControlEvent>(1024);
        // MIDI clock arrives at 24 PPQN (≈48 msgs/sec at 120 BPM)
        // plus Start/Stop/Continue. 4096 covers seconds of bursty
        // input even if the engine thread stalls.
        let (clock_tx, clock_rx) = crossbeam_channel::bounded::<MidiClockEvent>(4096);
        Self {
            cmd_tx,
            cmd_rx,
            event_tx,
            event_rx,
            live_midi_tx,
            live_midi_rx,
            live_midi_fwd_tx,
            live_midi_fwd_rx,
            live_control_tx,
            live_control_rx,
            clock_tx,
            clock_rx,
        }
    }
}

/// The wait-free snapshots the engine thread publishes and the audio
/// callback (and the offline renders) load.
struct Snapshots {
    shared: Arc<SharedState>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    /// Plugin-delay-compensation table: published by the engine thread
    /// on topology changes, loaded wait-free by the audio callback. See
    /// `crate::latency` for the compensation model.
    latency_comp: Arc<arc_swap::ArcSwap<crate::latency::LatencyComp>>,
    /// Parameter-automation snapshot: published by the engine thread
    /// whenever the lane set changes, loaded wait-free by the audio
    /// callback and the offline bounce. Empty until the first lane is
    /// stored. See `engine::automation` for the data model.
    automation: Arc<arc_swap::ArcSwap<AutomationSnapshot>>,
}

impl Snapshots {
    fn new() -> Self {
        Self {
            shared: Arc::new(SharedState::default()),
            tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
            latency_comp: Arc::new(arc_swap::ArcSwap::from_pointee(
                crate::latency::LatencyComp::empty(),
            )),
            automation: Arc::new(arc_swap::ArcSwap::from_pointee(AutomationSnapshot::default())),
        }
    }
}

/// Builds one fully-captured mixer callback plus the matching monitor-ring
/// producer. Callable more than once (the native PipeWire attempt, then
/// the cpal fallback) — each call allocates a fresh scratch set and the
/// losing attempt's set is simply dropped with its backend.
struct MixerFactory<'a> {
    snapshots: &'a Snapshots,
    live_midi_rx: &'a Receiver<LiveMidiEvent>,
    live_midi_fwd_tx: &'a Sender<LiveMidiEvent>,
    sample_rate: u32,
    buf_frames: usize,
    quantum: usize,
    pool_config: crate::render_pool::PoolConfig,
}

impl MixerFactory<'_> {
    fn make(&self, native_backend: bool) -> (mixer::MixFn, ringbuf::HeapProd<f32>) {
        // Clone captures that the closure needs to own
        let shared_audio = Arc::clone(&self.snapshots.shared);
        let tempo_audio = Arc::clone(&self.snapshots.tempo_map);
        let latency_comp_audio = Arc::clone(&self.snapshots.latency_comp);
        let automation_audio = Arc::clone(&self.snapshots.automation);
        let audio_sample_rate = self.sample_rate;
        let audio_buf_frames = self.buf_frames;
        let audio_quantum = self.quantum;
        let mut track_buf_l = vec![0.0f32; audio_buf_frames];
        let mut track_buf_r = vec![0.0f32; audio_buf_frames];
        // One render slot per track (the track jobs' output, reduced in
        // track order), grown by the engine thread on graph publish.
        let mut track_slots = shared_audio.graph.attach_live_slots(audio_buf_frames);
        // The render worker pool the track jobs spread over
        // (realtime-multithreading.md §4.3). Owned by this closure and
        // joined when the stream drops it; the engine loop reports its
        // status.
        let render_pool = crate::render_pool::RenderPool::new(self.pool_config.clone());
        *shared_audio.render_pool.lock() = Some(render_pool.monitor());
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
        // Sidechain key capture, pre-allocated for the widest block
        // the callback can hand us so the realtime path never grows it.
        let mut sidechain = crate::types::SidechainTaps::new(audio_buf_frames);
        // Dry staging for the bypass crossfades, sized for the widest
        // block so toggling a bypass never allocates in the callback.
        let mut fx_dry = crate::bypass::FxDryScratch::new(audio_buf_frames);
        // Transport continuity for the voice flush on a playhead jump
        // (code review MIX-06).
        let mut continuity = mixer::TransportContinuity::default();
        // Native backend: the monitor ring may adaptively drain its
        // sticky startup backlog (same graph clock); the cpal
        // fallback keeps the standing margin (doc #260 finding #12).
        let mut monitor_drain = mixer::MonitorDrain::new(native_backend);
        let live_midi_rx = self.live_midi_rx.clone();
        let live_midi_fwd = self.live_midi_fwd_tx.clone();

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
        // Per-cycle DSP load meter (see `cycle_load`): quiet by
        // default (reports only over-budget cycles / monitor
        // shortfalls / lock misses / near-budget peaks), verbose
        // with RESONANCE_AUDIO_STATS=1. The report is published
        // into `SharedState::cycle_report`; the engine loop prints
        // it, so this closure never formats or writes to stderr.
        let mut load_meter = crate::cycle_load::CycleLoadMeter::new(
            std::env::var_os("RESONANCE_AUDIO_STATS").is_some(),
        );
        let mix: mixer::MixFn = Box::new(move |data: &mut [f32], channels: usize| {
            let mix_start = std::time::Instant::now();
            mixer::mix_audio(
                mixer::CallbackInputs {
                    channels,
                    shared: &shared_audio,
                    tempo_map: &tempo_audio,
                    latency_comp: &latency_comp_audio,
                    automation: &automation_audio,
                    sample_rate: audio_sample_rate,
                    live_midi_rx: &live_midi_rx,
                    live_midi_fwd: &live_midi_fwd,
                    buf_frames: audio_buf_frames,
                    quantum: audio_quantum,
                },
                &mut mixer::CallbackScratch {
                    data,
                    track_buf_l: &mut track_buf_l,
                    track_buf_r: &mut track_buf_r,
                    bus_bufs: &mut bus_bufs,
                    port_scratch: &mut port_scratch,
                    note_event_buf: &mut note_event_buf,
                    midi_stash: &mut midi_stash,
                    monitor_cons: &mut monitor_cons,
                    monitor_temp: &mut monitor_temp,
                    monitor_drain: &mut monitor_drain,
                    ab_meters: &mut ab_meters,
                    sidechain: &mut sidechain,
                    track_slots: &mut track_slots,
                    pool: &render_pool,
                    fx_dry: &mut fx_dry,
                    continuity: &mut continuity,
                },
            );
            let mix_end = std::time::Instant::now();
            load_meter.record_pass(&track_slots.current().take_stats());
            if let Some(report) = load_meter.record(
                mix_end,
                mix_end - mix_start,
                data.len() / channels.max(1),
                audio_sample_rate,
                &shared_audio,
            ) {
                shared_audio.cycle_report.publish(&report);
            }
        });
        (mix, prod)
    }
}

/// cpal fallback: the historical output path through the pipewire-alsa
/// shim. Kept for non-PipeWire hosts (and the RESONANCE_FORCE_CPAL_OUTPUT
/// escape hatch); its ALSA ring adds 2+ periods of extra latency over the
/// native stream.
fn build_cpal_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    shared: &Arc<SharedState>,
    mixer: &MixerFactory<'_>,
) -> Result<(cpal::Stream, ringbuf::HeapProd<f32>), cpal::BuildStreamError> {
    let (mut mix, prod) = mixer.make(false);
    let shared_err = Arc::clone(shared);
    // cpal spawns the callback thread itself, so it cannot claim
    // its arc-swap node before its first block: leave free ones
    // for it to take (`rt_prep`).
    crate::rt_prep::seed_arc_swap_nodes(2);
    let result = device.build_output_stream(
        config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| mix(data, channels),
        // Runs on cpal's ALSA worker, the audio thread: atomics
        // only; the engine loop rate-limits underruns and logs
        // (code review FU-H6b).
        move |err| {
            if !matches!(err, cpal::StreamError::BufferUnderrun) {
                // DeviceNotAvailable / StreamInvalidated / backend
                // errors: the stream is dead but the engine thread
                // is not, so without this flag the failure is
                // invisible to the app (the engine-death banner
                // keys off the command channel, which is fine).
                // cpal delivers no "recovered" callback, so the
                // flag stays set until the app restarts.
                shared_err
                    .output_stream_lost
                    .store(true, std::sync::atomic::Ordering::Release);
            }
            shared_err.output_stream_errors.record(&err);
        },
        None,
    );
    result.map(|stream| (stream, prod))
}

/// [`build_cpal_stream`] at the fixed quantum, falling back to
/// `BufferSize::Default` when the device rejects it. The `bool` says
/// whether the fixed size was honoured.
fn build_cpal_with_fallback(
    device: &cpal::Device,
    stream_config: &cpal::StreamConfig,
    quantum: usize,
    channels: usize,
    shared: &Arc<SharedState>,
    mixer: &MixerFactory<'_>,
) -> Result<(cpal::Stream, ringbuf::HeapProd<f32>, bool), EngineInitError> {
    match build_cpal_stream(device, stream_config, channels, shared, mixer) {
        Ok((stream, prod)) => Ok((stream, prod, true)),
        Err(fixed_err) => {
            // Fall back to default buffer size if fixed quantum was rejected.
            let mut fallback_config = stream_config.clone();
            fallback_config.buffer_size = cpal::BufferSize::Default;
            match build_cpal_stream(device, &fallback_config, channels, shared, mixer) {
                Ok((stream, prod)) => {
                    tracing::warn!(
                        "audio: Fixed({}) rejected ({}) — falling back to BufferSize::Default (HIGH LATENCY)",
                        quantum, fixed_err
                    );
                    Ok((stream, prod, false))
                }
                Err(e) => Err(EngineInitError::BuildStream(e)),
            }
        }
    }
}

/// The opened output backend: the cpal stream (or none, on native
/// PipeWire), the monitor-ring producer the mix callback drains, and the
/// facts the start-up log reports.
struct OutputBackend {
    stream: Option<cpal::Stream>,
    monitor_prod: ringbuf::HeapProd<f32>,
    used_fixed_buffer: bool,
    #[cfg(target_os = "linux")]
    pw_output: Option<crate::output_pipewire::PipeWireOutputHandle>,
    name: &'static str,
}

/// Output backend selection: native PipeWire stream first — it renders
/// straight into the graph cycle (~1 quantum of output buffering) and
/// exposes real latency/time info — with cpal as the explicit fallback
/// for non-PipeWire hosts (doc #260 finding #11). PipeWire only exists on
/// Linux; everywhere else the cpal path *is* the output backend.
#[cfg(target_os = "linux")]
fn open_output_backend(
    probe: &OutputProbe,
    stream_config: &cpal::StreamConfig,
    channels: usize,
    shared: &Arc<SharedState>,
    mixer: &MixerFactory<'_>,
) -> Result<OutputBackend, EngineInitError> {
    let force_cpal = std::env::var_os("RESONANCE_FORCE_CPAL_OUTPUT").is_some();
    let cpal_fallback = || -> Result<OutputBackend, EngineInitError> {
        let (s, p, fixed) = build_cpal_with_fallback(
            &probe.device,
            stream_config,
            probe.quantum,
            channels,
            shared,
            mixer,
        )?;
        Ok(OutputBackend {
            stream: Some(s),
            monitor_prod: p,
            used_fixed_buffer: fixed,
            pw_output: None,
            name: "cpal",
        })
    };
    if force_cpal {
        tracing::info!("audio: RESONANCE_FORCE_CPAL_OUTPUT set — skipping native PipeWire output");
        return cpal_fallback();
    }
    let (mix, prod) = mixer.make(true);
    match crate::output_pipewire::build(
        None,
        Arc::clone(shared),
        probe.sample_rate,
        2,
        probe.quantum as u32,
        probe.buf_frames,
        mix,
    ) {
        Ok((handle, pw_rate, pw_channels)) => {
            tracing::info!(
                "audio: native PipeWire output up: rate={} channels={} latency_vote={}/{}",
                pw_rate, pw_channels, probe.quantum, probe.sample_rate
            );
            Ok(OutputBackend {
                stream: None,
                monitor_prod: prod,
                used_fixed_buffer: true,
                pw_output: Some(handle),
                name: "pipewire",
            })
        }
        Err(e) => {
            tracing::warn!(
                "audio: native PipeWire output unavailable ({e}) — falling back to cpal (ALSA shim, higher latency)"
            );
            cpal_fallback()
        }
    }
}

/// See the Linux version: here the cpal path is the only output backend.
#[cfg(not(target_os = "linux"))]
fn open_output_backend(
    probe: &OutputProbe,
    stream_config: &cpal::StreamConfig,
    channels: usize,
    shared: &Arc<SharedState>,
    mixer: &MixerFactory<'_>,
) -> Result<OutputBackend, EngineInitError> {
    if std::env::var_os("RESONANCE_FORCE_CPAL_OUTPUT").is_some() {
        tracing::info!(
            "audio: RESONANCE_FORCE_CPAL_OUTPUT set — cpal is already the only output backend on this platform"
        );
    }
    let (s, p, fixed) = build_cpal_with_fallback(
        &probe.device,
        stream_config,
        probe.quantum,
        channels,
        shared,
        mixer,
    )?;
    Ok(OutputBackend {
        stream: Some(s),
        monitor_prod: p,
        used_fixed_buffer: fixed,
        name: "cpal",
    })
}

/// Spawn the engine control thread.
fn spawn_engine_thread(
    params: thread::EngineThreadParams,
) -> Result<std::thread::JoinHandle<()>, EngineInitError> {
    std::thread::Builder::new()
        .name("resonance-engine".into())
        .spawn(move || thread::engine_thread(params))
        .map_err(EngineInitError::SpawnThread)
}
