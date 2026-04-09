# Create Plugin

This skill should be used when the user asks to "create a plugin", "add a new effect", "add a new instrument", "scaffold a plugin", or otherwise wants to build a new audio plugin for the Resonance project.

## Overview

Resonance plugins are CLAP audio plugins built with **resonance-plugin** (a thin wrapper over clack-plugin). They live in `plugins/resonance-<name>/` and are compiled as `cdylib` shared libraries that the host loads dynamically.

## Project Structure

A plugin has this layout:

```
plugins/resonance-<name>/
  Cargo.toml
  src/
    lib.rs        # Plugin struct, ResonancePlugin trait impl, export_clap!
    params.rs     # Parameter definitions (FloatParam, IntParam, BoolParam)
    dsp.rs        # DSP processing engine (optional, for complex plugins)
```

## Step-by-Step: Creating a New Plugin

### 1. Cargo.toml

```toml
[package]
name = "resonance-<name>"
version = "0.1.0"
edition = "2021"
description = "<Short description of the plugin>"

[lib]
crate-type = ["cdylib", "lib"]

[dependencies]
resonance-plugin = { path = "../../resonance-plugin" }
resonance-common = { path = "../../resonance-common" }
resonance-dsp = { path = "../../resonance-dsp" }
```

Add `resonance-dsp` only if using shared DSP primitives (delay lines, filters, LFOs, etc.).
Add `parking_lot = "0.12"` if you need `Mutex` for shared state (file paths, background loading).
Add `serde_json = "1"` if you need custom state serialization (file path persistence).

Register the plugin in the workspace `Cargo.toml` at the project root under `[workspace] members`.

### 2. Parameters (params.rs)

```rust
use resonance_plugin::*;

pub struct <Name>Params {
    pub gain: FloatParam,
    pub mix: FloatParam,
    pub bypass: BoolParam,
}

impl Default for <Name>Params {
    fn default() -> Self {
        Self {
            gain: FloatParam::new(
                "gain",     // stable ID for automation
                "Gain",     // display name
                0.0,        // default value
                FloatRange::Skewed {
                    min: -60.0,
                    max: 12.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            mix: FloatParam::new(
                "mix",
                "Mix",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            bypass: BoolParam::new("bypass", "Bypass", false),
        }
    }
}
```

**Parameter types:**
- `FloatParam::new(id, name, default, range)` - continuous values
- `IntParam::new(id, name, default, range)` - discrete integer values
- `BoolParam::new(id, name, default)` - toggles

**Range types:**
- `FloatRange::Linear { min, max }` - linear 0-1 style params (mix, width, etc.)
- `FloatRange::Skewed { min, max, factor }` - logarithmic-feeling ranges (frequency, time, gain)
  - Use `FloatRange::skew_factor(-1.5)` to `-2.0` for parameters where lower values need more resolution
- `IntRange::Linear { min, max }` - integer ranges

**Smoothing** prevents zipper noise on parameter changes:
- `SmoothingStyle::Linear(ms)` - linear ramp (most common, use 50-100ms)
- `SmoothingStyle::Logarithmic(ms)` - exponential ramp
- Call `param.smoother.set_target(param.value())` then `param.smoother.next()` per-sample in process()

**Builder methods:**
- `.with_smoother(SmoothingStyle)` - add parameter smoothing
- `.with_unit(" dB")` - display unit suffix
- `.with_value_to_string(Arc<dyn Fn(f32) -> String>)` - custom value display
- `.with_string_to_value(Arc<dyn Fn(&str) -> Option<f32>>)` - custom value parsing
- `.hidden()` - hide from the host (for internal params)

**Persisted fields** (non-parameter state like loaded file paths):
Use `Arc<Mutex<String>>` fields in the params struct and override `save_state()`/`load_state()` on the plugin.

**Array params** (e.g., per-pad or per-band):
Use a `[PadParams; N]` array and iterate all sub-params in the `params()` method. Generate unique IDs with format like `"pad_0_volume"`.

### 3. Plugin Entry Point (lib.rs)

