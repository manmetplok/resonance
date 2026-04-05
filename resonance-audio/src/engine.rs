/// The core audio engine managing tracks, clips, and the cpal output stream.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};

use crate::decode;
use crate::types::*;

/// Shared state between the engine control thread and the audio callback.
struct SharedState {
    /// Current playhead position in sample frames.
    playhead: AtomicU64,
    /// Whether playback is active.
    playing: AtomicBool,
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
                }
                AudioCommand::Pause => {
                    shared.playing.store(false, Ordering::SeqCst);
                }
                AudioCommand::Stop => {
                    shared.playing.store(false, Ordering::SeqCst);
                    shared.playhead.store(0, Ordering::SeqCst);
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
            },
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
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
