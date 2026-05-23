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
use crate::midi_hardware::LiveMidiEvent;
use crate::mixer;
use crate::platform::{self, DeviceDirection};
use crate::stream_errors::{format_underrun_line, UnderrunRateLimiter};
use crate::types::*;

mod bounce;
pub use bounce::try_lock_with_backoff;
mod bounce_common;

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

mod bounce_realtime;
mod busses;
mod clips;
pub use clips::transcode_to_wav;
mod master;
pub(crate) mod midi;
mod plugins;
mod scan;
mod thread;
mod tracks;
mod transport;

/// Shared state between the engine control thread and the audio callback.
pub(crate) struct SharedState {
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
    /// directly — but stays in shared state so the offline renderer
    /// running on the engine thread can be aborted from the same
    /// `CancelBounce` command without threading another channel.
    pub bounce_cancel: AtomicBool,
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
        // MIDI clock arrives at 24 PPQN (≈48 msgs/sec at 120 BPM)
        // plus Start/Stop/Continue. 4096 covers seconds of bursty
        // input even if the engine thread stalls.
        let (clock_tx, clock_rx) = crossbeam_channel::bounded::<MidiClockEvent>(4096);

        let shared = Arc::new(SharedState {
            playhead: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            recording: AtomicBool::new(false),
            monitoring: AtomicBool::new(false),
            master_volume_bits: AtomicU32::new(1.0f32.to_bits()),
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
        });

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
            // Monitor scratch + ring are sized for the widest multi-channel
            // interleaved input we're likely to see (e.g. an 18-in audio
            // interface). 32 channels × a few blocks of headroom covers
            // everything reasonable without leaking meaningful RAM.
            use crate::limits::MAX_INPUT_CHANNELS;
            let mut monitor_temp = vec![0.0f32; audio_buf_frames * MAX_INPUT_CHANNELS];
            let monitor_ring = ringbuf::HeapRb::<f32>::new(audio_quantum * MAX_INPUT_CHANNELS * 4);
            let (prod, mut monitor_cons) = monitor_ring.split();
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
                        audio_sample_rate,
                        &mut track_buf_l,
                        &mut track_buf_r,
                        &mut bus_bufs,
                        &mut port_scratch,
                        &mut note_event_buf,
                        &mut monitor_cons,
                        &mut monitor_temp,
                        audio_buf_frames,
                        audio_quantum,
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
                    monitor_prod_audio,
                    live_midi_tx,
                    live_midi_rx,
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

    /// Send a command to the audio engine. Silently drops the command
    /// if the engine thread has disconnected (post-shutdown or panic);
    /// the first time that happens, an eprintln warning is emitted so
    /// the situation isn't completely invisible. Callers that need to
    /// observe disconnect should use `try_send`.
    pub fn send(&self, cmd: AudioCommand) {
        if let Err(e) = self.cmd_tx.send(cmd) {
            self.report_send_failure(&e.0);
        }
    }

    /// Send a command and surface a disconnect to the caller. Returned
    /// `Err` carries the original command so the caller can retry or
    /// surface a fatal error.
    pub fn try_send(&self, cmd: AudioCommand) -> Result<(), AudioCommand> {
        self.cmd_tx.send(cmd).map_err(|e| e.0)
    }

    /// Emit a one-shot eprintln when the command channel disconnects.
    /// Uses an atomic latch so a stuck app doesn't flood stderr.
    fn report_send_failure(&self, _cmd: &AudioCommand) {
        use std::sync::atomic::{AtomicBool, Ordering};
        static REPORTED: AtomicBool = AtomicBool::new(false);
        if !REPORTED.swap(true, Ordering::Relaxed) {
            eprintln!(
                "audio: engine command channel disconnected — subsequent commands will be dropped silently"
            );
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

    /// Get the command sender for cloning.
    pub fn command_sender(&self) -> Sender<AudioCommand> {
        self.cmd_tx.clone()
    }

    /// Get the event receiver for cloning.
    pub fn event_receiver(&self) -> Receiver<AudioEvent> {
        self.event_rx.clone()
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
