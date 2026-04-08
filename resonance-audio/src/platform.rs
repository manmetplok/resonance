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
pub(crate) fn pick_sample_rate(
    device: &cpal::Device,
    default_config: &cpal::SupportedStreamConfig,
    direction: DeviceDirection,
) -> u32 {
    let default_rate = default_config.sample_rate().0;

    // Try to read PipeWire's graph rate from pw-metadata
    if let Some(graph_rate) = default_sink_sample_rate() {
        // Verify the device actually supports this rate
        let supported = match direction {
            DeviceDirection::Output => {
                device.supported_output_configs().ok().map(|mut configs| {
                    configs.any(|c| {
                        c.min_sample_rate().0 <= graph_rate
                            && graph_rate <= c.max_sample_rate().0
                    })
                })
            }
            DeviceDirection::Input => {
                device.supported_input_configs().ok().map(|mut configs| {
                    configs.any(|c| {
                        c.min_sample_rate().0 <= graph_rate
                            && graph_rate <= c.max_sample_rate().0
                    })
                })
            }
        };
        if supported == Some(true) {
            return graph_rate;
        }
    }

    default_rate
}

/// Run a command with a timeout (in seconds). Returns stdout on success.
fn run_command_with_timeout(cmd: &str, args: &[&str], timeout_secs: u64) -> Option<String> {
    let cmd = cmd.to_string();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::spawn(move || {
        let result = std::process::Command::new(&cmd).args(&args).output();
        let _ = tx.send(result);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(timeout_secs)) {
        Ok(Ok(output)) if output.status.success() => {
            Some(String::from_utf8_lossy(&output.stdout).to_string())
        }
        _ => None,
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

/// Query the default output device's actual sample rate via pactl.
/// This matches the hardware rate, avoiding PipeWire resampling.
pub(crate) fn default_sink_sample_rate() -> Option<u32> {
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

/// Query PipeWire's current quantum (buffer period in frames).
pub(crate) fn pipewire_quantum() -> Option<u32> {
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
        std::env::set_var("PIPEWIRE_NODE", name);
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

    // Pre-allocated buffer for mono->stereo conversion in monitor path
    let mut mon_stereo_buf = vec![0.0f32; buf_frames * 2];
    let input_channels = channels;

    let stream = device
        .build_input_stream(
            &stream_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                // Push to recording ring buffer (raw -- engine thread handles channel conversion)
                if shared.recording.load(Ordering::Relaxed) {
                    if let Some(ref mut prod) = rec_producer {
                        let _ = prod.push_slice(data);
                    }
                }
                // Push to monitor ring buffer (always stereo interleaved)
                if shared.monitoring.load(Ordering::Relaxed) {
                    if let Some(mut prod) = mon_producer.try_lock() {
                        if input_channels == 2 {
                            let _ = prod.push_slice(data);
                        } else {
                            // Convert to stereo interleaved
                            let ch = input_channels as usize;
                            let frames = data.len() / ch;
                            let stereo_len = frames * 2;
                            let buf = &mut mon_stereo_buf[..stereo_len];
                            for f in 0..frames {
                                let src = f * ch;
                                let l = data[src];
                                let r = if ch > 1 { data[src + 1] } else { l };
                                buf[f * 2] = l;
                                buf[f * 2 + 1] = r;
                            }
                            let _ = prod.push_slice(buf);
                        }
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
