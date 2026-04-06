/// The core audio engine managing tracks, clips, and the cpal output stream.
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use ringbuf::traits::{Consumer, Producer, Split};

use crate::clap_host::{ClapBundle, SyncClapInstance};
use crate::decode;
use crate::types::*;

/// Shared state between the engine control thread and the audio callback.
struct SharedState {
    /// Current playhead position in sample frames.
    playhead: AtomicU64,
    /// Whether playback is active.
    playing: AtomicBool,
    /// Whether recording is active.
    recording: AtomicBool,
    /// Whether any track is monitoring input.
    monitoring: AtomicBool,
}

/// The audio engine.
pub struct AudioEngine {
    cmd_tx: Sender<AudioCommand>,
    event_rx: Receiver<AudioEvent>,
    _stream: Option<cpal::Stream>,
}

// cpal::Stream is not Send by default on some platforms but we manage it carefully
unsafe impl Send for AudioEngine {}

/// Serializes access to PIPEWIRE_NODE env var manipulation.
static PIPEWIRE_ENV_LOCK: Mutex<()> = Mutex::new(());

impl AudioEngine {
    /// Create and start the audio engine. Returns the engine handle.
    pub fn new(buffer_size: u32) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "No audio output device found".to_string())?;

        let config = device
            .default_output_config()
            .map_err(|e| format!("Failed to get default output config: {}", e))?;

        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as usize;

        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<AudioCommand>();
        let (event_tx, event_rx) = crossbeam_channel::unbounded::<AudioEvent>();

        let shared = Arc::new(SharedState {
            playhead: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            recording: AtomicBool::new(false),
            monitoring: AtomicBool::new(false),
        });

        let shared_audio = Arc::clone(&shared);

        let tracks: Arc<parking_lot::RwLock<HashMap<TrackId, Track>>> =
            Arc::new(parking_lot::RwLock::new(HashMap::new()));
        let clips: Arc<parking_lot::RwLock<Vec<AudioClip>>> =
            Arc::new(parking_lot::RwLock::new(Vec::new()));

        let tracks_audio = Arc::clone(&tracks);
        let clips_audio = Arc::clone(&clips);

        let tempo_map: Arc<parking_lot::RwLock<TempoMap>> =
            Arc::new(parking_lot::RwLock::new(TempoMap::default()));
        let tempo_audio = Arc::clone(&tempo_map);

        // Plugin instances shared between engine thread and audio callback
        let plugins: Arc<parking_lot::RwLock<HashMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>>> =
            Arc::new(parking_lot::RwLock::new(HashMap::new()));
        let plugins_audio = Arc::clone(&plugins);

        // Build the cpal stream
        let mut stream_config: cpal::StreamConfig = config.into();
        stream_config.buffer_size = cpal::BufferSize::Fixed(buffer_size);
        let audio_sample_rate = sample_rate;

        // Pre-allocated per-track processing buffers (moved into audio callback closure)
        let mut track_buf_l = vec![0.0f32; 8192];
        let mut track_buf_r = vec![0.0f32; 8192];
        let mut monitor_temp = vec![0.0f32; 8192 * 2]; // stereo interleaved temp

        // Monitor ring buffer: input callback → output callback
        // Separate from the recording ring buffer (which goes input → engine thread)
        let monitor_ring = ringbuf::HeapRb::<f32>::new(4096 * 2); // ~4096 stereo frames
        let (monitor_prod, mut monitor_cons) = monitor_ring.split();
        let monitor_prod = Arc::new(parking_lot::Mutex::new(monitor_prod));
        let monitor_prod_audio = Arc::clone(&monitor_prod);

        let stream = device
            .build_output_stream(
                &stream_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    mix_audio(
                        data,
                        channels,
                        &shared_audio,
                        &tracks_audio,
                        &clips_audio,
                        &plugins_audio,
                        &tempo_audio,
                        audio_sample_rate,
                        &mut track_buf_l,
                        &mut track_buf_r,
                        &mut monitor_cons,
                        &mut monitor_temp,
                    );
                },
                |err| {
                    eprintln!("Audio stream error: {}", err);
                },
                None,
            )
            .map_err(|e| format!("Failed to build output stream: {}", e))?;

        stream
            .play()
            .map_err(|e| format!("Failed to start stream: {}", e))?;

        // Spawn the engine control thread
        let shared_ctrl = Arc::clone(&shared);
        let tracks_ctrl = Arc::clone(&tracks);
        let clips_ctrl = Arc::clone(&clips);
        let tempo_ctrl = Arc::clone(&tempo_map);
        let plugins_ctrl = Arc::clone(&plugins);

        std::thread::Builder::new()
            .name("resonance-engine".into())
            .spawn(move || {
                engine_thread(
                    cmd_rx,
                    event_tx,
                    shared_ctrl,
                    tracks_ctrl,
                    clips_ctrl,
                    tempo_ctrl,
                    plugins_ctrl,
                    monitor_prod_audio,
                    sample_rate,
                    buffer_size,
                );
            })
            .map_err(|e| format!("Failed to spawn engine thread: {}", e))?;

        Ok(Self {
            cmd_tx,
            event_rx,
            _stream: Some(stream),
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
}

/// Run a pactl command with a 2-second timeout. Returns stdout on success.
fn run_pactl(args: &[&str]) -> Option<String> {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::spawn(move || {
        let result = std::process::Command::new("pactl")
            .args(&args)
            .output();
        let _ = tx.send(result);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(2)) {
        Ok(Ok(output)) if output.status.success() => {
            Some(String::from_utf8_lossy(&output.stdout).to_string())
        }
        _ => None,
    }
}

/// Enumerate available PipeWire/PulseAudio input sources via `pactl`.
fn enumerate_input_devices() -> (Vec<InputDeviceInfo>, Option<String>) {
    let mut devices = Vec::new();

    let default_name = run_pactl(&["get-default-source"])
        .map(|s| s.trim().to_string());

    let short_text = run_pactl(&["list", "sources", "short"]);
    let full_text = run_pactl(&["list", "sources"]);

    if let (Some(short), Some(full)) = (short_text, full_text) {
        let mut descriptions: HashMap<String, String> = HashMap::new();
        let mut current_name = None;
        for line in full.lines() {
            let trimmed = line.trim();
            if let Some(name) = trimmed.strip_prefix("Name: ") {
                current_name = Some(name.to_string());
            } else if let Some(desc) = trimmed.strip_prefix("Description: ") {
                if let Some(name) = current_name.take() {
                    descriptions.insert(name, desc.to_string());
                }
            }
        }

        for line in short.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 2 {
                let name = parts[1].to_string();
                let description = descriptions
                    .get(&name)
                    .cloned()
                    .unwrap_or_else(|| name.clone());
                devices.push(InputDeviceInfo { name, description });
            }
        }
    }

    (devices, default_name)
}

/// Build a cpal input stream that pushes samples into ring buffer producers.
/// `rec_producer` is for recording (engine thread drains it).
/// `mon_producer` is for monitoring (audio callback reads it).
fn build_input_stream(
    source_name: Option<&str>,
    shared: Arc<SharedState>,
    mut rec_producer: Option<ringbuf::HeapProd<f32>>,
    mon_producer: Arc<parking_lot::Mutex<ringbuf::HeapProd<f32>>>,
    buffer_size: u32,
) -> Result<(cpal::Stream, u32, u16), String> {
    let _env_guard = PIPEWIRE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(name) = source_name {
        std::env::set_var("PIPEWIRE_NODE", name);
    }

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "No input device found".to_string())?;

    let config = device
        .default_input_config()
        .map_err(|e| format!("No default input config: {}", e))?;

    let sample_rate = config.sample_rate().0;
    let channels = config.channels();
    let mut stream_config: cpal::StreamConfig = config.into();
    stream_config.buffer_size = cpal::BufferSize::Fixed(buffer_size);

    let stream = device
        .build_input_stream(
            &stream_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                // Push to recording ring buffer
                if shared.recording.load(Ordering::Relaxed) {
                    if let Some(ref mut prod) = rec_producer {
                        let _ = prod.push_slice(data);
                    }
                }
                // Push to monitor ring buffer
                if shared.monitoring.load(Ordering::Relaxed) {
                    if let Some(mut prod) = mon_producer.try_lock() {
                        let _ = prod.push_slice(data);
                    }
                }
            },
            |err| {
                eprintln!("Input stream error: {}", err);
            },
            None,
        )
        .map_err(|e| format!("Failed to build input stream: {}", e))?;

    stream
        .play()
        .map_err(|e| format!("Failed to start input stream: {}", e))?;

    std::env::remove_var("PIPEWIRE_NODE");

    Ok((stream, sample_rate, channels))
}

