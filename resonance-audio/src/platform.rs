/// Platform-specific audio device functions (PipeWire / PulseAudio).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::traits::Producer;

use crate::engine::SharedState;
use crate::types::*;

use std::sync::atomic::Ordering;

/// Serializes access to PIPEWIRE_NODE env var manipulation.
pub(crate) static PIPEWIRE_ENV_LOCK: Mutex<()> = Mutex::new(());

/// Direction of device (input vs output) for sample rate selection.
pub(crate) enum DeviceDirection {
    Input,
    Output,
}

/// Pick the best sample rate: prefer the PipeWire graph rate to avoid resampling.
/// Falls back to the default config rate if we can't determine the graph rate.
/// Works for both input and output devices.
///
/// Priority: pw-metadata graph rate > pactl sink rate > cpal default.
pub(crate) fn pick_sample_rate(
    device: &cpal::Device,
    default_config: &cpal::SupportedStreamConfig,
    direction: DeviceDirection,
) -> u32 {
    let default_rate = default_config.sample_rate().0;

    // Try pw-metadata first (authoritative graph rate), then pactl as fallback.
    let candidates = [pipewire_graph_rate(), default_sink_sample_rate()];

    for candidate in candidates.into_iter().flatten() {
        let supported = match direction {
            DeviceDirection::Output => {
                device.supported_output_configs().ok().map(|mut configs| {
                    configs.any(|c| {
                        c.min_sample_rate().0 <= candidate
                            && candidate <= c.max_sample_rate().0
                    })
                })
            }
            DeviceDirection::Input => {
                device.supported_input_configs().ok().map(|mut configs| {
                    configs.any(|c| {
                        c.min_sample_rate().0 <= candidate
                            && candidate <= c.max_sample_rate().0
                    })
                })
            }
        };
        if supported == Some(true) {
            return candidate;
        }
    }

    default_rate
}

/// Run a command with a timeout (in seconds). Returns stdout on success.
///
/// Spawns the command as a child process and polls `try_wait` in a loop.
/// If the timeout expires, the child is killed to avoid leaked processes.
fn run_command_with_timeout(cmd: &str, args: &[&str], timeout_secs: u64) -> Option<String> {
    let mut child = std::process::Command::new(cmd)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    let mut stdout = Vec::new();
                    if let Some(mut out) = child.stdout.take() {
                        use std::io::Read;
                        let _ = out.read_to_end(&mut stdout);
                    }
                    return Some(String::from_utf8_lossy(&stdout).to_string());
                }
                return None;
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    }
}

/// Run a pactl command with a 2-second timeout. Returns stdout on success.
fn run_pactl(args: &[&str]) -> Option<String> {
    run_command_with_timeout("pactl", args, 2)
}

/// Run a pw-metadata query with a 2-second timeout. Returns the parsed value on success.
fn run_pw_metadata(key: &str) -> Option<String> {
    let stdout = run_command_with_timeout("pw-metadata", &["-n", "settings", "0", key], 2)?;
    // Format: "update: id:0 key:'clock.quantum' value:'1024' type:''"
    if let Some(start) = stdout.find("value:'") {
        let rest = &stdout[start + 7..];
        if let Some(end) = rest.find('\'') {
            return rest[..end].parse::<u32>().ok().map(|v| v.to_string());
        }
    }
    None
}

/// Query PipeWire's graph sample rate via `pw-metadata`.
///
/// Prefers `clock.force-rate` (user override) when non-zero, otherwise
/// reads `clock.rate`. This reflects the actual rate the graph runs at,
/// unlike `pactl list sinks short` which can report a stale/internal rate
/// (e.g. 48000) even when the graph is running at 96000.
pub(crate) fn pipewire_graph_rate() -> Option<u32> {
    if let Some(forced) = run_pw_metadata("clock.force-rate")
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|v| *v > 0)
    {
        return Some(forced);
    }
    run_pw_metadata("clock.rate").and_then(|s| s.parse().ok())
}

/// Query the default output device's actual sample rate via pactl.
/// This matches the hardware rate, avoiding PipeWire resampling.
fn default_sink_sample_rate() -> Option<u32> {
    let sink_name = run_pactl(&["get-default-sink"])?.trim().to_string();
    let sinks = run_pactl(&["list", "sinks", "short"])?;
    for line in sinks.lines() {
        if line.contains(&sink_name) {
            // Format: <id>\t<name>\t<driver>\t<sample_spec>\t<state>
            // sample_spec e.g. "s32le 26ch 48000Hz"
            for word in line.split_whitespace() {
                if let Some(rate_str) = word.strip_suffix("Hz") {
                    return rate_str.parse().ok();
                }
            }
        }
    }
    None
}

/// Query PipeWire's effective quantum (buffer period in frames).
///
/// PipeWire exposes both `clock.quantum` (the default target) and
/// `clock.force-quantum` (the user override, set e.g. by
/// `pw-metadata 0 clock.force-quantum 64`). When a force value is
/// present and non-zero, the graph actually runs at that size — the
/// plain `clock.quantum` still reports the default (typically 1024),
/// so reading it alone gives the wrong answer and leaves the engine
/// sizing its buffers for ~21 ms instead of ~1.3 ms.
pub(crate) fn pipewire_quantum() -> Option<u32> {
    if let Some(forced) = run_pw_metadata("clock.force-quantum")
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|v| *v > 0)
    {
        return Some(forced);
    }
    run_pw_metadata("clock.quantum").and_then(|s| s.parse().ok())
}