```rust
use resonance_plugin::*;

pub mod params;
// pub mod dsp;  // if you have a separate DSP module

use params::<Name>Params;

pub struct Resonance<Name> {
    params: <Name>Params,
    sample_rate: f32,
    // ... DSP state
}

impl ResonancePlugin for Resonance<Name> {
    const CLAP_ID: &'static str = "com.resonance.<name>";
    const NAME: &'static str = "Resonance <Name>";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str = "<Short description>";
    const FEATURES: &'static [&'static str] = &["audio-effect", "stereo"];
    // Pick from: "instrument", "audio-effect", "stereo", "mono",
    // "reverb", "sampler", "drum-machine", "cabinet-simulator"

    // Stereo in/out (most common for effects)
    const INPUT_CHANNELS: Option<u32> = Some(2);
    const OUTPUT_CHANNELS: u32 = 2;
    // Set INPUT_CHANNELS to None for instruments (no audio input)
    // Set MIDI_INPUT to true for MIDI-responsive plugins

    fn new() -> Self {
        Self {
            params: <Name>Params::default(),
            sample_rate: 44100.0,
        }
    }

    fn params(&self) -> Vec<&dyn Param> {
        vec![
            &self.params.gain,
            &self.params.mix,
            &self.params.bypass,
        ]
    }

    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        self.sample_rate = sample_rate;
        // Set smoother sample rates
        self.params.gain.smoother.set_sample_rate(sample_rate);
        self.params.mix.smoother.set_sample_rate(sample_rate);
        // Reset smoothers to current values
        self.params.gain.smoother.reset(self.params.gain.value());
        self.params.mix.smoother.reset(self.params.mix.value());
        // Initialize DSP state here
        true
    }

    fn reset(&mut self) {
        // Clear delay lines, filter state, etc.
    }

    fn process(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        frames: usize,
        _events: &mut EventIterator,
    ) {
        resonance_common::flush_denormals();

        // Set smoother targets from current param values
        self.params.gain.smoother.set_target(self.params.gain.value());
        self.params.mix.smoother.set_target(self.params.mix.value());

        for i in 0..frames {
            // Read smoothed parameters per-sample
            let gain_db = self.params.gain.smoother.next();
            let mix = self.params.mix.smoother.next();

            let gain_linear = 10.0_f32.powf(gain_db / 20.0);

            let dry_l = left[i];
            let dry_r = right[i];
            let wet_l = dry_l * gain_linear; // your processing here
            let wet_r = dry_r * gain_linear;

            left[i] = dry_l * (1.0 - mix) + wet_l * mix;
            right[i] = dry_r * (1.0 - mix) + wet_r * mix;
        }
    }
}

resonance_plugin::export_clap!(Resonance<Name>);
```

### 4. MIDI Instrument Plugin Variant

For instruments that respond to MIDI (like resonance-drums):

```rust
const INPUT_CHANNELS: Option<u32> = None;  // no audio input
const OUTPUT_CHANNELS: u32 = 2;
const MIDI_INPUT: bool = true;

fn process(
    &mut self,
    left: &mut [f32],
    right: &mut [f32],
    frames: usize,
    events: &mut EventIterator,
) {
    resonance_common::flush_denormals();

    let mut next_event = events.next_event();

    for sample_id in 0..frames {
        // Process MIDI events at this sample position
        while let Some(event) = next_event {
            if event.timing() > sample_id as u32 {
                break;
            }
            match event {
                NoteEvent::NoteOn { note, velocity, .. } => {
                    // Trigger voice/sample
                }
                NoteEvent::NoteOff { note, .. } => {
                    // Release voice
                }
                NoteEvent::Choke { note, .. } => {
                    // Kill voice immediately
                }
            }
            next_event = events.next_event();
        }

        // Render one stereo frame
        let (out_l, out_r) = self.render_sample();
        left[sample_id] = out_l;
        right[sample_id] = out_r;
    }
}
```

### 5. Background File Loading (amp/IR pattern)

For plugins that load files (models, IRs, samples) in the background:

```rust
use parking_lot::Mutex;
use std::sync::Arc;

pub struct ResonanceMyPlugin {
    params: MyParams,
    active_model: Option<Box<MyModel>>,
    model_mailbox: Arc<Mutex<Option<Box<MyModel>>>>,
}

impl ResonancePlugin for ResonanceMyPlugin {
    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        let path = self.params.model_path.lock().clone();
        if !path.is_empty() {
            // Blocking load on init
            let mailbox = self.model_mailbox.clone();
            let handle = std::thread::spawn(move || {
                if let Ok(model) = load_model(&path) {
                    *mailbox.lock() = Some(model);
                }
            });
            let _ = handle.join();
            self.active_model = self.model_mailbox.lock().take();
        }
        true
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32], frames: usize, _events: &mut EventIterator) {
        // Check mailbox for newly loaded model
        if let Some(mut guard) = self.model_mailbox.try_lock() {
            if guard.is_some() {
                self.active_model = guard.take();
            }
        }

        // Detect file change param → spawn background load
        let idx = self.params.file_select.value();
        if idx != self.last_index {
            self.last_index = idx;
            let mailbox = self.model_mailbox.clone();
            let file_list = self.params.file_list.clone();
            std::thread::spawn(move || {
                let list = file_list.lock();
                if let Some(path) = list.get(idx as usize) {
                    if let Ok(model) = load_model(path) {
                        *mailbox.lock() = Some(model);
                    }
                }
            });
        }

        // Process with active model...
    }

    // Custom state to persist file path
    fn save_state(&self) -> Vec<u8> {
        let mut json = resonance_plugin::state::params_to_json(&self.params());
        json["model_path"] = serde_json::Value::String(self.params.model_path.lock().clone());
        serde_json::to_vec(&json).unwrap_or_default()
    }

    fn load_state(&mut self, data: &[u8]) -> bool {
        if let Ok(state) = serde_json::from_slice::<serde_json::Value>(data) {
            resonance_plugin::state::load_params_from_json(&self.params(), &state);
            if let Some(path) = state.get("model_path").and_then(|v| v.as_str()) {
                *self.params.model_path.lock() = path.to_string();
            }
            true
        } else {
            false
        }
    }

    // Report latency if applicable
    fn latency_samples(&self) -> u32 {
        if self.active_model.is_some() { BLOCK_SIZE as u32 } else { 0 }
    }
}
```