/// Drain all available samples from the ring buffer consumer into per-track recording buffers.
fn drain_ring_to_buffers(
    consumer: &mut ringbuf::HeapCons<f32>,
    buffers: &mut HashMap<TrackId, Vec<f32>>,
    input_channels: u16,
) {
    let channels = input_channels as usize;
    let mut temp = [0.0f32; 4096];
    loop {
        let count = consumer.pop_slice(&mut temp);
        if count == 0 {
            break;
        }
        let chunk = &temp[..count];

        if channels == 2 {
            for buffer in buffers.values_mut() {
                buffer.extend_from_slice(chunk);
            }
        } else {
            let frames = chunk.len() / channels;
            for buffer in buffers.values_mut() {
                buffer.reserve(frames * 2);
                for f in 0..frames {
                    let base = f * channels;
                    let left = chunk[base];
                    let right = if channels > 1 { chunk[base + 1] } else { left };
                    buffer.push(left);
                    buffer.push(right);
                }
            }
        }
    }
}

/// Finalize recording: drain remaining samples, create clips, emit events.
fn finalize_recording(
    ring_consumer: &mut Option<ringbuf::HeapCons<f32>>,
    recording_buffers: &mut HashMap<TrackId, Vec<f32>>,
    input_channels: u16,
    input_sample_rate: u32,
    output_sample_rate: u32,
    recording_start_sample: SamplePos,
    punch_enabled: bool,
    punch_in: SamplePos,
    punch_out: SamplePos,
    next_clip_id: &mut ClipId,
    clips: &parking_lot::RwLock<Vec<AudioClip>>,
    event_tx: &Sender<AudioEvent>,
) {
    if let Some(ref mut cons) = ring_consumer {
        drain_ring_to_buffers(cons, recording_buffers, input_channels);
    }

    for (track_id, buffer) in recording_buffers.drain() {
        if buffer.is_empty() {
            continue;
        }

        let clip_id = *next_clip_id;
        *next_clip_id += 1;

        let final_data = if input_sample_rate != output_sample_rate {
            decode::linear_resample(&buffer, input_sample_rate, output_sample_rate)
        } else {
            buffer
        };

        // Trim to punch range if enabled
        let (clip_start_sample, final_data) = if punch_enabled && punch_out > punch_in {
            let total_frames = (final_data.len() / 2) as u64;
            let trim_start_frame = punch_in.saturating_sub(recording_start_sample);
            let trim_end_frame = punch_out
                .saturating_sub(recording_start_sample)
                .min(total_frames);

            if trim_start_frame >= trim_end_frame {
                continue; // Nothing in the punch range
            }

            let trim_start_idx = (trim_start_frame * 2) as usize;
            let trim_end_idx = (trim_end_frame * 2) as usize;
            (punch_in, final_data[trim_start_idx..trim_end_idx].to_vec())
        } else {
            (recording_start_sample, final_data)
        };

        let duration_samples = (final_data.len() / 2) as u64;
        let name = format!("Recording {}", clip_id);

        let clip = AudioClip {
            id: clip_id,
            track_id,
            start_sample: clip_start_sample,
            data: final_data,
            name: name.clone(),
        };
        clips.write().push(clip);

        let _ = event_tx.send(AudioEvent::RecordingFinished {
            clip_id,
            track_id,
            start_sample: clip_start_sample,
            duration_samples,
            name,
        });
    }

    *ring_consumer = None;
}

