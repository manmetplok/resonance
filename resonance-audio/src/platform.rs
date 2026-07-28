/// Platform-specific audio device functions (PipeWire / PulseAudio).
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::traits::{Observer, Producer};

use crate::engine::SharedState;
use crate::stream_errors::{format_underrun_line, UnderrunRateLimiter};
use crate::types::*;

use std::sync::atomic::Ordering;

/// Replace ALSA's default `stderr` error handler with a no-op so the
/// startup PCM probing doesn't print things like "Cannot open device
/// /dev/dsp" or "unable to open slave" — they're benign (ALSA is just
/// walking PCM definitions in `asound.conf`) but clutter the logs.
/// Wrap behind a `Once` so reinit / multiple AudioEngine instances
/// don't reinstall the handler. Linux-only; other platforms are
/// no-ops at compile time.
#[cfg(target_os = "linux")]
pub(crate) fn silence_alsa_diagnostic_output() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    // Fixed-arity stub to install. ALSA's real handler signature has
    // C variadic args (printf-style) that stable Rust can't express,
    // so we declare a non-variadic function and cast its pointer to
    // the variadic type via a `*const ()` round-trip — same calling
    // convention, so the stack args ALSA pushes are simply ignored.
    extern "C" fn null_error_handler(
        _file: *const std::os::raw::c_char,
        _line: std::os::raw::c_int,
        _function: *const std::os::raw::c_char,
        _err: std::os::raw::c_int,
        _fmt: *const std::os::raw::c_char,
    ) {
    }
    INIT.call_once(|| {
        // SAFETY: function pointers, `*const ()`, and
        // `Option<unsafe extern "C" fn>` are all the same size on all
        // supported platforms — the niche optimization stores
        // `Some(fn)` as the function pointer itself. The installed
        // handler is non-variadic but called by ALSA as variadic;
        // since we don't read the variadic args, the System V / SysV
        // / Win64 / AArch64 ABIs all let this work harmlessly.
        unsafe {
            let ptr = null_error_handler as *const ();
            let handler: alsa_sys::snd_lib_error_handler_t = std::mem::transmute(ptr);
            alsa_sys::snd_lib_error_set_handler(handler);
        }
    });
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn silence_alsa_diagnostic_output() {}

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
    let default_rate = default_config.sample_rate();

    // Try pw-metadata first (authoritative graph rate), then pactl as fallback.
    let candidates = [pipewire_graph_rate(), default_sink_sample_rate()];

    for candidate in candidates.into_iter().flatten() {
        let supported = match direction {
            DeviceDirection::Output => device.supported_output_configs().ok().map(|mut configs| {
                configs.any(|c| c.min_sample_rate() <= candidate && candidate <= c.max_sample_rate())
            }),
            DeviceDirection::Input => device.supported_input_configs().ok().map(|mut configs| {
                configs.any(|c| c.min_sample_rate() <= candidate && candidate <= c.max_sample_rate())
            }),
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
///
/// `desired_channels` is the minimum channel count the caller needs.
/// Required to make multi-channel input work on PipeWire / ALSA: the
/// default config is almost always stereo, so picking a port past 2
/// would otherwise read past the end of a 2-channel callback buffer
/// regardless of what the underlying device offers. The function
/// walks `supported_input_configs()` for the highest channel count
/// that meets the request and clamps the stream config to it.
/// Public entry point: dispatches between the native PipeWire backend
/// (Linux only, preferred) and the cpal fallback (everywhere). Returns
/// an [`InputHandle`] so the caller doesn't have to know which path
/// won. PipeWire init failures fall through to cpal so a system
/// without a running PipeWire daemon still records (just at the
/// cpal-via-ALSA cap of two channels).
/// `capture_gate` is an optional per-stream capture enable ORed with
/// `shared.recording`: the latency ping records through its own stream
/// while the global recording flag is off (doc #260 finding #3).
/// Regular recording / monitoring streams pass `None`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_input_stream(
    source_name: Option<&str>,
    shared: Arc<SharedState>,
    mut rec_producer: Option<ringbuf::HeapProd<f32>>,
    mon_producer: Arc<parking_lot::Mutex<ringbuf::HeapProd<f32>>>,
    buf_frames: usize,
    quantum: usize,
    engine_sample_rate: u32,
    desired_channels: u16,
    capture_gate: Option<Arc<std::sync::atomic::AtomicBool>>,
) -> Result<(crate::input_handle::InputHandle, u32, u16), String> {
    #[cfg(target_os = "linux")]
    {
        match crate::input_pipewire::build(
            source_name,
            Arc::clone(&shared),
            &mut rec_producer,
            Arc::clone(&mon_producer),
            engine_sample_rate,
            quantum as u32,
            desired_channels,
            capture_gate.clone(),
        ) {
            Ok((handle, sr, ch)) => {
                return Ok((crate::input_handle::InputHandle::PipeWire(handle), sr, ch));
            }
            Err(e) => {
                eprintln!("[input] PipeWire backend failed ({e}); falling back to cpal");
            }
        }
    }
    let (stream, sr, ch) = build_input_stream_cpal(
        source_name,
        shared,
        rec_producer,
        mon_producer,
        buf_frames,
        quantum,
        engine_sample_rate,
        desired_channels,
        capture_gate,
    )?;
    Ok((crate::input_handle::InputHandle::Cpal(stream), sr, ch))
}

/// Convert a `pw_time.delay` (expressed in the time domain of the
/// graph, `rate = num/denom` seconds per tick — usually `1/graph_rate`)
/// into whole samples at the engine rate. Negative delays (possible
/// with user-configured latency offsets) clamp to 0 — the engine treats
/// I/O latency as non-negative. Pure; unit-tested (doc #260 finding
/// #13).
pub fn pw_delay_to_engine_samples(
    delay: i64,
    rate_num: u32,
    rate_denom: u32,
    engine_rate: u32,
) -> u64 {
    if delay <= 0 || rate_denom == 0 {
        return 0;
    }
    ((delay as u128 * rate_num as u128 * engine_rate as u128) / rate_denom as u128) as u64
}

/// N-channel monitor-path rate converter for the cpal fallback input
/// (doc #260 finding #21): when the device rate differs from the engine
/// rate, the monitor ring would otherwise carry device-rate frames that
/// the engine-rate consumer replays pitch-shifted and glitchy. Wraps
/// the existing stereo [`StreamingLinearResampler`] (the same one the
/// recording drain uses) over channel pairs, preserving the interleaved
/// N-channel layout. The native PipeWire input negotiates the engine
/// rate in the graph and never needs this.
///
/// The recording push is untouched — takes stay at the device rate and
/// are resampled once at drain time, exactly as before.
pub struct MonitorResampler {
    channels: usize,
    pairs: Vec<crate::decode::StreamingLinearResampler>,
    pair_in: Vec<f32>,
    pair_outs: Vec<Vec<f32>>,
    out: Vec<f32>,
}

impl MonitorResampler {
    pub fn new(source_rate: u32, target_rate: u32, channels: usize) -> Self {
        let channels = channels.max(1);
        let n_pairs = channels.div_ceil(2);
        Self {
            channels,
            pairs: (0..n_pairs)
                .map(|_| crate::decode::StreamingLinearResampler::new(source_rate, target_rate))
                .collect(),
            pair_in: Vec::new(),
            pair_outs: (0..n_pairs).map(|_| Vec::new()).collect(),
            out: Vec::new(),
        }
    }

    /// Convert one interleaved input chunk (`channels`-wide frames at
    /// the source rate) and return the converted chunk at the target
    /// rate, same channel layout. Every pair advances with the same
    /// ratio over the same frame count, so all pair outputs are equal
    /// length and re-interleave losslessly.
    pub fn process(&mut self, input: &[f32]) -> &[f32] {
        let frames = input.len() / self.channels;
        if frames == 0 {
            return &[];
        }
        for (p, resampler) in self.pairs.iter_mut().enumerate() {
            let cl = (p * 2).min(self.channels - 1);
            let cr = (p * 2 + 1).min(self.channels - 1);
            self.pair_in.clear();
            for f in 0..frames {
                let base = f * self.channels;
                self.pair_in.push(input[base + cl]);
                self.pair_in.push(input[base + cr]);
            }
            self.pair_outs[p].clear();
            resampler.process(&self.pair_in, &mut self.pair_outs[p]);
        }
        let out_frames = self.pair_outs[0].len() / 2;
        self.out.clear();
        for f in 0..out_frames {
            for c in 0..self.channels {
                let pair = &self.pair_outs[c / 2];
                self.out.push(pair[f * 2 + (c % 2)]);
            }
        }
        &self.out
    }
}

/// cpal-based input stream builder. Kept as the fallback for non-
/// Linux platforms and for Linux setups where PipeWire init fails.
#[allow(clippy::too_many_arguments)]
fn build_input_stream_cpal(
    source_name: Option<&str>,
    shared: Arc<SharedState>,
    mut rec_producer: Option<ringbuf::HeapProd<f32>>,
    mon_producer: Arc<parking_lot::Mutex<ringbuf::HeapProd<f32>>>,
    buf_frames: usize,
    quantum: usize,
    engine_sample_rate: u32,
    desired_channels: u16,
    capture_gate: Option<Arc<std::sync::atomic::AtomicBool>>,
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

    // Pick the device.
    //
    // The ALSA PCM `default` is what `cpal::default_input_device()`
    // gives us, and on most PipeWire-via-ALSA setups that PCM is
    // configured for stereo only — `set_channels(4)` then fails and
    // we get a 2-channel capture node regardless of `PIPEWIRE_NODE`.
    // The explicit ALSA PCM `pipewire` is the same plugin without
    // the channel restriction; preferring it lets the channel count
    // we asked for actually take effect.
    //
    // Order: pactl source-name match (works on any host), then
    // `pipewire` PCM, then ALSA `default`.
    let pipewire_pcm = || {
        host.input_devices().ok().and_then(|mut devs| {
            devs.find(|d| {
                d.description()
                    .map(|desc| desc.name() == "pipewire")
                    .unwrap_or(false)
            })
        })
    };
    let device = source_name
        .and_then(|name| {
            let target = name.to_ascii_lowercase();
            host.input_devices().ok().and_then(|mut devs| {
                devs.find(|d| {
                    d.description()
                        .map(|desc| {
                            let n = desc.name().to_ascii_lowercase();
                            n == target || n.contains(&target) || target.contains(&n)
                        })
                        .unwrap_or(false)
                })
            })
        })
        .or_else(pipewire_pcm)
        .or_else(|| host.default_input_device())
        .ok_or_else(|| "No input device found".to_string())?;

    let default_config = device
        .default_input_config()
        .map_err(|e| format!("No default input config: {}", e))?;

    let sample_rate = pick_sample_rate(&device, &default_config, DeviceDirection::Input);
    let default_channels = default_config.channels();
    let base_config: cpal::StreamConfig = default_config.into();

    // Two attempts: first ask for the channel count we actually need,
    // and fall back to the default if cpal / the underlying backend
    // rejects it. PipeWire's ALSA plugin honours arbitrary channel
    // counts (its `supported_input_configs` usually doesn't even
    // enumerate them), but plain ALSA / PulseAudio might not.
    let _ = buf_frames; // unused once the monitor path stopped pre-converting
    let make_callback = move |channels: u16,
                              shared: Arc<SharedState>,
                              mon_producer: Arc<parking_lot::Mutex<ringbuf::HeapProd<f32>>>,
                              mut rec_producer: Option<ringbuf::HeapProd<f32>>,
                              capture_gate: Option<Arc<std::sync::atomic::AtomicBool>>| {
        let stride = channels.max(1) as usize;
        // Monitor-path rate conversion (finding #21): the monitor ring's
        // consumer replays at the engine rate, so a device running at a
        // different rate must be converted before the push — otherwise
        // monitoring pitch-shifts and glitches. Recording pushes stay at
        // the device rate (the drain resamples them, as before).
        let mut monitor_resampler = (sample_rate != engine_sample_rate)
            .then(|| MonitorResampler::new(sample_rate, engine_sample_rate, stride));
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            let recording = shared.recording.load(Ordering::Relaxed);
            let capture = recording
                || capture_gate.as_ref().is_some_and(|g| g.load(Ordering::Relaxed));
            if capture {
                if let Some(ref mut prod) = rec_producer {
                    if recording {
                        // First push of a session latches the aligned
                        // take start (doc #260 finding #2).
                        shared.latch_recording_start();
                    }
                    // Whole frames only — a partial push on overflow
                    // would rotate the take's channels (finding #17).
                    if crate::mixer::push_recording_frames(prod, data, stride) {
                        shared.recording_overflow.store(true, Ordering::Relaxed);
                    }
                }
            }
            if shared.monitoring.load(Ordering::Relaxed) {
                let monitor: &[f32] = match monitor_resampler.as_mut() {
                    Some(rs) => rs.process(data),
                    None => data,
                };
                if let Some(mut prod) = mon_producer.try_lock() {
                    let take = crate::mixer::whole_frame_push_len(
                        monitor.len(),
                        prod.vacant_len(),
                        stride,
                    );
                    let _ = prod.push_slice(&monitor[..take]);
                }
            }
        }
    };

    // Rate-limited counter for `StreamError::BufferUnderrun` on the
    // input stream. See `stream_errors.rs` for the rationale — same
    // story as the output stream in `engine::AudioEngine::new`.
    let underrun_limiter = Arc::new(UnderrunRateLimiter::new());
    let attempt = |channels: u16,
                   shared: Arc<SharedState>,
                   mon_producer: Arc<parking_lot::Mutex<ringbuf::HeapProd<f32>>>,
                   rec_producer: Option<ringbuf::HeapProd<f32>>,
                   capture_gate: Option<Arc<std::sync::atomic::AtomicBool>>| {
        let mut cfg = base_config.clone();
        cfg.sample_rate = sample_rate;
        cfg.buffer_size = cpal::BufferSize::Fixed(quantum as cpal::FrameCount);
        cfg.channels = channels;
        let underrun_limiter = Arc::clone(&underrun_limiter);
        device.build_input_stream(
            &cfg,
            make_callback(channels, shared, mon_producer, rec_producer, capture_gate),
            move |err| match err {
                cpal::StreamError::BufferUnderrun => {
                    if let Some(report) =
                        underrun_limiter.record(std::time::Instant::now())
                    {
                        eprintln!("{}", format_underrun_line("input", &report));
                    }
                }
                other => {
                    eprintln!("Input stream error: {}", other);
                }
            },
            None,
        )
    };

    let primary_channels = desired_channels.max(default_channels);
    let (stream, channels) = match attempt(
        primary_channels,
        Arc::clone(&shared),
        Arc::clone(&mon_producer),
        rec_producer.take(),
        capture_gate.clone(),
    ) {
        Ok(s) => (s, primary_channels),
        Err(primary_err) => {
            // Fall back to the device's default channel count. The
            // recording / deinterleave layer will then clamp ports past
            // that to the last channel — same caveat as before this
            // fix, but at least monitoring of channels 1+2 still works.
            eprintln!(
                "[input] {} channels rejected ({}); falling back to {} channels",
                primary_channels, primary_err, default_channels
            );
            match attempt(default_channels, shared, mon_producer, rec_producer, capture_gate) {
                Ok(s) => (s, default_channels),
                Err(e) => {
                    return Err(format!(
                        "Failed to build input stream (requested {primary_channels}ch: {primary_err}; fell back to {default_channels}ch: {e})"
                    ));
                }
            }
        }
    };

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