/// Query PipeWire's maximum quantum.
pub(crate) fn pipewire_max_quantum() -> Option<u32> {
    run_pw_metadata("clock.max-quantum").and_then(|s| s.parse().ok())
}

/// Enumerate available PipeWire/PulseAudio input sources via `pactl`.
pub(crate) fn enumerate_input_devices() -> (Vec<InputDeviceInfo>, Option<String>) {
    let mut devices = Vec::new();

    let default_name = run_pactl(&["get-default-source"]).map(|s| s.trim().to_string());

    let short_text = run_pactl(&["list", "sources", "short"]);
    let full_text = run_pactl(&["list", "sources"]);

    if let (Some(short), Some(full)) = (short_text, full_text) {
        let mut descriptions: HashMap<String, String> = HashMap::new();
        let mut channel_counts: HashMap<String, u16> = HashMap::new();
        let mut current_name: Option<String> = None;
        let mut current_channels: Option<u16> = None;
        let mut current_description: Option<String> = None;
        // Walk the pactl full output as a simple state machine: we
        // accumulate Name / Description / Sample Specification lines
        // until the next Source # boundary, then commit.
        for line in full.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("Source #") {
                if let Some(name) = current_name.take() {
                    if let Some(desc) = current_description.take() {
                        descriptions.insert(name.clone(), desc);
                    }
                    if let Some(ch) = current_channels.take() {
                        channel_counts.insert(name, ch);
                    }
                }
            } else if let Some(name) = trimmed.strip_prefix("Name: ") {
                current_name = Some(name.to_string());
            } else if let Some(desc) = trimmed.strip_prefix("Description: ") {
                current_description = Some(desc.to_string());
            } else if let Some(spec) = trimmed.strip_prefix("Sample Specification: ") {
                // Format: "float32le 18ch 48000Hz" — take the token
                // ending in "ch" and parse its numeric prefix.
                if let Some(ch) = spec
                    .split_whitespace()
                    .find_map(|tok| tok.strip_suffix("ch").and_then(|n| n.parse::<u16>().ok()))
                {
                    current_channels = Some(ch);
                }
            }
        }
        // Flush the last section.
        if let Some(name) = current_name {
            if let Some(desc) = current_description {
                descriptions.insert(name.clone(), desc);
            }
            if let Some(ch) = current_channels {
                channel_counts.insert(name, ch);
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
                let channels = channel_counts.get(&name).copied().unwrap_or(0);
                devices.push(InputDeviceInfo {
                    name,
                    description,
                    channels,
                });
            }
        }
    }

    (devices, default_name)
}

/// Build a cpal input stream that pushes samples into ring buffer producers.
/// `rec_producer` is for recording (engine thread drains it).
/// `mon_producer` is for monitoring (audio callback reads it).
pub(crate) fn build_input_stream(
    source_name: Option<&str>,
    shared: Arc<SharedState>,
    mut rec_producer: Option<ringbuf::HeapProd<f32>>,
    mon_producer: Arc<parking_lot::Mutex<ringbuf::HeapProd<f32>>>,
    buf_frames: usize,
    quantum: usize,
) -> Result<(cpal::Stream, u32, u16), String> {
    let _env_guard = PIPEWIRE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(name) = source_name {
        // SAFETY: the PIPEWIRE_ENV_LOCK mutex serializes all write accesses within
        // this process, and this is only called during stream construction (not in the
        // audio callback). This is a known limitation pending a PIPEWIRE_NODE API in cpal.
        unsafe {
            std::env::set_var("PIPEWIRE_NODE", name);
        }
    }

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "No input device found".to_string())?;

    let config = device
        .default_input_config()
        .map_err(|e| format!("No default input config: {}", e))?;

    let channels = config.channels();
    let sample_rate = pick_sample_rate(&device, &config, DeviceDirection::Input);
    let mut stream_config: cpal::StreamConfig = config.into();
    stream_config.sample_rate = cpal::SampleRate(sample_rate);
    stream_config.buffer_size = cpal::BufferSize::Fixed(quantum as cpal::FrameCount);

    let _ = buf_frames; // unused once the monitor path stopped pre-converting

    let stream = device
        .build_input_stream(
            &stream_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                // Push to recording ring buffer (raw interleaved multi-channel).
                if shared.recording.load(Ordering::Relaxed) {
                    if let Some(ref mut prod) = rec_producer {
                        let written = prod.push_slice(data);
                        if written < data.len() {
                            shared.recording_overflow.store(true, Ordering::Relaxed);
                        }
                    }
                }
                // Push raw interleaved multi-channel data to the monitor
                // ring buffer. The mix callback de-interleaves per track
                // based on each track's input_port_index, so two tracks
                // can monitor different physical inputs from the same
                // soundcard simultaneously.
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

    // SAFETY: the PIPEWIRE_ENV_LOCK mutex serializes all write accesses within
    // this process, and this is only called during stream construction (not in the
    // audio callback). This is a known limitation pending a PIPEWIRE_NODE API in cpal.
    unsafe {
        std::env::remove_var("PIPEWIRE_NODE");
    }

    Ok((stream, sample_rate, channels))
}
