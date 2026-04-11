//! The core audio engine. `AudioEngine::new` wires up the cpal output
//! stream and spawns the engine control thread; the control thread's
//! command dispatch and per-concern handlers live in the submodules
//! (`thread`, `transport`, `tracks`, `clips`, `midi`, `plugins`,
//! `busses`, plus `scan` and `bounce`).

/// Ring buffer size for recording input: ~10 seconds at 96kHz stereo.
pub(crate) const RECORDING_RING_SIZE: usize = 96000 * 2 * 10;
/// Pre-allocation for recording buffers: ~60 seconds of stereo audio.
pub(crate) const RECORDING_PREALLOC_SECONDS: usize = 60;
/// Hard cap on concurrent busses. Used to pre-allocate bus summing
/// buffers at startup so the audio thread never has to allocate on a
/// bus add. 32 is well past what any realistic project needs.
pub(crate) const MAX_BUSSES: usize = 32;

use indexmap::IndexMap;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use ringbuf::traits::Split;

use crate::clap_host::SyncClapInstance;
use crate::mixer;
use crate::platform::{self, DeviceDirection};
use crate::types::*;

mod bounce;
mod busses;
mod clips;
mod midi;
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
}

/// The audio engine.
#[allow(dead_code)]
pub struct AudioEngine {
    cmd_tx: Sender<AudioCommand>,
    event_rx: Receiver<AudioEvent>,
    _stream: Option<cpal::Stream>,
    // Shared state for live stream rebuilding (e.g. buffer size changes)
    shared: Arc<SharedState>,
    tracks: Arc<parking_lot::RwLock<IndexMap<TrackId, Track>>>,
    busses: Arc<parking_lot::RwLock<IndexMap<BusId, Bus>>>,
    clips: Arc<parking_lot::RwLock<Vec<AudioClip>>>,
    midi_clips: Arc<parking_lot::RwLock<Vec<MidiClip>>>,
    plugins: Arc<parking_lot::RwLock<IndexMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>>>,
    tempo_map: Arc<parking_lot::RwLock<TempoMap>>,
    monitor_prod: Arc<parking_lot::Mutex<ringbuf::HeapProd<f32>>>,
    sample_rate: u32,
    channels: usize,
    quantum: usize,
}

// Safety: cpal::Stream is !Send on some platforms, but `_stream` is stored as
// `Option<cpal::Stream>` and is never accessed after construction — it is held
// solely to keep the stream alive via Drop. All other fields are Send.
unsafe impl Send for AudioEngine {}