/// The engine control thread processes commands and sends events.
fn engine_thread(
    cmd_rx: Receiver<AudioCommand>,
    event_tx: Sender<AudioEvent>,
    shared: Arc<SharedState>,
    tracks: Arc<parking_lot::RwLock<HashMap<TrackId, Track>>>,
    clips: Arc<parking_lot::RwLock<Vec<AudioClip>>>,
    tempo_map: Arc<parking_lot::RwLock<TempoMap>>,
    plugins: Arc<parking_lot::RwLock<HashMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>>>,
    monitor_prod: Arc<parking_lot::Mutex<ringbuf::HeapProd<f32>>>,
    sample_rate: u32,
    buffer_size: u32,
) {
    let mut next_track_id: TrackId = 1;
    let mut next_clip_id: ClipId = 1;
    let mut next_plugin_id: PluginInstanceId = 1;
    let mut last_playhead_report = std::time::Instant::now();

    // Recording state (engine thread local)
    let mut recording_buffers: HashMap<TrackId, Vec<f32>> = HashMap::new();
    let mut recording_start_sample: SamplePos = 0;
    let mut ring_consumer: Option<ringbuf::HeapCons<f32>> = None;
    let mut _input_stream: Option<cpal::Stream> = None;
    let mut input_channels: u16 = 2;
    let mut input_sample_rate: u32 = sample_rate;

    // Punch in/out state (engine thread local)
    let mut punch_enabled: bool = false;
    let mut punch_in: SamplePos = 0;
    let mut punch_out: SamplePos = 0;

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

                    let armed_tracks: Vec<(TrackId, Option<String>)> = {
                        let tracks_guard = tracks.read();
                        tracks_guard
                            .values()
                            .filter(|t| t.record_armed)
                            .map(|t| (t.id, t.input_device_name.clone()))
                            .collect()
                    };

                    if !armed_tracks.is_empty() {
                        let source_name: Option<String> = armed_tracks
                            .iter()
                            .find_map(|(_, name)| name.clone());

                        let ring_size = 96000 * 2 * 10;
                        let ring = ringbuf::HeapRb::<f32>::new(ring_size);
                        let (prod, cons) = ring.split();
                        ring_consumer = Some(cons);

                        recording_start_sample = shared.playhead.load(Ordering::SeqCst);
                        for (track_id, _) in &armed_tracks {
                            recording_buffers.insert(
                                *track_id,
                                Vec::with_capacity(sample_rate as usize * 2 * 60),
                            );
                        }
                        shared.recording.store(true, Ordering::SeqCst);

                        match build_input_stream(source_name.as_deref(), Arc::clone(&shared), Some(prod), Arc::clone(&monitor_prod), buffer_size) {
                            Ok((stream, in_sr, in_ch)) => {
                                _input_stream = Some(stream);
                                input_sample_rate = in_sr;
                                input_channels = in_ch;

                                let _ = event_tx.send(AudioEvent::RecordingStarted {
                                    start_sample: recording_start_sample,
                                });
                            }
                            Err(e) => {
                                shared.recording.store(false, Ordering::SeqCst);
                                recording_buffers.clear();
                                ring_consumer = None;
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
                        finalize_recording(
                            &mut ring_consumer,
                            &mut recording_buffers,
                            input_channels,
                            input_sample_rate,
                            sample_rate,
                            recording_start_sample,
                            punch_enabled,
                            punch_in,
                            punch_out,
                            &mut next_clip_id,
                            &clips,
                            &event_tx,
                        );
                        _input_stream = None;
                    }
                }
                AudioCommand::Stop => {
                    let was_recording = shared.recording.load(Ordering::SeqCst);
                    shared.playing.store(false, Ordering::SeqCst);
                    shared.recording.store(false, Ordering::SeqCst);
                    shared.playhead.store(0, Ordering::SeqCst);

                    if was_recording {
                        finalize_recording(
                            &mut ring_consumer,
                            &mut recording_buffers,
                            input_channels,
                            input_sample_rate,
                            sample_rate,
                            recording_start_sample,
                            punch_enabled,
                            punch_in,
                            punch_out,
                            &mut next_clip_id,
                            &clips,
                            &event_tx,
                        );
                        _input_stream = None;
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
                    let clips = Arc::clone(&clips);
                    let thread_event_tx = event_tx.clone();
                    let clip_id = next_clip_id;
                    next_clip_id += 1;
                    let sr = sample_rate;

                    let spawn_result = std::thread::Builder::new()
                        .name("resonance-decode".into())
                        .spawn(move || {
                            match decode::decode_file(&path, sr) {
                                Ok((data, name)) => {
                                    let duration = (data.len() / 2) as u64;
                                    let clip = AudioClip {
                                        id: clip_id,
                                        track_id,
                                        start_sample,
                                        data,
                                        name: name.clone(),
                                    };
                                    clips.write().push(clip);
                                    let _ = thread_event_tx.send(AudioEvent::ClipImported {
                                        clip_id,
                                        track_id,
                                        start_sample,
                                        duration_samples: duration,
                                        name,
                                    });
                                }
                                Err(e) => {
                                    let _ = thread_event_tx.send(AudioEvent::Error(format!(
                                        "Failed to import clip: {}",
                                        e
                                    )));
                                }
                            }
                        });
                    if let Err(e) = spawn_result {
                        let _ = event_tx.send(AudioEvent::Error(format!(
                            "Failed to spawn decode thread: {}",
                            e
                        )));
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
                AudioCommand::DeleteClip { clip_id } => {
                    clips.write().retain(|c| c.id != clip_id);
                    let _ = event_tx.send(AudioEvent::ClipDeleted { clip_id });
                }
                AudioCommand::SetTrackVolume { track_id, volume } => {
                    if let Some(track) = tracks.write().get_mut(&track_id) {
                        track.volume = volume.clamp(0.0, 1.0);
                    }
                }
                AudioCommand::SetTrackMute { track_id, muted } => {
                    if let Some(track) = tracks.write().get_mut(&track_id) {
                        track.muted = muted;
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
                    // Remove plugins for this track (extract IDs before taking write lock)
                    let plugin_ids = tracks
                        .read()
                        .get(&track_id)
                        .map(|t| t.plugin_ids.clone());
                    if let Some(ids) = plugin_ids {
                        let mut plugins_guard = plugins.write();
                        for pid in ids {
                            plugins_guard.remove(&pid);
                        }
                    }
                    tracks.write().remove(&track_id);
                    clips.write().retain(|c| c.track_id != track_id);
                    recording_buffers.remove(&track_id);
                    let _ = event_tx.send(AudioEvent::TrackRemoved { track_id });
                }
                AudioCommand::SetTrackRecordArm { track_id, armed } => {
                    if let Some(track) = tracks.write().get_mut(&track_id) {
                        track.record_armed = armed;
                    }
                }
                AudioCommand::SetTrackMonitor {
                    track_id,
                    enabled,
                } => {
                    if let Some(track) = tracks.write().get_mut(&track_id) {
                        track.monitor_enabled = enabled;
                    }
                    // Update monitoring flag: true if any track has monitoring enabled
                    let any_monitoring = tracks.read().values().any(|t| t.monitor_enabled);
                    shared.monitoring.store(any_monitoring, Ordering::SeqCst);

                    // Start input stream if monitoring and no stream active
                    if any_monitoring && _input_stream.is_none() {
                        let source_name: Option<String> = {
                            let tg = tracks.read();
                            tg.values()
                                .find(|t| t.monitor_enabled)
                                .and_then(|t| t.input_device_name.clone())
                        };
                        match build_input_stream(
                            source_name.as_deref(),
                            Arc::clone(&shared),
                            None,
                            Arc::clone(&monitor_prod),
                            buffer_size,
                        ) {
                            Ok((stream, in_sr, in_ch)) => {
                                _input_stream = Some(stream);
                                input_sample_rate = in_sr;
                                input_channels = in_ch;
                            }
                            Err(e) => {
                                let _ = event_tx.send(AudioEvent::Error(format!(
                                    "Failed to start monitoring: {}", e
                                )));
                            }
                        }
                    } else if !any_monitoring && !shared.recording.load(Ordering::SeqCst) {
                        // Stop input stream if no monitoring and not recording
                        _input_stream = None;
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
                    let (devices, default_name) = enumerate_input_devices();
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

                            // Query params before moving instance into shared map
                            let params = instance.query_params();

                            plugins.write().insert(instance_id, parking_lot::Mutex::new(SyncClapInstance(instance)));

                            if let Some(track) = tracks.write().get_mut(&track_id) {
                                track.plugin_ids.push(instance_id);
                            }

                            let _ = event_tx.send(AudioEvent::PluginAdded {
                                track_id,
                                instance_id,
                                plugin_name,
                                clap_plugin_id: actual_plugin_id.clone(),
                                params,
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
                    plugins.write().remove(&instance_id);
                    let _ = event_tx.send(AudioEvent::PluginRemoved {
                        track_id,
                        instance_id,
                    });
                }
                AudioCommand::ScanPlugins => {
                    let mut scanned = Vec::new();
                    let mut scan_dirs: Vec<std::path::PathBuf> = Vec::new();
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
                            // cargo run: target/debug/ → look for ../../target/bundled/
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
                        scan_dirs.push(workspace_bundled);
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
                AudioCommand::SavePluginState { instance_id } => {
                    if let Some(mutex) = plugins.read().get(&instance_id) {
                        let data = mutex.lock().0.save_state();
                        if let Some(data) = data {
                            let _ = event_tx.send(AudioEvent::PluginStateSaved {
                                instance_id,
                                data,
                            });
                        }
                    }
                }
                AudioCommand::LoadPluginState { instance_id, data } => {
                    if let Some(mutex) = plugins.read().get(&instance_id) {
                        mutex.lock().0.reload_with_state(&data);
                    }
                }
                AudioCommand::SetPunchRange {
                    enabled,
                    punch_in: pi,
                    punch_out: po,
                } => {
                    punch_enabled = enabled;
                    punch_in = pi;
                    punch_out = po;
                }
            },
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }

        // Drain recording ring buffer into per-track buffers
        if shared.recording.load(Ordering::Relaxed) {
            if let Some(ref mut cons) = ring_consumer {
                drain_ring_to_buffers(cons, &mut recording_buffers, input_channels);
            }

            // Auto-stop recording at punch_out (keep playing)
            if punch_enabled && punch_out > punch_in {
                let current_pos = shared.playhead.load(Ordering::SeqCst);
                if current_pos >= punch_out {
                    shared.recording.store(false, Ordering::SeqCst);
                    finalize_recording(
                        &mut ring_consumer,
                        &mut recording_buffers,
                        input_channels,
                        input_sample_rate,
                        sample_rate,
                        recording_start_sample,
                        punch_enabled,
                        punch_in,
                        punch_out,
                        &mut next_clip_id,
                        &clips,
                        &event_tx,
                    );
                    _input_stream = None;
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

/// Mix audio from all active clips into the output buffer.
/// This runs on the cpal audio callback thread — must be allocation-free
/// (uses pre-allocated track_buf_l/track_buf_r).
fn mix_audio(
    data: &mut [f32],
    channels: usize,
    shared: &SharedState,
    tracks: &parking_lot::RwLock<HashMap<TrackId, Track>>,
    clips: &parking_lot::RwLock<Vec<AudioClip>>,
    plugins: &parking_lot::RwLock<HashMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>>,
    tempo_map: &parking_lot::RwLock<TempoMap>,
    sample_rate: u32,
    track_buf_l: &mut Vec<f32>,
    track_buf_r: &mut Vec<f32>,
    monitor_cons: &mut ringbuf::HeapCons<f32>,
    monitor_temp: &mut Vec<f32>,
) {
    // Zero the output buffer
    data.fill(0.0);

    let output_frames = data.len() / channels;
    let frames = output_frames.min(8192);

    // Read monitor input (always drain to keep buffer fresh, even when not playing)
    let monitor_samples = monitor_cons.pop_slice(&mut monitor_temp[..frames * 2]);
    let monitor_frames = monitor_samples / 2;

    // Drain excess samples to prevent latency accumulation
    {
        let mut discard = [0.0f32; 512];
        while monitor_cons.pop_slice(&mut discard) > 0 {}
    }

    if !shared.playing.load(Ordering::Relaxed) {
        // Even when stopped, output monitored audio for armed tracks
        if monitor_frames > 0 && shared.monitoring.load(Ordering::Relaxed) {
            let (Some(tracks_guard), Some(plugins_guard)) = (tracks.try_read(), plugins.try_read()) else {
                // Lock contended — output silence rather than block the audio thread
                return;
            };
            let any_monitor = tracks_guard.values().any(|t| t.monitor_enabled && !t.muted);

            if any_monitor {
                for track in tracks_guard.values() {
                    if !track.monitor_enabled || track.muted {
                        continue;
                    }

                    // De-interleave monitor input into track buffers
                    track_buf_l[..monitor_frames].fill(0.0);
                    track_buf_r[..monitor_frames].fill(0.0);
                    for f in 0..monitor_frames {
                        track_buf_l[f] = monitor_temp[f * 2];
                        track_buf_r[f] = monitor_temp[f * 2 + 1];
                    }

                    // Process through plugin chain
                    for &plugin_id in &track.plugin_ids {
                        if let Some(si) = plugins_guard.get(&plugin_id) {
                            if let Some(mut inst) = si.try_lock() {
                                inst.0.process(
                                    &mut track_buf_l[..monitor_frames],
                                    &mut track_buf_r[..monitor_frames],
                                    monitor_frames,
                                );
                            }
                        }
                    }

                    // Sum to output
                    let volume = track.volume;
                    for f in 0..monitor_frames {
                        let out_idx = f * channels;
                        if channels >= 2 {
                            data[out_idx] += track_buf_l[f] * volume;
                            data[out_idx + 1] += track_buf_r[f] * volume;
                        } else {
                            data[out_idx] +=
                                (track_buf_l[f] + track_buf_r[f]) * 0.5 * volume;
                        }
                    }
                }

                // Hard clip
                for sample in data.iter_mut() {
                    *sample = sample.clamp(-1.0, 1.0);
                }
            }
        }
        return;
    }

    let playhead = shared.playhead.load(Ordering::Relaxed);

    let (Some(tracks_guard), Some(clips_guard), Some(plugins_guard)) = (tracks.try_read(), clips.try_read(), plugins.try_read()) else {
        // Lock contended — output silence for this buffer rather than block the audio thread
        return;
    };

    // Per-track processing: (clips + monitor input) → plugins → volume → master
    for track in tracks_guard.values() {
        if track.muted {
            continue;
        }

        // Zero per-track buffers
        track_buf_l[..frames].fill(0.0);
        track_buf_r[..frames].fill(0.0);

        // Mix monitor input for tracks with monitoring enabled
        let mut has_audio = false;
        if track.monitor_enabled && monitor_frames > 0 {
            let mix_frames = frames.min(monitor_frames);
            for f in 0..mix_frames {
                track_buf_l[f] += monitor_temp[f * 2];
                track_buf_r[f] += monitor_temp[f * 2 + 1];
            }
            has_audio = true;
        }

        // Accumulate all clips for this track into de-interleaved track buffers
        for clip in clips_guard.iter() {
            if clip.track_id != track.id {
                continue;
            }

            let clip_frames = clip.duration_frames();
            // Compute overlap between this clip and the current output buffer
            let clip_start = clip.start_sample;
            let clip_end = clip_start + clip_frames;
            let buf_start = playhead;
            let buf_end = playhead + frames as u64;

            if buf_end <= clip_start || buf_start >= clip_end {
                continue; // No overlap
            }

            let overlap_start = buf_start.max(clip_start);
            let overlap_end = buf_end.min(clip_end);

            for timeline_frame in overlap_start..overlap_end {
                let frame_offset = (timeline_frame - buf_start) as usize;
                let clip_frame = (timeline_frame - clip_start) as usize;
                let clip_idx = clip_frame * 2;
                if clip_idx + 1 < clip.data.len() {
                    track_buf_l[frame_offset] += clip.data[clip_idx];
                    track_buf_r[frame_offset] += clip.data[clip_idx + 1];
                    has_audio = true;
                }
            }
        }

        // Process through plugin chain (even if no audio — plugins may generate tails)
        if !track.plugin_ids.is_empty() {
            for &plugin_id in &track.plugin_ids {
                if let Some(mutex) = plugins_guard.get(&plugin_id) {
                    if let Some(mut inst) = mutex.try_lock() {
                        inst.0
                            .process(&mut track_buf_l[..frames], &mut track_buf_r[..frames], frames);
                        has_audio = true;
                    }
                }
            }
        }

        if !has_audio {
            continue;
        }

        // Apply track volume and sum to master output
        let volume = track.volume;
        for frame_offset in 0..frames {
            let out_idx = frame_offset * channels;
            if channels >= 2 {
                data[out_idx] += track_buf_l[frame_offset] * volume;
                data[out_idx + 1] += track_buf_r[frame_offset] * volume;
            } else {
                data[out_idx] +=
                    (track_buf_l[frame_offset] + track_buf_r[frame_offset]) * 0.5 * volume;
            }
        }
    }

    drop(plugins_guard);

    // Metronome click synthesis
    if let Some(tm) = tempo_map.try_read() {
        if tm.metronome_enabled {
            let spb = tm.samples_per_beat(sample_rate);
            let numerator = tm.numerator as u64;
            let click_duration_samples = (sample_rate as f32 * 0.02) as u64;

            for frame_offset in 0..output_frames {
                let timeline_frame = playhead + frame_offset as u64;
                // Use round() to avoid drift: find the nearest beat boundary
                let beat_index = (timeline_frame as f64 / spb).floor();
                let beat_start = (beat_index * spb).round() as u64;
                let beat_pos = timeline_frame.saturating_sub(beat_start);

                if beat_pos < click_duration_samples {
                    let t = beat_pos as f32 / sample_rate as f32;
                    let beat_in_bar = (beat_index as u64) % numerator;
                    let freq = if beat_in_bar == 0 { 1500.0 } else { 1000.0 };
                    let amplitude = 0.3 * (-t * 200.0).exp();
                    let click = amplitude * (2.0 * std::f32::consts::PI * freq * t).sin();

                    let out_idx = frame_offset * channels;
                    if channels >= 2 {
                        data[out_idx] += click;
                        data[out_idx + 1] += click;
                    } else {
                        data[out_idx] += click;
                    }
                }
            }
        }
    }

    // Hard clip at [-1.0, 1.0]
    for sample in data.iter_mut() {
        *sample = sample.clamp(-1.0, 1.0);
    }

    // Advance playhead
    let new_playhead = playhead + output_frames as u64;
    shared.playhead.store(new_playhead, Ordering::Relaxed);
}
