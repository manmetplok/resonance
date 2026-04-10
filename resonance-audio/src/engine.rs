/// The core audio engine managing tracks, clips, and the cpal output stream.

/// Ring buffer size for recording input: ~10 seconds at 96kHz stereo.
const RECORDING_RING_SIZE: usize = 96000 * 2 * 10;
/// Pre-allocation for recording buffers: ~60 seconds of stereo audio.
const RECORDING_PREALLOC_SECONDS: usize = 60;
/// Hard cap on concurrent busses. Used to pre-allocate bus summing
/// buffers at startup so the audio thread never has to allocate on a
/// bus add. 32 is well past what any realistic project needs.
pub(crate) const MAX_BUSSES: usize = 32;

use indexmap::IndexMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use ringbuf::traits::Split;

use crate::clap_host::{ClapBundle, SyncClapInstance};
use crate::decode;
use crate::mixer;
use crate::platform::{self, DeviceDirection};
use crate::recording::RecordingState;
use crate::types::*;

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
            let mut note_event_buf: Vec<PendingNoteEvent> = Vec::with_capacity(256);
            let mut monitor_temp = vec![0.0f32; audio_buf_frames * 2];
            let monitor_ring = ringbuf::HeapRb::<f32>::new(audio_quantum * 2 * 4);
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
                engine_thread(
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

/// The engine control thread processes commands and sends events.
fn engine_thread(
    cmd_rx: Receiver<AudioCommand>,
    cmd_tx_retry: Sender<AudioCommand>,
    event_tx: Sender<AudioEvent>,
    shared: Arc<SharedState>,
    tracks: Arc<parking_lot::RwLock<IndexMap<TrackId, Track>>>,
    busses: Arc<parking_lot::RwLock<IndexMap<BusId, Bus>>>,
    clips: Arc<parking_lot::RwLock<Vec<AudioClip>>>,
    midi_clips: Arc<parking_lot::RwLock<Vec<MidiClip>>>,
    tempo_map: Arc<parking_lot::RwLock<TempoMap>>,
    plugins: Arc<parking_lot::RwLock<IndexMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>>>,
    monitor_prod: Arc<parking_lot::Mutex<ringbuf::HeapProd<f32>>>,
    sample_rate: u32,
    buf_frames: usize,
    quantum: usize,
) {
    let mut next_track_id: TrackId = 1;
    let mut next_bus_id: BusId = 1;
    let mut next_clip_id: ClipId = 1;
    let mut next_plugin_id: PluginInstanceId = 1;
    let mut last_playhead_report = std::time::Instant::now();

    // Limit concurrent import threads to avoid unbounded thread spawning.
    let active_imports: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    const MAX_CONCURRENT_IMPORTS: usize = 4;

    // Recording state (engine thread local)
    let mut rec = RecordingState::new(sample_rate);

    // Plugin hosting state (engine thread local)
    let mut bundles: Vec<ClapBundle> = Vec::new();

    // Report actual sample rate to GUI
    let _ = event_tx.send(AudioEvent::SampleRateDetected { sample_rate });

    // Add a default track
    {
        let track = Track::new(next_track_id, "Track 1".to_string());
        let id = next_track_id;
        tracks.write().insert(id, track);
        let _ = event_tx.send(AudioEvent::TrackAdded { track_id: id });
        next_track_id += 1;
    }

    loop {
        match cmd_rx.recv_timeout(std::time::Duration::from_millis(16)) {
            Ok(cmd) => match cmd {
                AudioCommand::Play => {
                    shared.playing.store(true, Ordering::SeqCst);
                }
                AudioCommand::Record => {
                    shared.playing.store(true, Ordering::SeqCst);

                    let armed_tracks: Vec<(TrackId, Option<String>)> = {
                        let tracks_guard = tracks.read();
                        tracks_guard
                            .values()
                            .filter(|t| t.record_armed())
                            .map(|t| (t.id, t.input_device_name.clone()))
                            .collect()
                    };

                    if !armed_tracks.is_empty() {
                        let source_name: Option<String> = armed_tracks
                            .iter()
                            .find_map(|(_, name)| name.clone());

                        let ring_size = RECORDING_RING_SIZE;
                        let ring = ringbuf::HeapRb::<f32>::new(ring_size);
                        let (prod, cons) = ring.split();
                        rec.ring_consumer = Some(cons);

                        rec.start_sample = shared.playhead.load(Ordering::SeqCst);
                        for (track_id, _) in &armed_tracks {
                            rec.buffers.insert(
                                *track_id,
                                Vec::with_capacity(sample_rate as usize * 2 * RECORDING_PREALLOC_SECONDS),
                            );
                        }
                        shared.recording.store(true, Ordering::SeqCst);

                        match platform::build_input_stream(
                            source_name.as_deref(),
                            Arc::clone(&shared),
                            Some(prod),
                            Arc::clone(&monitor_prod),
                            buf_frames,
                            quantum,
                        ) {
                            Ok((stream, in_sr, in_ch)) => {
                                rec.input_stream = Some(stream);
                                rec.input_sample_rate = in_sr;
                                rec.input_channels = in_ch;

                                let _ = event_tx.send(AudioEvent::RecordingStarted {
                                    start_sample: rec.start_sample,
                                });
                            }
                            Err(e) => {
                                shared.recording.store(false, Ordering::SeqCst);
                                rec.buffers.clear();
                                rec.ring_consumer = None;
                                let _ = event_tx.send(AudioEvent::Error(format!(
                                    "Failed to start recording: {}",
                                    e
                                )));
                            }
                        }
                    }
                }
                AudioCommand::Pause => {
                    let was_recording = shared.recording.load(Ordering::SeqCst);
                    shared.playing.store(false, Ordering::SeqCst);
                    shared.recording.store(false, Ordering::SeqCst);

                    if was_recording {
                        rec.finalize_recording(
                            sample_rate,
                            &mut next_clip_id,
                            &clips,
                            &event_tx,
                        );
                        rec.input_stream = None;
                    }
                }
                AudioCommand::Stop => {
                    let was_recording = shared.recording.load(Ordering::SeqCst);
                    shared.playing.store(false, Ordering::SeqCst);
                    shared.recording.store(false, Ordering::SeqCst);
                    shared.playhead.store(0, Ordering::SeqCst);

                    if was_recording {
                        rec.finalize_recording(
                            sample_rate,
                            &mut next_clip_id,
                            &clips,
                            &event_tx,
                        );
                        rec.input_stream = None;
                    }

                    // Send all-notes-off to instrument plugins to prevent stuck notes
                    {
                        let tracks_guard = tracks.read();
                        let plugins_guard = plugins.read();
                        for track in tracks_guard.values() {
                            if track.track_type == TrackType::Instrument {
                                if let Some(&inst_id) = track.plugin_ids.first() {
                                    if let Some(mutex) = plugins_guard.get(&inst_id) {
                                        let mut inst = mutex.lock();
                                        inst.0.all_notes_off();
                                    }
                                }
                            }
                        }
                    }

                    let _ = event_tx.send(AudioEvent::Stopped);
                }
                AudioCommand::SeekTo(pos) => {
                    shared.playhead.store(pos, Ordering::SeqCst);
                }
                AudioCommand::ImportClip {
                    track_id,
                    path,
                    start_sample,
                } => {
                    if active_imports.load(Ordering::Relaxed) >= MAX_CONCURRENT_IMPORTS {
                        eprintln!("Warning: too many concurrent imports ({MAX_CONCURRENT_IMPORTS}), skipping import of {:?}", path);
                        let _ = event_tx.send(AudioEvent::Error(
                            "Too many concurrent imports, please wait for current imports to finish.".to_string(),
                        ));
                    } else {
                    let clips = Arc::clone(&clips);
                    let thread_event_tx = event_tx.clone();
                    let clip_id = next_clip_id;
                    next_clip_id += 1;
                    let sr = sample_rate;
                    let imports_counter = Arc::clone(&active_imports);
                    imports_counter.fetch_add(1, Ordering::Relaxed);

                    let spawn_result = std::thread::Builder::new()
                        .name("resonance-decode".into())
                        .spawn(move || {
                            match decode::decode_file(&path, sr) {
                                Ok((data, name)) => {
                                    let duration = (data.len() / 2) as u64;
                                    let waveform_peaks = compute_waveform_peaks(&data);
                                    let clip = AudioClip {
                                        id: clip_id,
                                        track_id,
                                        start_sample,
                                        data,
                                        name: name.clone(),
                                        trim_start_frames: 0,
                                        trim_end_frames: 0,
                                    };
                                    clips.write().push(clip);
                                    let _ = thread_event_tx.send(AudioEvent::ClipImported {
                                        clip_id,
                                        track_id,
                                        start_sample,
                                        duration_samples: duration,
                                        name,
                                        waveform_peaks,
                                    });
                                }
                                Err(e) => {
                                    let _ = thread_event_tx.send(AudioEvent::Error(format!(
                                        "Failed to import clip: {}",
                                        e
                                    )));
                                }
                            }
                            imports_counter.fetch_sub(1, Ordering::Relaxed);
                        });
                    if let Err(e) = spawn_result {
                        active_imports.fetch_sub(1, Ordering::Relaxed);
                        let _ = event_tx.send(AudioEvent::Error(format!(
                            "Failed to spawn decode thread: {}",
                            e
                        )));
                    }
                    }
                }
                AudioCommand::MoveClip {
                    clip_id,
                    new_start_sample,
                    new_track_id,
                } => {
                    let mut clips = clips.write();
                    if let Some(clip) = clips.iter_mut().find(|c| c.id == clip_id) {
                        clip.start_sample = new_start_sample;
                        clip.track_id = new_track_id;
                        let _ = event_tx.send(AudioEvent::ClipMoved {
                            clip_id,
                            new_start_sample,
                            new_track_id,
                        });
                    }
                }
                AudioCommand::TrimClip {
                    clip_id,
                    new_start_sample,
                    trim_start_frames,
                    trim_end_frames,
                } => {
                    let mut clips = clips.write();
                    if let Some(clip) = clips.iter_mut().find(|c| c.id == clip_id) {
                        clip.start_sample = new_start_sample;
                        clip.trim_start_frames = trim_start_frames;
                        clip.trim_end_frames = trim_end_frames;
                        let _ = event_tx.send(AudioEvent::ClipTrimmed {
                            clip_id,
                            new_start_sample,
                            new_duration_samples: clip.duration_frames(),
                            trim_start_frames,
                            trim_end_frames,
                        });
                    }
                }
                AudioCommand::DeleteClip { clip_id } => {
                    clips.write().retain(|c| c.id != clip_id);
                    let _ = event_tx.send(AudioEvent::ClipDeleted { clip_id });
                }
                AudioCommand::SetTrackVolume { track_id, volume } => {
                    if let Some(track) = tracks.read().get(&track_id) {
                        track.set_volume(volume.max(0.0));
                    }
                }
                AudioCommand::SetTrackPan { track_id, pan } => {
                    if let Some(track) = tracks.read().get(&track_id) {
                        track.set_pan(pan.clamp(-1.0, 1.0));
                    }
                }
                AudioCommand::SetTrackMute { track_id, muted } => {
                    if let Some(track) = tracks.read().get(&track_id) {
                        track.set_muted(muted);
                    }
                }
                AudioCommand::SetMasterVolume { volume } => {
                    shared.master_volume_bits.store(volume.max(0.0).to_bits(), Ordering::Relaxed);
                }
                AudioCommand::SetTrackSolo { track_id, soloed } => {
                    if let Some(track) = tracks.read().get(&track_id) {
                        track.set_soloed(soloed);
                    }
                }
                AudioCommand::AddTrack => {
                    let id = next_track_id;
                    next_track_id += 1;
                    let track = Track::new(id, format!("Track {}", id));
                    tracks.write().insert(id, track);
                    let _ = event_tx.send(AudioEvent::TrackAdded { track_id: id });
                }
                AudioCommand::RemoveTrack { track_id } => {
                    // Remove plugins for this track -- extract under write lock,
                    // then drop instances outside the lock so audio callback isn't blocked
                    let removed_plugins: Vec<_> = {
                        let plugin_ids = tracks
                            .read()
                            .get(&track_id)
                            .map(|t| t.plugin_ids.clone());
                        if let Some(ids) = plugin_ids {
                            let mut plugins_guard = plugins.write();
                            ids.iter().filter_map(|pid| plugins_guard.shift_remove(pid)).collect()
                        } else {
                            Vec::new()
                        }
                    };
                    drop(removed_plugins);
                    tracks.write().shift_remove(&track_id);
                    // Remove clips -- collect removed clips so dealloc happens outside lock
                    let removed_clips: Vec<_> = {
                        let mut clips_guard = clips.write();
                        let mut removed = Vec::new();
                        let mut i = 0;
                        while i < clips_guard.len() {
                            if clips_guard[i].track_id == track_id {
                                removed.push(clips_guard.swap_remove(i));
                            } else {
                                i += 1;
                            }
                        }
                        removed
                    };
                    drop(removed_clips);
                    rec.buffers.remove(&track_id);
                    let _ = event_tx.send(AudioEvent::TrackRemoved { track_id });
                }
                AudioCommand::SetTrackRecordArm { track_id, armed } => {
                    if let Some(track) = tracks.read().get(&track_id) {
                        track.set_record_armed(armed);
                    }
                }
                AudioCommand::SetTrackMono { track_id, mono } => {
                    if let Some(track) = tracks.read().get(&track_id) {
                        track.set_mono(mono);
                    }
                }
                AudioCommand::SetTrackMonitor {
                    track_id,
                    enabled,
                } => {
                    if let Some(track) = tracks.read().get(&track_id) {
                        track.set_monitor_enabled(enabled);
                    }
                    // Update monitoring flag: true if any track has monitoring enabled
                    let any_monitoring = tracks.read().values().any(|t| t.monitor_enabled());
                    shared.monitoring.store(any_monitoring, Ordering::SeqCst);

                    // Start input stream if monitoring and no stream active
                    if any_monitoring && rec.input_stream.is_none() {
                        let source_name: Option<String> = {
                            let tg = tracks.read();
                            tg.values()
                                .find(|t| t.monitor_enabled())
                                .and_then(|t| t.input_device_name.clone())
                        };
                        match platform::build_input_stream(
                            source_name.as_deref(),
                            Arc::clone(&shared),
                            None,
                            Arc::clone(&monitor_prod),
                            buf_frames,
                            quantum,
                        ) {
                            Ok((stream, in_sr, in_ch)) => {
                                rec.input_stream = Some(stream);
                                rec.input_sample_rate = in_sr;
                                rec.input_channels = in_ch;
                            }
                            Err(e) => {
                                let _ = event_tx.send(AudioEvent::Error(format!(
                                    "Failed to start monitoring: {}", e
                                )));
                            }
                        }
                    } else if !any_monitoring && !shared.recording.load(Ordering::SeqCst) {
                        // Stop input stream if no monitoring and not recording
                        rec.input_stream = None;
                    }
                }
                AudioCommand::SetTrackInputDevice {
                    track_id,
                    device_name,
                } => {
                    if let Some(track) = tracks.write().get_mut(&track_id) {
                        track.input_device_name = device_name;
                    }
                }
                AudioCommand::ListInputDevices => {
                    let (devices, default_name) = platform::enumerate_input_devices();
                    let _ = event_tx.send(AudioEvent::InputDevicesListed {
                        devices,
                        default_name,
                    });
                }
                AudioCommand::SetBpm { bpm } => {
                    tempo_map.write().bpm = bpm.clamp(20.0, 999.0);
                }
                AudioCommand::SetTimeSignature {
                    numerator,
                    denominator,
                } => {
                    let mut tm = tempo_map.write();
                    tm.numerator = numerator.max(1);
                    tm.denominator = denominator.max(1);
                }
                AudioCommand::SetMetronomeEnabled { enabled } => {
                    tempo_map.write().metronome_enabled = enabled;
                }
                AudioCommand::AddPlugin {
                    track_id,
                    clap_file_path,
                    clap_plugin_id,
                } => {
                    let path = Path::new(&clap_file_path);

                    // Load bundle (or reuse existing)
                    let bundle_idx = bundles.iter().position(|b| {
                        b.descriptors().iter().any(|d| d.id == clap_plugin_id)
                    });

                    let bundle_idx = match bundle_idx {
                        Some(idx) => idx,
                        None => {
                            match ClapBundle::load(path) {
                                Ok(bundle) => {
                                    bundles.push(bundle);
                                    bundles.len() - 1
                                }
                                Err(e) => {
                                    let _ = event_tx.send(AudioEvent::Error(format!(
                                        "Failed to load plugin: {}", e
                                    )));
                                    continue;
                                }
                            }
                        }
                    };

                    // Determine which plugin ID to instantiate
                    let actual_plugin_id = if clap_plugin_id.is_empty() {
                        match bundles[bundle_idx].descriptors().first() {
                            Some(d) => d.id.clone(),
                            None => {
                                let _ = event_tx.send(AudioEvent::Error(
                                    "No plugins found in file".to_string(),
                                ));
                                continue;
                            }
                        }
                    } else {
                        clap_plugin_id
                    };

                    let plugin_name = bundles[bundle_idx]
                        .descriptors()
                        .iter()
                        .find(|d| d.id == actual_plugin_id)
                        .map(|d| d.name.clone())
                        .unwrap_or_else(|| actual_plugin_id.clone());

                    match bundles[bundle_idx].create_instance(&actual_plugin_id, sample_rate) {
                        Ok(instance) => {
                            let instance_id = next_plugin_id;
                            next_plugin_id += 1;

                            // Query params + has_gui before moving instance into shared map
                            let params = instance.query_params();
                            let has_gui = instance.has_gui();

                            plugins.write().insert(instance_id, parking_lot::Mutex::new(SyncClapInstance(instance)));

                            if let Some(track) = tracks.write().get_mut(&track_id) {
                                track.plugin_ids.push(instance_id);
                            }

                            let _ = event_tx.send(AudioEvent::PluginAdded {
                                track_id,
                                instance_id,
                                plugin_name,
                                clap_plugin_id: actual_plugin_id.clone(),
                                clap_file_path,
                                params,
                                has_gui,
                            });
                        }
                        Err(e) => {
                            let _ = event_tx.send(AudioEvent::Error(format!(
                                "Failed to create plugin instance: {}", e
                            )));
                        }
                    }
                }
                AudioCommand::RemovePlugin {
                    track_id,
                    instance_id,
                } => {
                    if let Some(track) = tracks.write().get_mut(&track_id) {
                        track.plugin_ids.retain(|&id| id != instance_id);
                    }
                    // Remove from map then drop outside the write lock
                    // so the audio callback isn't blocked during plugin deactivation
                    let removed = plugins.write().shift_remove(&instance_id);
                    drop(removed);
                    let _ = event_tx.send(AudioEvent::PluginRemoved {
                        track_id,
                        instance_id,
                    });
                }
                AudioCommand::ScanPlugins => {
                    let mut scanned = Vec::new();
                    let mut scan_dirs: Vec<std::path::PathBuf> = Vec::new();
                    // Drop all existing plugin instances before clearing bundles,
                    // to prevent use-after-free from accessing unloaded shared libraries
                    {
                        let mut plugins_guard = plugins.write();
                        let removed: Vec<_> = plugins_guard.drain(..).collect();
                        drop(plugins_guard);
                        drop(removed);
                    }
                    for track in tracks.write().values_mut() {
                        track.plugin_ids.clear();
                    }
                    // Clear previous scan results to avoid duplicates
                    bundles.clear();

                    // ~/.clap/
                    if let Some(home) = std::env::var_os("HOME") {
                        let clap_dir = std::path::PathBuf::from(home).join(".clap");
                        if clap_dir.is_dir() {
                            scan_dirs.push(clap_dir);
                        }
                    }

                    // /usr/lib/clap/
                    let sys_dir = std::path::PathBuf::from("/usr/lib/clap");
                    if sys_dir.is_dir() {
                        scan_dirs.push(sys_dir);
                    }

                    // Bundled plugins: find target/bundled/ relative to the executable
                    if let Ok(exe) = std::env::current_exe() {
                        if let Some(exe_dir) = exe.parent() {
                            // cargo run: target/debug/ -> look for ../../target/bundled/
                            let bundled = exe_dir
                                .parent()
                                .and_then(|p| p.parent())
                                .map(|p| p.join("target").join("bundled"));
                            if let Some(dir) = bundled {
                                if dir.is_dir() {
                                    scan_dirs.push(dir);
                                }
                            }
                        }
                    }

                    // Also check workspace root target/bundled/
                    let workspace_bundled = std::path::PathBuf::from("target/bundled");
                    if workspace_bundled.is_dir() {
                        if let Ok(canonical) = workspace_bundled.canonicalize() {
                            if !scan_dirs.iter().any(|d| d.canonicalize().ok().as_ref() == Some(&canonical)) {
                                scan_dirs.push(workspace_bundled);
                            }
                        } else {
                            scan_dirs.push(workspace_bundled);
                        }
                    }

                    for dir in &scan_dirs {
                        let entries = match std::fs::read_dir(dir) {
                            Ok(e) => e,
                            Err(_) => continue,
                        };
                        for entry in entries.flatten() {
                            let path = entry.path();
                            // Handle both .clap files and .clap directories (bundles)
                            let is_clap = path
                                .extension()
                                .map(|e| e == "clap")
                                .unwrap_or(false);
                            // Also follow symlinks to .so files named *.clap
                            let is_clap = is_clap || path.to_str().map(|s| s.ends_with(".clap")).unwrap_or(false);

                            if !is_clap {
                                continue;
                            }

                            // Resolve symlinks for loading
                            let real_path = match std::fs::canonicalize(&path) {
                                Ok(p) => p,
                                Err(_) => path.clone(),
                            };

                            match ClapBundle::load(&real_path) {
                                Ok(bundle) => {
                                    for desc in bundle.descriptors() {
                                        scanned.push(ScannedPlugin {
                                            clap_file_path: real_path
                                                .to_string_lossy()
                                                .to_string(),
                                            clap_plugin_id: desc.id.clone(),
                                            name: desc.name.clone(),
                                            vendor: desc.vendor.clone(),
                                            is_instrument: desc.is_instrument,
                                        });
                                    }
                                    // Keep bundle alive for later instantiation
                                    bundles.push(bundle);
                                }
                                Err(e) => {
                                    eprintln!(
                                        "Failed to scan {}: {}",
                                        path.display(),
                                        e
                                    );
                                }
                            }
                        }
                    }

                    let _ = event_tx.send(AudioEvent::PluginsScanned {
                        plugins: scanned,
                    });
                }
                AudioCommand::SetPluginParam {
                    instance_id,
                    param_id,
                    value,
                } => {
                    if let Some(mutex) = plugins.read().get(&instance_id) {
                        mutex.lock().0.set_param(param_id, value);
                    }
                }
                AudioCommand::OpenPluginEditor { instance_id } => {
                    if let Some(mutex) = plugins.read().get(&instance_id) {
                        // open_gui is a main-thread operation; the audio
                        // thread holds a different lock. Block briefly if
                        // the audio thread is mid-process and retry.
                        if let Some(mut inst) = mutex.try_lock() {
                            if !inst.0.open_gui() {
                                let _ = event_tx.send(AudioEvent::Error(
                                    "Failed to open plugin editor".to_string(),
                                ));
                            }
                        } else {
                            let _ = cmd_tx_retry
                                .send(AudioCommand::OpenPluginEditor { instance_id });
                        }
                    }
                }
                AudioCommand::ClosePluginEditor { instance_id } => {
                    if let Some(mutex) = plugins.read().get(&instance_id) {
                        if let Some(mut inst) = mutex.try_lock() {
                            inst.0.close_gui();
                        } else {
                            let _ = cmd_tx_retry
                                .send(AudioCommand::ClosePluginEditor { instance_id });
                        }
                    }
                }
                AudioCommand::SavePluginState { instance_id } => {
                    if let Some(mutex) = plugins.read().get(&instance_id) {
                        if let Some(inst) = mutex.try_lock() {
                            let data = inst.0.save_state();
                            if let Some(data) = data {
                                let _ = event_tx.send(AudioEvent::PluginStateSaved {
                                    instance_id,
                                    data,
                                });
                            }
                        } else {
                            // Audio thread holds the lock — retry next tick
                            let _ = cmd_tx_retry.send(AudioCommand::SavePluginState { instance_id });
                        }
                    }
                }
                AudioCommand::LoadPluginState { instance_id, data } => {
                    if let Some(mutex) = plugins.read().get(&instance_id) {
                        if let Some(mut inst) = mutex.try_lock() {
                            inst.0.reload_with_state(&data);
                        } else {
                            // Audio thread holds the lock — retry next tick
                            let _ = cmd_tx_retry.send(AudioCommand::LoadPluginState { instance_id, data });
                        }
                    }
                }
                AudioCommand::BounceToWav { path } => {
                    // Compute project range from audio clips + MIDI clips
                    let (render_start, render_end) = {
                        let clips_guard = clips.read();
                        let midi_guard = midi_clips.read();
                        let tm = tempo_map.read();
                        let spt = tm.samples_per_beat(sample_rate) / TICKS_PER_QUARTER_NOTE as f64;

                        if clips_guard.is_empty() && midi_guard.is_empty() {
                            let _ = event_tx.send(AudioEvent::BounceError(
                                "No clips to bounce".into(),
                            ));
                            continue;
                        }
                        let audio_start = clips_guard.iter().map(|c| c.start_sample).min();
                        let audio_end = clips_guard.iter().map(|c| c.end_sample()).max();
                        let midi_start = midi_guard.iter().map(|c| c.start_sample).min();
                        let midi_end = midi_guard.iter().map(|c| c.end_sample(spt)).max();

                        let start = audio_start.into_iter().chain(midi_start).min().unwrap_or(0);
                        let end = audio_end.into_iter().chain(midi_end).max().unwrap_or(0);
                        (start, end)
                    };

                    if render_end <= render_start {
                        let _ = event_tx.send(AudioEvent::BounceError(
                            "No audio to bounce".into(),
                        ));
                        continue;
                    }

                    // Open WAV writer
                    let spec = hound::WavSpec {
                        channels: 2,
                        sample_rate,
                        bits_per_sample: 32,
                        sample_format: hound::SampleFormat::Float,
                    };
                    let mut writer = match hound::WavWriter::create(&path, spec) {
                        Ok(w) => w,
                        Err(e) => {
                            let _ = event_tx.send(AudioEvent::BounceError(
                                format!("Failed to create WAV file: {e}"),
                            ));
                            continue;
                        }
                    };

                    // Reset all plugins for clean render
                    {
                        let plugins_guard = plugins.read();
                        for mutex in plugins_guard.values() {
                            let mut inst = mutex.lock();
                            inst.0.reset_processing();
                        }
                    }

                    // Offline render in chunks
                    const BOUNCE_CHUNK: usize = 1024;
                    let mut track_buf_l = vec![0.0f32; BOUNCE_CHUNK];
                    let mut track_buf_r = vec![0.0f32; BOUNCE_CHUNK];
                    // Bounce mirrors live playback: pre-allocate one stereo
                    // buffer per potential bus so bus routing survives the
                    // offline render.
                    let mut bounce_bus_bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..MAX_BUSSES)
                        .map(|_| (vec![0.0f32; BOUNCE_CHUNK], vec![0.0f32; BOUNCE_CHUNK]))
                        .collect();
                    let mut bounce_note_buf: Vec<PendingNoteEvent> = Vec::with_capacity(256);
                    let mut mix_buf = vec![0.0f32; BOUNCE_CHUNK * 2];
                    let master_vol =
                        f32::from_bits(shared.master_volume_bits.load(Ordering::Relaxed));
                    let mut write_error = false;

                    let bounce_spt = {
                        let tm = tempo_map.read();
                        tm.samples_per_beat(sample_rate) / TICKS_PER_QUARTER_NOTE as f64
                    };

                    let mut pos = render_start;
                    while pos < render_end && !write_error {
                        let frames = ((render_end - pos) as usize).min(BOUNCE_CHUNK);
                        mix_buf[..frames * 2].fill(0.0);

                        let tracks_guard = tracks.read();
                        let busses_guard = busses.read();
                        let clips_guard = clips.read();
                        let midi_guard = midi_clips.read();
                        let plugins_guard = plugins.read();

                        let active_busses = busses_guard.len().min(bounce_bus_bufs.len());
                        for (bl, br) in bounce_bus_bufs.iter_mut().take(active_busses) {
                            bl[..frames].fill(0.0);
                            br[..frames].fill(0.0);
                        }

                        let any_solo = tracks_guard.values().any(|t| t.soloed());

                        for track in tracks_guard.values() {
                            if track.muted() {
                                continue;
                            }
                            if any_solo && !track.soloed() {
                                continue;
                            }

                            track_buf_l[..frames].fill(0.0);
                            track_buf_r[..frames].fill(0.0);
                            let mut has_audio = false;

                            if track.track_type == TrackType::Instrument {
                                // Instrument track: collect MIDI events and process
                                bounce_note_buf.clear();
                                mixer::collect_midi_events_bounce(
                                    &midi_guard, track.id, pos, frames,
                                    bounce_spt, &mut bounce_note_buf,
                                );
                                let mut plugin_iter = track.plugin_ids.iter();
                                if let Some(&inst_id) = plugin_iter.next() {
                                    if let Some(mutex) = plugins_guard.get(&inst_id) {
                                        let mut inst = mutex.lock();
                                        for ev in bounce_note_buf.iter() {
                                            if ev.is_note_on {
                                                inst.0.queue_note_on(ev.note, ev.velocity, ev.sample_offset);
                                            } else {
                                                inst.0.queue_note_off(ev.note, ev.sample_offset);
                                            }
                                        }
                                        inst.0.process(&mut track_buf_l[..frames], &mut track_buf_r[..frames], frames);
                                        has_audio = true;
                                    }
                                }
                                for &plugin_id in plugin_iter {
                                    if let Some(mutex) = plugins_guard.get(&plugin_id) {
                                        let mut inst = mutex.lock();
                                        inst.0.process(&mut track_buf_l[..frames], &mut track_buf_r[..frames], frames);
                                        has_audio = true;
                                    }
                                }
                            } else {
                                // Audio track: mix clips + plugin chain
                                for clip in clips_guard.iter() {
                                    if clip.track_id != track.id {
                                        continue;
                                    }
                                    let clip_start = clip.start_sample;
                                    let clip_end = clip_start + clip.duration_frames();
                                    let buf_end = pos + frames as u64;
                                    if buf_end <= clip_start || pos >= clip_end {
                                        continue;
                                    }
                                    let overlap_start = pos.max(clip_start);
                                    let overlap_end = buf_end.min(clip_end);
                                    for timeline_frame in overlap_start..overlap_end {
                                        let frame_offset = (timeline_frame - pos) as usize;
                                        let clip_frame = (timeline_frame - clip_start) as usize
                                            + clip.trim_start_frames as usize;
                                        let clip_idx = clip_frame * 2;
                                        if clip_idx + 1 < clip.data.len() {
                                            track_buf_l[frame_offset] += clip.data[clip_idx];
                                            track_buf_r[frame_offset] += clip.data[clip_idx + 1];
                                            has_audio = true;
                                        }
                                    }
                                }

                                // Process through plugin chain
                                if !track.plugin_ids.is_empty() {
                                    for &plugin_id in &track.plugin_ids {
                                        if let Some(mutex) = plugins_guard.get(&plugin_id) {
                                            let mut inst = mutex.lock();
                                            inst.0.process(
                                                &mut track_buf_l[..frames],
                                                &mut track_buf_r[..frames],
                                                frames,
                                            );
                                            has_audio = true;
                                        }
                                    }
                                }
                            }

                            if !has_audio {
                                continue;
                            }

                            // Apply track volume + pan, route to master or bus.
                            let volume = track.volume();
                            let (pan_l, pan_r) =
                                resonance_dsp::constant_power_pan(track.pan());
                            let gain_l = volume * pan_l;
                            let gain_r = volume * pan_r;

                            let routed_to_bus = match track.output() {
                                TrackOutput::Bus(bus_id) => busses_guard
                                    .get_index_of(&bus_id)
                                    .filter(|idx| *idx < active_busses)
                                    .map(|idx| {
                                        let (bl, br) = &mut bounce_bus_bufs[idx];
                                        for f in 0..frames {
                                            bl[f] += track_buf_l[f] * gain_l;
                                            br[f] += track_buf_r[f] * gain_r;
                                        }
                                    })
                                    .is_some(),
                                TrackOutput::Master => false,
                            };
                            if !routed_to_bus {
                                for f in 0..frames {
                                    mix_buf[f * 2] += track_buf_l[f] * gain_l;
                                    mix_buf[f * 2 + 1] += track_buf_r[f] * gain_r;
                                }
                            }
                        }

                        // Per-bus plugin chain + volume/pan + sum to master.
                        for (bus_idx, bus) in
                            busses_guard.values().enumerate().take(active_busses)
                        {
                            if bus.muted() {
                                continue;
                            }
                            let (bl, br) = &mut bounce_bus_bufs[bus_idx];
                            for &plugin_id in &bus.plugin_ids {
                                if let Some(mutex) = plugins_guard.get(&plugin_id) {
                                    let mut inst = mutex.lock();
                                    inst.0.process(
                                        &mut bl[..frames],
                                        &mut br[..frames],
                                        frames,
                                    );
                                }
                            }
                            let bus_volume = bus.volume();
                            let (bus_pan_l, bus_pan_r) =
                                resonance_dsp::constant_power_pan(bus.pan());
                            let bus_gain_l = bus_volume * bus_pan_l;
                            let bus_gain_r = bus_volume * bus_pan_r;
                            for f in 0..frames {
                                mix_buf[f * 2] += bl[f] * bus_gain_l;
                                mix_buf[f * 2 + 1] += br[f] * bus_gain_r;
                            }
                        }

                        drop(plugins_guard);
                        drop(clips_guard);
                        drop(busses_guard);
                        drop(tracks_guard);

                        // Apply master volume and hard clip
                        for s in &mut mix_buf[..frames * 2] {
                            *s = (*s * master_vol).clamp(-1.0, 1.0);
                        }

                        // Write to WAV
                        for &sample in &mix_buf[..frames * 2] {
                            if let Err(e) = writer.write_sample(sample) {
                                let _ = event_tx.send(AudioEvent::BounceError(
                                    format!("WAV write error: {e}"),
                                ));
                                write_error = true;
                                break;
                            }
                        }

                        pos += frames as u64;
                    }

                    if !write_error {
                        match writer.finalize() {
                            Ok(()) => {
                                let _ = event_tx
                                    .send(AudioEvent::BounceComplete { path });
                            }
                            Err(e) => {
                                let _ = event_tx.send(AudioEvent::BounceError(
                                    format!("WAV finalize error: {e}"),
                                ));
                            }
                        }
                    }
                }
                AudioCommand::SetPunchRange {
                    enabled,
                    punch_in: pi,
                    punch_out: po,
                } => {
                    rec.punch_enabled = enabled;
                    rec.punch_in = pi;
                    rec.punch_out = po;
                }
                AudioCommand::AddTrackWithId { track_id, name } => {
                    let track = Track::new(track_id, name);
                    tracks.write().insert(track_id, track);
                    next_track_id = next_track_id.max(track_id + 1);
                    let _ = event_tx.send(AudioEvent::TrackAdded { track_id });
                }
                AudioCommand::AddPluginWithId {
                    track_id,
                    instance_id,
                    clap_file_path,
                    clap_plugin_id,
                } => {
                    let path = Path::new(&clap_file_path);

                    let bundle_idx = bundles.iter().position(|b| {
                        b.descriptors().iter().any(|d| d.id == clap_plugin_id)
                    });

                    let bundle_idx = match bundle_idx {
                        Some(idx) => idx,
                        None => {
                            match ClapBundle::load(path) {
                                Ok(bundle) => {
                                    bundles.push(bundle);
                                    bundles.len() - 1
                                }
                                Err(e) => {
                                    let _ = event_tx.send(AudioEvent::Error(format!(
                                        "Failed to load plugin: {}", e
                                    )));
                                    continue;
                                }
                            }
                        }
                    };

                    let plugin_name = bundles[bundle_idx]
                        .descriptors()
                        .iter()
                        .find(|d| d.id == clap_plugin_id)
                        .map(|d| d.name.clone())
                        .unwrap_or_else(|| clap_plugin_id.clone());

                    match bundles[bundle_idx].create_instance(&clap_plugin_id, sample_rate) {
                        Ok(instance) => {
                            let params = instance.query_params();
                            let has_gui = instance.has_gui();
                            plugins.write().insert(instance_id, parking_lot::Mutex::new(SyncClapInstance(instance)));
                            next_plugin_id = next_plugin_id.max(instance_id + 1);

                            if let Some(track) = tracks.write().get_mut(&track_id) {
                                track.plugin_ids.push(instance_id);
                            }

                            let _ = event_tx.send(AudioEvent::PluginAdded {
                                track_id,
                                instance_id,
                                plugin_name,
                                clap_plugin_id,
                                clap_file_path,
                                params,
                                has_gui,
                            });
                        }
                        Err(e) => {
                            let _ = event_tx.send(AudioEvent::Error(format!(
                                "Failed to create plugin instance: {}", e
                            )));
                        }
                    }
                }
                AudioCommand::LoadClipDirect {
                    clip_id,
                    track_id,
                    start_sample,
                    data,
                    name,
                    trim_start_frames,
                    trim_end_frames,
                } => {
                    let total_frames = (data.len() / 2) as u64;
                    let waveform_peaks = compute_waveform_peaks(&data);
                    let duration_samples = total_frames
                        .saturating_sub(trim_start_frames)
                        .saturating_sub(trim_end_frames);
                    let clip = AudioClip {
                        id: clip_id,
                        track_id,
                        start_sample,
                        data,
                        name: name.clone(),
                        trim_start_frames,
                        trim_end_frames,
                    };
                    clips.write().push(clip);
                    next_clip_id = next_clip_id.max(clip_id + 1);
                    let _ = event_tx.send(AudioEvent::ClipImported {
                        clip_id,
                        track_id,
                        start_sample,
                        duration_samples,
                        name,
                        waveform_peaks,
                    });
                }
                AudioCommand::ExportAllClipData => {
                    let clips_guard = clips.read();
                    for clip in clips_guard.iter() {
                        let _ = event_tx.send(AudioEvent::ClipDataExported {
                            clip_id: clip.id,
                            data: clip.data.clone(),
                        });
                    }
                    let _ = event_tx.send(AudioEvent::AllClipDataExported);
                }
                AudioCommand::SaveAllPluginStates => {
                    let mut states = Vec::new();
                    let plugins_guard = plugins.read();
                    let mut retry = false;
                    for (&instance_id, mutex) in plugins_guard.iter() {
                        if let Some(inst) = mutex.try_lock() {
                            if let Some(data) = inst.0.save_state() {
                                states.push((instance_id, data));
                            }
                        } else {
                            retry = true;
                            break;
                        }
                    }
                    drop(plugins_guard);
                    if retry {
                        let _ = cmd_tx_retry.send(AudioCommand::SaveAllPluginStates);
                    } else {
                        let _ = event_tx.send(AudioEvent::AllPluginStatesSaved { states });
                    }
                }
                AudioCommand::ClearAll => {
                    // Stop playback/recording
                    shared.playing.store(false, Ordering::SeqCst);
                    shared.recording.store(false, Ordering::SeqCst);
                    shared.playhead.store(0, Ordering::SeqCst);
                    rec.input_stream = None;
                    rec.buffers.clear();

                    // Drop all plugin instances outside the write lock
                    {
                        let mut plugins_guard = plugins.write();
                        let removed: Vec<_> = plugins_guard.drain(..).collect();
                        drop(plugins_guard);
                        drop(removed);
                    }

                    // Clear tracks
                    tracks.write().clear();

                    // Clear clips -- collect to drop outside lock
                    let removed_clips: Vec<_> = clips.write().drain(..).collect();
                    drop(removed_clips);

                    // Clear MIDI clips
                    midi_clips.write().clear();

                    // Clear bundles
                    bundles.clear();

                    // Reset ID counters
                    next_track_id = 1;
                    next_clip_id = 1;
                    next_plugin_id = 1;

                    let _ = event_tx.send(AudioEvent::AllCleared);
                }

                // -- Instrument track commands --
                AudioCommand::AddInstrumentTrack => {
                    let id = next_track_id;
                    next_track_id += 1;
                    let track = Track::with_type(id, format!("Instrument {}", id), TrackType::Instrument);
                    tracks.write().insert(id, track);
                    let _ = event_tx.send(AudioEvent::InstrumentTrackAdded { track_id: id });
                }
                AudioCommand::AddInstrumentTrackWithId { track_id, name } => {
                    let track = Track::with_type(track_id, name, TrackType::Instrument);
                    tracks.write().insert(track_id, track);
                    next_track_id = next_track_id.max(track_id + 1);
                    let _ = event_tx.send(AudioEvent::InstrumentTrackAdded { track_id });
                }

                // -- MIDI clip commands --
                AudioCommand::CreateMidiClip { track_id, start_sample, duration_ticks, name } => {
                    let clip_id = next_clip_id;
                    next_clip_id += 1;
                    let clip = MidiClip {
                        id: clip_id,
                        track_id,
                        start_sample,
                        duration_ticks,
                        notes: Vec::new(),
                        name: name.clone(),
                        trim_start_ticks: 0,
                        trim_end_ticks: 0,
                    };
                    midi_clips.write().push(clip);
                    let _ = event_tx.send(AudioEvent::MidiClipCreated {
                        clip_id, track_id, start_sample, duration_ticks,
                        name, notes: Vec::new(), trim_start_ticks: 0, trim_end_ticks: 0,
                    });
                }
                AudioCommand::LoadMidiClipDirect {
                    clip_id, track_id, start_sample, duration_ticks,
                    notes, name, trim_start_ticks, trim_end_ticks,
                } => {
                    let clip = MidiClip {
                        id: clip_id,
                        track_id,
                        start_sample,
                        duration_ticks,
                        notes: notes.clone(),
                        name: name.clone(),
                        trim_start_ticks,
                        trim_end_ticks,
                    };
                    midi_clips.write().push(clip);
                    next_clip_id = next_clip_id.max(clip_id + 1);
                    let _ = event_tx.send(AudioEvent::MidiClipCreated {
                        clip_id, track_id, start_sample, duration_ticks,
                        name, notes, trim_start_ticks, trim_end_ticks,
                    });
                }
                AudioCommand::MoveMidiClip { clip_id, new_start_sample, new_track_id } => {
                    let mut guard = midi_clips.write();
                    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
                        clip.start_sample = new_start_sample;
                        clip.track_id = new_track_id;
                    }
                    let _ = event_tx.send(AudioEvent::MidiClipMoved {
                        clip_id, new_start_sample, new_track_id,
                    });
                }
                AudioCommand::TrimMidiClip { clip_id, new_start_sample, trim_start_ticks, trim_end_ticks } => {
                    let mut guard = midi_clips.write();
                    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
                        clip.start_sample = new_start_sample;
                        clip.trim_start_ticks = trim_start_ticks;
                        clip.trim_end_ticks = trim_end_ticks;
                    }
                    let _ = event_tx.send(AudioEvent::MidiClipTrimmed {
                        clip_id, new_start_sample, trim_start_ticks, trim_end_ticks,
                    });
                }
                AudioCommand::DeleteMidiClip { clip_id } => {
                    midi_clips.write().retain(|c| c.id != clip_id);
                    let _ = event_tx.send(AudioEvent::MidiClipDeleted { clip_id });
                }

                // -- MIDI note editing --
                AudioCommand::AddMidiNote { clip_id, note } => {
                    let mut guard = midi_clips.write();
                    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
                        let n = note.clone();
                        // Insert sorted by start_tick
                        let pos = clip.notes.partition_point(|n| n.start_tick <= note.start_tick);
                        clip.notes.insert(pos, note.clone());
                        let _ = event_tx.send(AudioEvent::MidiNoteAdded { clip_id, note: n });
                    }
                }
                AudioCommand::RemoveMidiNote { clip_id, note_index } => {
                    let mut guard = midi_clips.write();
                    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
                        if note_index < clip.notes.len() {
                            clip.notes.remove(note_index);
                            let _ = event_tx.send(AudioEvent::MidiNoteRemoved { clip_id, note_index });
                        }
                    }
                }
                AudioCommand::MoveMidiNote { clip_id, note_index, new_start_tick, new_note } => {
                    let mut guard = midi_clips.write();
                    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
                        if note_index < clip.notes.len() {
                            clip.notes[note_index].start_tick = new_start_tick;
                            clip.notes[note_index].note = new_note;
                            // Re-sort
                            clip.notes.sort_by_key(|n| n.start_tick);
                            let _ = event_tx.send(AudioEvent::MidiNoteMoved {
                                clip_id, note_index, new_start_tick, new_note,
                            });
                        }
                    }
                }
                AudioCommand::ResizeMidiNote { clip_id, note_index, new_duration_ticks } => {
                    let mut guard = midi_clips.write();
                    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
                        if note_index < clip.notes.len() {
                            clip.notes[note_index].duration_ticks = new_duration_ticks;
                            let _ = event_tx.send(AudioEvent::MidiNoteResized {
                                clip_id, note_index, new_duration_ticks,
                            });
                        }
                    }
                }
                AudioCommand::SetMidiNoteVelocity { clip_id, note_index, velocity } => {
                    let mut guard = midi_clips.write();
                    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
                        if note_index < clip.notes.len() {
                            clip.notes[note_index].velocity = velocity;
                            let _ = event_tx.send(AudioEvent::MidiNoteVelocitySet {
                                clip_id, note_index, velocity,
                            });
                        }
                    }
                }

                // -- Live MIDI input --
                AudioCommand::SendNoteOn { track_id, note, velocity } => {
                    let tracks_guard = tracks.read();
                    if let Some(track) = tracks_guard.get(&track_id) {
                        if track.track_type == TrackType::Instrument {
                            if let Some(&inst_id) = track.plugin_ids.first() {
                                let plugins_guard = plugins.read();
                                if let Some(mutex) = plugins_guard.get(&inst_id) {
                                    let mut inst = mutex.lock();
                                    inst.0.queue_note_on(note, velocity, 0);
                                }
                            }
                        }
                    }
                }
                AudioCommand::SendNoteOff { track_id, note } => {
                    let tracks_guard = tracks.read();
                    if let Some(track) = tracks_guard.get(&track_id) {
                        if track.track_type == TrackType::Instrument {
                            if let Some(&inst_id) = track.plugin_ids.first() {
                                let plugins_guard = plugins.read();
                                if let Some(mutex) = plugins_guard.get(&inst_id) {
                                    let mut inst = mutex.lock();
                                    inst.0.queue_note_off(note, 0);
                                }
                            }
                        }
                    }
                }

                // -- Bus commands --
                AudioCommand::AddBus => {
                    let mut busses_guard = busses.write();
                    if busses_guard.len() >= MAX_BUSSES {
                        let _ = event_tx.send(AudioEvent::Error(format!(
                            "Cannot add bus: maximum of {MAX_BUSSES} busses reached"
                        )));
                    } else {
                        let bus_id = next_bus_id;
                        next_bus_id += 1;
                        let name = format!("Bus {bus_id}");
                        busses_guard.insert(bus_id, Bus::new(bus_id, name.clone()));
                        drop(busses_guard);
                        let _ = event_tx.send(AudioEvent::BusAdded { bus_id, name });
                    }
                }
                AudioCommand::AddBusWithId { bus_id, name } => {
                    let mut busses_guard = busses.write();
                    if busses_guard.len() >= MAX_BUSSES {
                        let _ = event_tx.send(AudioEvent::Error(format!(
                            "Cannot add bus: maximum of {MAX_BUSSES} busses reached"
                        )));
                    } else if !busses_guard.contains_key(&bus_id) {
                        busses_guard.insert(bus_id, Bus::new(bus_id, name.clone()));
                        next_bus_id = next_bus_id.max(bus_id + 1);
                        drop(busses_guard);
                        let _ = event_tx.send(AudioEvent::BusAdded { bus_id, name });
                    }
                }
                AudioCommand::RemoveBus { bus_id } => {
                    // First: unassign any track that was routed here so no
                    // dangling references survive the removal.
                    {
                        let tracks_guard = tracks.read();
                        for track in tracks_guard.values() {
                            if track.output() == TrackOutput::Bus(bus_id) {
                                track.set_output(TrackOutput::Master);
                            }
                        }
                    }
                    // Collect the bus's plugin ids before removing it so we
                    // can tear them down outside the busses lock.
                    let removed_plugins: Vec<PluginInstanceId> = {
                        let mut busses_guard = busses.write();
                        if let Some(bus) = busses_guard.shift_remove(&bus_id) {
                            bus.plugin_ids
                        } else {
                            Vec::new()
                        }
                    };
                    // Drop plugin instances off the audio path.
                    {
                        let mut plugins_guard = plugins.write();
                        for pid in &removed_plugins {
                            if let Some(inst) = plugins_guard.shift_remove(pid) {
                                drop(inst);
                            }
                        }
                    }
                    let _ = event_tx.send(AudioEvent::BusRemoved { bus_id });
                }
                AudioCommand::SetBusVolume { bus_id, volume } => {
                    if let Some(bus) = busses.read().get(&bus_id) {
                        bus.set_volume(volume);
                    }
                }
                AudioCommand::SetBusPan { bus_id, pan } => {
                    if let Some(bus) = busses.read().get(&bus_id) {
                        bus.set_pan(pan);
                    }
                }
                AudioCommand::SetBusMute { bus_id, muted } => {
                    if let Some(bus) = busses.read().get(&bus_id) {
                        bus.set_muted(muted);
                    }
                }
                AudioCommand::SetBusName { bus_id, name } => {
                    if let Some(bus) = busses.write().get_mut(&bus_id) {
                        bus.name = name;
                    }
                }
                AudioCommand::SetTrackOutput { track_id, output } => {
                    if let Some(track) = tracks.read().get(&track_id) {
                        track.set_output(output);
                    }
                }
                AudioCommand::AddPluginToBus {
                    bus_id,
                    clap_file_path,
                    clap_plugin_id,
                } => {
                    let path = Path::new(&clap_file_path);
                    let bundle_idx = bundles.iter().position(|b| {
                        b.descriptors().iter().any(|d| d.id == clap_plugin_id)
                    });
                    let bundle_idx = match bundle_idx {
                        Some(idx) => idx,
                        None => match ClapBundle::load(path) {
                            Ok(bundle) => {
                                bundles.push(bundle);
                                bundles.len() - 1
                            }
                            Err(e) => {
                                let _ = event_tx.send(AudioEvent::Error(format!(
                                    "Failed to load plugin: {}", e
                                )));
                                continue;
                            }
                        },
                    };
                    let actual_plugin_id = if clap_plugin_id.is_empty() {
                        match bundles[bundle_idx].descriptors().first() {
                            Some(d) => d.id.clone(),
                            None => {
                                let _ = event_tx.send(AudioEvent::Error(
                                    "No plugins found in file".to_string(),
                                ));
                                continue;
                            }
                        }
                    } else {
                        clap_plugin_id
                    };
                    let plugin_name = bundles[bundle_idx]
                        .descriptors()
                        .iter()
                        .find(|d| d.id == actual_plugin_id)
                        .map(|d| d.name.clone())
                        .unwrap_or_else(|| actual_plugin_id.clone());
                    match bundles[bundle_idx].create_instance(&actual_plugin_id, sample_rate) {
                        Ok(instance) => {
                            let instance_id = next_plugin_id;
                            next_plugin_id += 1;
                            let params = instance.query_params();
                            let has_gui = instance.has_gui();
                            plugins.write().insert(
                                instance_id,
                                parking_lot::Mutex::new(SyncClapInstance(instance)),
                            );
                            if let Some(bus) = busses.write().get_mut(&bus_id) {
                                bus.plugin_ids.push(instance_id);
                            }
                            let _ = event_tx.send(AudioEvent::BusPluginAdded {
                                bus_id,
                                instance_id,
                                plugin_name,
                                clap_plugin_id: actual_plugin_id.clone(),
                                clap_file_path,
                                params,
                                has_gui,
                            });
                        }
                        Err(e) => {
                            let _ = event_tx.send(AudioEvent::Error(format!(
                                "Failed to create plugin instance: {}", e
                            )));
                        }
                    }
                }
                AudioCommand::AddPluginToBusWithId {
                    bus_id,
                    instance_id,
                    clap_file_path,
                    clap_plugin_id,
                } => {
                    let path = Path::new(&clap_file_path);
                    let bundle_idx = bundles.iter().position(|b| {
                        b.descriptors().iter().any(|d| d.id == clap_plugin_id)
                    });
                    let bundle_idx = match bundle_idx {
                        Some(idx) => idx,
                        None => match ClapBundle::load(path) {
                            Ok(bundle) => {
                                bundles.push(bundle);
                                bundles.len() - 1
                            }
                            Err(e) => {
                                let _ = event_tx.send(AudioEvent::Error(format!(
                                    "Failed to load plugin: {}", e
                                )));
                                continue;
                            }
                        },
                    };
                    let plugin_name = bundles[bundle_idx]
                        .descriptors()
                        .iter()
                        .find(|d| d.id == clap_plugin_id)
                        .map(|d| d.name.clone())
                        .unwrap_or_else(|| clap_plugin_id.clone());
                    match bundles[bundle_idx].create_instance(&clap_plugin_id, sample_rate) {
                        Ok(instance) => {
                            let params = instance.query_params();
                            let has_gui = instance.has_gui();
                            plugins.write().insert(
                                instance_id,
                                parking_lot::Mutex::new(SyncClapInstance(instance)),
                            );
                            if let Some(bus) = busses.write().get_mut(&bus_id) {
                                bus.plugin_ids.push(instance_id);
                            }
                            next_plugin_id = next_plugin_id.max(instance_id + 1);
                            let _ = event_tx.send(AudioEvent::BusPluginAdded {
                                bus_id,
                                instance_id,
                                plugin_name,
                                clap_plugin_id,
                                clap_file_path,
                                params,
                                has_gui,
                            });
                        }
                        Err(e) => {
                            let _ = event_tx.send(AudioEvent::Error(format!(
                                "Failed to create plugin instance: {}", e
                            )));
                        }
                    }
                }
                AudioCommand::RemovePluginFromBus {
                    bus_id,
                    instance_id,
                } => {
                    if let Some(bus) = busses.write().get_mut(&bus_id) {
                        bus.plugin_ids.retain(|&id| id != instance_id);
                    }
                    let removed = plugins.write().shift_remove(&instance_id);
                    drop(removed);
                    let _ = event_tx.send(AudioEvent::BusPluginRemoved {
                        bus_id,
                        instance_id,
                    });
                }
            },
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }

        // Drain recording ring buffer into per-track buffers
        if shared.recording.load(Ordering::Relaxed) {
            rec.drain_ring_to_buffers();

            // Auto-stop recording at punch_out (keep playing)
            if rec.punch_enabled && rec.punch_out > rec.punch_in {
                let current_pos = shared.playhead.load(Ordering::SeqCst);
                if current_pos >= rec.punch_out {
                    shared.recording.store(false, Ordering::SeqCst);
                    rec.finalize_recording(
                        sample_rate,
                        &mut next_clip_id,
                        &clips,
                        &event_tx,
                    );
                    rec.input_stream = None;
                }
            }
        }

        // Report playhead position at ~60Hz using wall-clock time
        if shared.playing.load(Ordering::SeqCst)
            && last_playhead_report.elapsed() >= std::time::Duration::from_millis(16)
        {
            last_playhead_report = std::time::Instant::now();
            let pos = shared.playhead.load(Ordering::SeqCst);
            let _ = event_tx.send(AudioEvent::PlayheadMoved(pos));
        }
    }
}