impl AudioEngine {
    /// Create and start the audio engine. Returns the engine handle.
    pub fn new() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "No audio output device found".to_string())?;

        let config = device
            .default_output_config()
            .map_err(|e| format!("Failed to get default output config: {}", e))?;

        let channels = config.channels() as usize;

        // Prefer the PipeWire graph sample rate (typically 48000) to avoid resampling.
        // cpal's default_output_config often returns 44100 via ALSA compat, but the
        // actual hardware/graph runs at a different rate -- causing PipeWire to resample
        // every buffer and inflating the quantum (e.g. 1102 frames instead of 128).
        let sample_rate = platform::pick_sample_rate(&device, &config, DeviceDirection::Output);

        // Query PipeWire quantum to size buffers relative to the actual period.
        let quantum = platform::pipewire_quantum().unwrap_or(1024) as usize;
        let max_quantum = platform::pipewire_max_quantum().unwrap_or(2048) as usize;
        let buf_frames = max_quantum.max(quantum * 2).max(256);

        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<AudioCommand>();
        let (event_tx, event_rx) = crossbeam_channel::unbounded::<AudioEvent>();

        let shared = Arc::new(SharedState {
            playhead: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            recording: AtomicBool::new(false),
            monitoring: AtomicBool::new(false),
            master_volume_bits: AtomicU32::new(1.0f32.to_bits()),
            master_peak_l_bits: AtomicU32::new(0),
            master_peak_r_bits: AtomicU32::new(0),
            recording_overflow: AtomicBool::new(false),
            input_channels: AtomicU16::new(0),
            loop_enabled: AtomicBool::new(false),
            loop_in: AtomicU64::new(0),
            loop_out: AtomicU64::new(0),
        });

        let shared_audio = Arc::clone(&shared);

        let tracks: Arc<parking_lot::RwLock<IndexMap<TrackId, Track>>> =
            Arc::new(parking_lot::RwLock::new(IndexMap::new()));
        let busses: Arc<parking_lot::RwLock<IndexMap<BusId, Bus>>> =
            Arc::new(parking_lot::RwLock::new(IndexMap::new()));
        let clips: Arc<parking_lot::RwLock<Vec<AudioClip>>> =
            Arc::new(parking_lot::RwLock::new(Vec::new()));
        let midi_clips: Arc<parking_lot::RwLock<Vec<MidiClip>>> =
            Arc::new(parking_lot::RwLock::new(Vec::new()));

        let tracks_audio = Arc::clone(&tracks);
        let busses_audio = Arc::clone(&busses);
        let clips_audio = Arc::clone(&clips);
        let midi_clips_audio = Arc::clone(&midi_clips);

        let tempo_map: Arc<parking_lot::RwLock<TempoMap>> =
            Arc::new(parking_lot::RwLock::new(TempoMap::default()));
        let tempo_audio = Arc::clone(&tempo_map);

        // Plugin instances shared between engine thread and audio callback
        let plugins: Arc<parking_lot::RwLock<IndexMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>>> =
            Arc::new(parking_lot::RwLock::new(IndexMap::new()));
        let plugins_audio = Arc::clone(&plugins);

        let mut stream_config: cpal::StreamConfig = config.into();
        stream_config.sample_rate = cpal::SampleRate(sample_rate);
        stream_config.buffer_size = cpal::BufferSize::Fixed(quantum as cpal::FrameCount);
        let audio_sample_rate = sample_rate;

        let audio_buf_frames = buf_frames;
        let audio_quantum = quantum;
        let build_stream = |config: &cpal::StreamConfig| {
            // Clone captures that the closure needs to own
            let shared_audio = Arc::clone(&shared_audio);
            let tracks_audio = Arc::clone(&tracks_audio);
            let busses_audio = Arc::clone(&busses_audio);
            let clips_audio = Arc::clone(&clips_audio);
            let midi_clips_audio = Arc::clone(&midi_clips_audio);
            let plugins_audio = Arc::clone(&plugins_audio);
            let tempo_audio = Arc::clone(&tempo_audio);
            let mut track_buf_l = vec![0.0f32; audio_buf_frames];
            let mut track_buf_r = vec![0.0f32; audio_buf_frames];
            // Pre-allocate MAX_BUSSES stereo buffers so adding a bus at
            // runtime never allocates on the audio thread. mix_audio only
            // uses the first N slots where N = current bus count.
            let mut bus_bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..MAX_BUSSES)
                .map(|_| (vec![0.0f32; audio_buf_frames], vec![0.0f32; audio_buf_frames]))
                .collect();
            // Per-plugin-output-port scratch used for multi-output
            // instruments (resonance-drums declares 7 ports; this pool
            // carries room for a couple more).
            let mut port_scratch: Vec<(Vec<f32>, Vec<f32>)> =
                (0..crate::mixer::MAX_PLUGIN_OUTPUT_PORTS)
                    .map(|_| (vec![0.0f32; audio_buf_frames], vec![0.0f32; audio_buf_frames]))
                    .collect();
            let mut note_event_buf: Vec<PendingNoteEvent> = Vec::with_capacity(256);
            // Monitor scratch + ring are sized for the widest multi-channel
            // interleaved input we're likely to see (e.g. an 18-in audio
            // interface). 32 channels × a few blocks of headroom covers
            // everything reasonable without leaking meaningful RAM.
            const MAX_INPUT_CHANNELS: usize = 32;
            let mut monitor_temp =
                vec![0.0f32; audio_buf_frames * MAX_INPUT_CHANNELS];
            let monitor_ring =
                ringbuf::HeapRb::<f32>::new(audio_quantum * MAX_INPUT_CHANNELS * 4);
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
                |err| {
                    eprintln!("Audio stream error: {}", err);
                },
                None,
            );
            result.map(|stream| (stream, prod))
        };

        let (stream, monitor_prod_raw) = build_stream(&stream_config).or_else(|_| {
            // Fall back to default buffer size if fixed quantum was rejected
            let mut fallback_config = stream_config.clone();
            fallback_config.buffer_size = cpal::BufferSize::Default;
            build_stream(&fallback_config)
        }).map_err(|e| format!("Failed to build output stream: {}", e))?;

        let monitor_prod = Arc::new(parking_lot::Mutex::new(monitor_prod_raw));
        let monitor_prod_audio = Arc::clone(&monitor_prod);

        stream
            .play()
            .map_err(|e| format!("Failed to start stream: {}", e))?;

        // Spawn the engine control thread
        let shared_ctrl = Arc::clone(&shared);
        let tracks_ctrl = Arc::clone(&tracks);
        let busses_ctrl = Arc::clone(&busses);
        let clips_ctrl = Arc::clone(&clips);
        let midi_clips_ctrl = Arc::clone(&midi_clips);
        let tempo_ctrl = Arc::clone(&tempo_map);
        let plugins_ctrl = Arc::clone(&plugins);

        let cmd_tx_retry = cmd_tx.clone();
        std::thread::Builder::new()
            .name("resonance-engine".into())
            .spawn(move || {
                thread::engine_thread(
                    cmd_rx,
                    cmd_tx_retry,
                    event_tx,
                    shared_ctrl,
                    tracks_ctrl,
                    busses_ctrl,
                    clips_ctrl,
                    midi_clips_ctrl,
                    tempo_ctrl,
                    plugins_ctrl,
                    monitor_prod_audio,
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
            shared,
            tracks,
            busses,
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
    pub fn send(&self, cmd: AudioCommand) {
        let _ = self.cmd_tx.send(cmd);
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

    /// Read and clear peak levels for all tracks, busses, and master.
    /// Returns (track_peaks, bus_peaks, master_peak_l, master_peak_r).
    pub fn read_and_clear_peaks(
        &self,
    ) -> (Vec<(TrackId, f32, f32)>, Vec<(BusId, f32, f32)>, f32, f32) {
        let track_levels = if let Some(guard) = self.tracks.try_read() {
            guard
                .values()
                .map(|t| (t.id, t.swap_peak_l(), t.swap_peak_r()))
                .collect()
        } else {
            Vec::new()
        };
        let bus_levels = if let Some(guard) = self.busses.try_read() {
            guard
                .values()
                .map(|b| (b.id, b.swap_peak_l(), b.swap_peak_r()))
                .collect()
        } else {
            Vec::new()
        };
        let ml =
            f32::from_bits(self.shared.master_peak_l_bits.swap(0, Ordering::Relaxed));
        let mr =
            f32::from_bits(self.shared.master_peak_r_bits.swap(0, Ordering::Relaxed));
        (track_levels, bus_levels, ml, mr)
    }
}
