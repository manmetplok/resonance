/// The core audio engine managing tracks, clips, and the cpal output stream.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use ringbuf::traits::{Consumer, Producer, Split};

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
}

/// The audio engine.
pub struct AudioEngine {
    cmd_tx: Sender<AudioCommand>,
    event_rx: Receiver<AudioEvent>,
    _stream: Option<cpal::Stream>,
}

// cpal::Stream is not Send by default on some platforms but we manage it carefully
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

        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as usize;

        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<AudioCommand>();
        let (event_tx, event_rx) = crossbeam_channel::unbounded::<AudioEvent>();

        let shared = Arc::new(SharedState {
            playhead: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            recording: AtomicBool::new(false),
        });

        // Timeline state lives on the engine thread
        let shared_audio = Arc::clone(&shared);

        // Tracks and clips for the mixer - shared via Arc
        let tracks: Arc<parking_lot::RwLock<HashMap<TrackId, Track>>> =
            Arc::new(parking_lot::RwLock::new(HashMap::new()));
        let clips: Arc<parking_lot::RwLock<Vec<AudioClip>>> =
            Arc::new(parking_lot::RwLock::new(Vec::new()));

        let tracks_audio = Arc::clone(&tracks);
        let clips_audio = Arc::clone(&clips);

        // Build the cpal stream
        let stream_config: cpal::StreamConfig = config.into();

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

        std::thread::Builder::new()
            .name("resonance-engine".into())
            .spawn(move || {
                engine_thread(
                    cmd_rx,
                    event_tx,
                    shared_ctrl,
                    tracks_ctrl,
                    clips_ctrl,
                    sample_rate,
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

/// Enumerate available input devices. Returns (devices_list, default_device_index).
fn enumerate_input_devices() -> (Vec<InputDeviceInfo>, Option<usize>) {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|d| d.name().ok());

    let mut devices = Vec::new();
    let mut default_index = None;

    if let Ok(input_devices) = host.input_devices() {
        for (i, device) in input_devices.enumerate() {
            if let Ok(name) = device.name() {
                if Some(&name) == default_name.as_ref() {
                    default_index = Some(i);
                }
                devices.push(InputDeviceInfo { index: i, name });
            }
        }
    }
    (devices, default_index)
}

/// Build a cpal input stream that pushes samples into a ring buffer producer.
fn build_input_stream(
    device_index: usize,
    shared: Arc<SharedState>,
    mut producer: ringbuf::HeapProd<f32>,
) -> Result<(cpal::Stream, u32, u16), String> {
    let host = cpal::default_host();
    let device = host
        .input_devices()
        .map_err(|e| format!("Cannot list input devices: {}", e))?
        .nth(device_index)
        .ok_or_else(|| format!("Input device index {} not found", device_index))?;

    let config = device
        .default_input_config()
        .map_err(|e| format!("No default input config: {}", e))?;

    let sample_rate = config.sample_rate().0;
    let channels = config.channels();
    let stream_config: cpal::StreamConfig = config.into();

    let stream = device
        .build_input_stream(
            &stream_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if shared.recording.load(Ordering::Relaxed) {
                    let _ = producer.push_slice(data);
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

    Ok((stream, sample_rate, channels))
}

/// Drain all available samples from the ring buffer consumer into per-track recording buffers.
fn drain_ring_to_buffers(
    consumer: &mut ringbuf::HeapCons<f32>,
    buffers: &mut HashMap<TrackId, Vec<f32>>,
    input_channels: u16,
) {
    let mut temp = [0.0f32; 4096];
    loop {
        let count = consumer.pop_slice(&mut temp);
        if count == 0 {
            break;
        }
        let chunk = &temp[..count];
        let stereo = to_stereo_chunk(chunk, input_channels as usize);
        for buffer in buffers.values_mut() {
            buffer.extend_from_slice(&stereo);
        }
    }
}

/// Convert a chunk of interleaved multi-channel audio to stereo interleaved.
fn to_stereo_chunk(input: &[f32], channels: usize) -> Vec<f32> {
    if channels == 2 {
        return input.to_vec();
    }
    let frames = input.len() / channels;
    let mut stereo = Vec::with_capacity(frames * 2);
    for f in 0..frames {
        let base = f * channels;
        let left = input[base];
        let right = if channels > 1 { input[base + 1] } else { left };
        stereo.push(left);
        stereo.push(right);
    }
    stereo
}

/// Finalize recording: drain remaining samples, create clips, emit events.
fn finalize_recording(
    ring_consumer: &mut Option<ringbuf::HeapCons<f32>>,
    recording_buffers: &mut HashMap<TrackId, Vec<f32>>,
    input_channels: u16,
    input_sample_rate: u32,
    output_sample_rate: u32,
    recording_start_sample: SamplePos,
    next_clip_id: &mut ClipId,
    clips: &parking_lot::RwLock<Vec<AudioClip>>,
    event_tx: &Sender<AudioEvent>,
) {
    // Drain any remaining samples
    if let Some(ref mut cons) = ring_consumer {
        drain_ring_to_buffers(cons, recording_buffers, input_channels);
    }

    for (track_id, buffer) in recording_buffers.drain() {
        if buffer.is_empty() {
            continue;
        }

        let clip_id = *next_clip_id;
        *next_clip_id += 1;

        // Resample if input and output sample rates differ
        let final_data = if input_sample_rate != output_sample_rate {
            decode::linear_resample(&buffer, input_sample_rate, output_sample_rate)
        } else {
            buffer
        };

        let duration_samples = (final_data.len() / 2) as u64;
        let name = format!("Recording {}", clip_id);

        let clip = AudioClip {
            id: clip_id,
            track_id,
            start_sample: recording_start_sample,
            data: final_data,
            name: name.clone(),
        };
        clips.write().push(clip);

        let _ = event_tx.send(AudioEvent::RecordingFinished {
            clip_id,
            track_id,
            start_sample: recording_start_sample,
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
    sample_rate: u32,
) {
    let mut next_track_id: TrackId = 1;
    let mut next_clip_id: ClipId = 1;
    let mut playhead_report_counter: u32 = 0;
    let playhead_report_interval = sample_rate / 60; // ~60Hz

    // Recording state (engine thread local)
    let mut recording_buffers: HashMap<TrackId, Vec<f32>> = HashMap::new();
    let mut recording_start_sample: SamplePos = 0;
    let mut ring_consumer: Option<ringbuf::HeapCons<f32>> = None;
    let mut _input_stream: Option<cpal::Stream> = None;
    let mut input_channels: u16 = 2;
    let mut input_sample_rate: u32 = sample_rate;

    // Add a default track
    {
        let track = Track::new(next_track_id, "Track 1".to_string());
        let id = next_track_id;
        tracks.write().insert(id, track);
        let _ = event_tx.send(AudioEvent::TrackAdded { track_id: id });
        next_track_id += 1;
    }

    loop {
        // Process commands - use a short timeout to allow periodic playhead updates
        match cmd_rx.recv_timeout(std::time::Duration::from_millis(16)) {
            Ok(cmd) => match cmd {
                AudioCommand::Play => {
                    shared.playing.store(true, Ordering::SeqCst);

                    // Check if any tracks are armed for recording
                    let armed_tracks: Vec<(TrackId, Option<usize>)> = {
                        let tracks_guard = tracks.read();
                        tracks_guard
                            .values()
                            .filter(|t| t.record_armed)
                            .map(|t| (t.id, t.input_device_index))
                            .collect()
                    };

                    if !armed_tracks.is_empty() {
                        // Determine which input device to use
                        let device_idx = armed_tracks
                            .iter()
                            .find_map(|(_, idx)| *idx)
                            .unwrap_or_else(|| {
                                let (_, default_idx) = enumerate_input_devices();
                                default_idx.unwrap_or(0)
                            });

                        // Create ring buffer: 10 seconds at 48kHz stereo
                        let ring_size = 48000 * 2 * 10;
                        let ring = ringbuf::HeapRb::<f32>::new(ring_size);
                        let (prod, cons) = ring.split();
                        ring_consumer = Some(cons);

                        match build_input_stream(device_idx, Arc::clone(&shared), prod) {
                            Ok((stream, in_sr, in_ch)) => {
                                _input_stream = Some(stream);
                                input_sample_rate = in_sr;
                                input_channels = in_ch;

                                recording_start_sample =
                                    shared.playhead.load(Ordering::SeqCst);
                                for (track_id, _) in &armed_tracks {
                                    recording_buffers.insert(
                                        *track_id,
                                        Vec::with_capacity(sample_rate as usize * 2 * 60),
                                    );
                                }

                                shared.recording.store(true, Ordering::SeqCst);
                                let _ = event_tx.send(AudioEvent::RecordingStarted);
                            }
                            Err(e) => {
                                let _ = event_tx.send(AudioEvent::Error(format!(
                                    "Failed to start recording: {}",
                                    e
                                )));
                                ring_consumer = None;
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
                    match decode::decode_file(&path, sample_rate) {
                        Ok((data, name)) => {
                            let clip_id = next_clip_id;
                            next_clip_id += 1;
                            let duration = (data.len() / 2) as u64;
                            let clip = AudioClip {
                                id: clip_id,
                                track_id,
                                start_sample,
                                data,
                                name: name.clone(),
                            };
                            clips.write().push(clip);
                            let _ = event_tx.send(AudioEvent::ClipImported {
                                clip_id,
                                track_id,
                                duration_samples: duration,
                                name,
                            });
                        }
                        Err(e) => {
                            let _ = event_tx.send(AudioEvent::Error(format!(
                                "Failed to import clip: {}",
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
                    tracks.write().remove(&track_id);
                    clips.write().retain(|c| c.track_id != track_id);
                    let _ = event_tx.send(AudioEvent::TrackRemoved { track_id });
                }
                AudioCommand::SetTrackRecordArm { track_id, armed } => {
                    if let Some(track) = tracks.write().get_mut(&track_id) {
                        track.record_armed = armed;
                    }
                }
                AudioCommand::SetTrackInputDevice {
                    track_id,
                    device_index,
                } => {
                    if let Some(track) = tracks.write().get_mut(&track_id) {
                        track.input_device_index = device_index;
                    }
                }
                AudioCommand::ListInputDevices => {
                    let (devices, default_index) = enumerate_input_devices();
                    let _ = event_tx.send(AudioEvent::InputDevicesListed {
                        devices,
                        default_index,
                    });
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
        }

        // Report playhead position periodically
        if shared.playing.load(Ordering::SeqCst) {
            playhead_report_counter += 1;
            if playhead_report_counter >= playhead_report_interval / 60 {
                playhead_report_counter = 0;
                let pos = shared.playhead.load(Ordering::SeqCst);
                let _ = event_tx.send(AudioEvent::PlayheadMoved(pos));
            }
        }
    }
}

/// Mix audio from all active clips into the output buffer.
/// This runs on the cpal audio callback thread — must be allocation-free.
fn mix_audio(
    data: &mut [f32],
    channels: usize,
    shared: &SharedState,
    tracks: &parking_lot::RwLock<HashMap<TrackId, Track>>,
    clips: &parking_lot::RwLock<Vec<AudioClip>>,
) {
    // Zero the buffer first
    for sample in data.iter_mut() {
        *sample = 0.0;
    }

    if !shared.playing.load(Ordering::Relaxed) {
        return;
    }

    let playhead = shared.playhead.load(Ordering::Relaxed);
    let output_frames = data.len() / channels;

    let tracks_guard = tracks.read();
    let clips_guard = clips.read();

    for clip in clips_guard.iter() {
        let track = match tracks_guard.get(&clip.track_id) {
            Some(t) => t,
            None => continue,
        };

        if track.muted {
            continue;
        }

        let volume = track.volume;
        let clip_frames = clip.duration_frames();

        for frame_offset in 0..output_frames {
            let timeline_frame = playhead + frame_offset as u64;

            if timeline_frame < clip.start_sample || timeline_frame >= clip.start_sample + clip_frames
            {
                continue;
            }

            let clip_frame = (timeline_frame - clip.start_sample) as usize;
            let clip_idx = clip_frame * 2; // stereo interleaved

            if clip_idx + 1 >= clip.data.len() {
                continue;
            }

            let left = clip.data[clip_idx] * volume;
            let right = clip.data[clip_idx + 1] * volume;

            let out_idx = frame_offset * channels;
            if channels >= 2 {
                data[out_idx] += left;
                data[out_idx + 1] += right;
            } else {
                data[out_idx] += (left + right) * 0.5;
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