## Common Libraries

### resonance-common (`../../resonance-common`)
Shared utilities used across all plugins:
- `flush_denormals()` - **Always call at the start of `process()`**. Sets CPU flags (FTZ/DAZ on x86) to prevent denormal floats from tanking performance.
- `decode_wav_stereo(bytes, target_sr) -> Vec<f32>` - decode WAV to interleaved stereo samples
- `decode_wav_channels(bytes, target_sr) -> WavChannels` - decode WAV to separate L/R channels
- `linear_resample_mono/stereo(samples, from_rate, to_rate)` - sample rate conversion
- `scan_directory(dir, extension) -> Vec<String>` - list files for file-browser parameters

### resonance-dsp (`../../resonance-dsp`)
Reusable DSP building blocks (zero dependencies):
- `DelayLine` - power-of-2 circular buffer. `push(sample)`, `tap(delay)` for integer delay, `tap_linear(delay_f32)` for fractional/modulated delay.
- `OnePole` - single-pole lowpass filter. `set_cutoff(freq, sample_rate)`, `process(sample) -> f32`. Used for damping, smoothing.
- `Lfo` - sine wave oscillator. `set_rate(hz, sample_rate)`, `next() -> f32`. Used for modulation effects.
- `constant_power_pan(pan) -> (f32, f32)` - equal-power stereo panning (pan: -1.0=left, 1.0=right).
- `SimpleRng` - deterministic xorshift32 PRNG. `new(seed)`, `next_u32()`. For randomized delay times, diffusion.

### resonance-plugin (`../../resonance-plugin`)
The plugin framework. Key types:
- `ResonancePlugin` trait - the main plugin trait to implement
- `FloatParam`, `IntParam`, `BoolParam` - parameter types with atomic thread-safe values
- `FloatRange`, `IntRange` - parameter range definitions
- `Smoother`, `SmoothingStyle` - per-sample parameter smoothing
- `EventIterator`, `NoteEvent` - MIDI note event handling
- `Param` trait - common interface for parameter enumeration
- `formatters::*` - value display formatters (dB, percentage, rounded)
- `state::params_to_json()`, `state::load_params_from_json()` - JSON state serialization
- `export_clap!()` macro - generates CLAP plugin entry point

### Additional dependencies (add as needed)
- `parking_lot = "0.12"` - fast Mutex/RwLock (use for shared state, file paths, background loading)
- `rustfft = "6"` - FFT for convolution or spectral processing
- `serde = { version = "1", features = ["derive"] }` + `serde_json = "1"` - for loading config files and custom state

## Key Patterns and Rules

1. **Always call `resonance_common::flush_denormals()`** at the top of `process()`.
2. **Smooth params per-sample**: call `param.smoother.set_target(param.value())` before the sample loop, then `param.smoother.next()` per sample. Exception: expensive params that recalculate internal state (like reverb size) can be read once per block.
3. **Set smoother sample rates in `initialize()`**: call `param.smoother.set_sample_rate(sr)` and `param.smoother.reset(param.value())` for each smoothed param.
4. **No allocations in `process()`**. Pre-allocate all buffers in `initialize()` or `new()`.
5. **DSP state initialization** goes in `initialize()` (called with sample rate), not `new()`.
6. **State clearing** goes in `reset()` (called when playback restarts).
7. **Dry/wet mixing** pattern: `output = dry * (1.0 - mix) + wet * mix`.
8. **Stereo processing**: use `left[i]` and `right[i]` for direct sample access. For instruments (no input), buffers are zeroed before `process()` is called.
9. **Background loading**: use `Arc<Mutex<Option<T>>>` mailbox pattern with `std::thread::spawn`. Check with `try_lock()` in process(). Block with `.join()` in initialize().
10. **Custom state persistence**: override `save_state()` / `load_state()` to include extra fields (file paths) alongside params in JSON.
11. **Latency reporting**: override `fn latency_samples(&self) -> u32` if the plugin introduces latency.

## Registering in the Workspace

Add the plugin to the root `Cargo.toml`:

```toml
[workspace]
members = [
    # ...existing members...
    "plugins/resonance-<name>",
]
```

## Building

```bash
# Build a single plugin
cargo build --release -p resonance-<name>

# Bundle all plugins as .clap files
./scripts/bundle.sh
```

The compiled plugin is `target/release/libresonance_<name>.so`. The bundle script copies it to `target/bundled/resonance-<name>.clap`. The host discovers plugins in `target/bundled/`, `~/.clap/`, and `/usr/lib/clap/`.
