# Create Plugin

This skill should be used when the user asks to "create a plugin", "add a new effect", "add a new instrument", "scaffold a plugin", or otherwise wants to build a new audio plugin for the Resonance project.

## Overview

Resonance plugins are CLAP audio plugins built with **nih-plug**. They live in `plugins/resonance-<name>/` and are compiled as `cdylib` shared libraries that the host loads dynamically.

## Project Structure

A plugin has this layout:

```
plugins/resonance-<name>/
  Cargo.toml
  src/
    lib.rs        # Plugin struct, Plugin + ClapPlugin trait impls, nih_export_clap!
    params.rs     # Parameter definitions using #[derive(Params)]
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
nih_plug = { git = "https://github.com/robbert-vdh/nih-plug.git", features = ["assert_process_allocs", "standalone"] }
parking_lot = "0.12"
resonance-common = { path = "../../resonance-common" }
resonance-dsp = { path = "../../resonance-dsp" }
```

Add `resonance-dsp` only if using shared DSP primitives (delay lines, filters, LFOs, etc.).

Register the plugin in the workspace `Cargo.toml` at the project root under `[workspace] members`.

### 2. Parameters (params.rs)

```rust
use nih_plug::prelude::*;

#[derive(Params)]
pub struct <Name>Params {
    #[id = "gain"]
    pub gain: FloatParam,

    #[id = "mix"]
    pub mix: FloatParam,

    #[id = "bypass"]
    pub bypass: BoolParam,
}

impl Default for <Name>Params {
    fn default() -> Self {
        Self {
            gain: FloatParam::new(
                "Gain",
                0.0, // default in dB
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
                "Mix",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            bypass: BoolParam::new("Bypass", false),
        }
    }
}
```

**Parameter types:**
- `FloatParam` - continuous values with ranges
- `IntParam` - discrete integer values
- `BoolParam` - toggles

**Range types:**
- `FloatRange::Linear { min, max }` - linear 0-1 style params (mix, width, etc.)
- `FloatRange::Skewed { min, max, factor }` - logarithmic-feeling ranges (frequency, time, gain)
  - Use `FloatRange::skew_factor(-1.5)` to `-2.0` for parameters where lower values need more resolution
  - Use `FloatRange::gain_skew_factor(-24.0, 0.5)` for gain params

**Smoothing** prevents zipper noise on parameter changes:
- `SmoothingStyle::Linear(ms)` - linear ramp (most common, use 50-100ms)
- `SmoothingStyle::Logarithmic(ms)` - exponential ramp
- Read smoothed values per-sample with `param.smoothed.next()`

**Persisted fields** (non-parameter state like loaded file paths):
```rust
#[persist = "ir_path"]
pub ir_path: Arc<Mutex<Option<String>>>,
```

**Nested/array params** (e.g., per-voice or per-band):
```rust
#[nested(array, group = "Band")]
pub bands: [BandParams; 4],
```

### 3. Plugin Entry Point (lib.rs)

```rust
use nih_plug::prelude::*;
use std::sync::Arc;

pub mod params;
// pub mod dsp;  // if you have a separate DSP module

use params::<Name>Params;

pub struct Resonance<Name> {
    params: Arc<<Name>Params>,
    sample_rate: f32,
    // ... DSP state
}

impl Default for Resonance<Name> {
    fn default() -> Self {
        Self {
            params: Arc::new(<Name>Params::default()),
            sample_rate: 44100.0,
        }
    }
}

impl Plugin for Resonance<Name> {
    const NAME: &'static str = "Resonance <Name>";
    const VENDOR: &'static str = "Resonance";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // Stereo in/out (most common for effects)
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
    ];

    // Set to MidiConfig::Basic for instrument/MIDI-responsive plugins
    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        // Initialize DSP state here
        true
    }

    fn reset(&mut self) {
        // Clear delay lines, filter state, etc.
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        resonance_common::flush_denormals();

        for mut channel_samples in buffer.iter_samples() {
            // Read smoothed parameters per-sample
            let gain = self.params.gain.smoothed.next();
            let mix = self.params.mix.smoothed.next();

            let gain_linear = nih_plug::util::db_to_gain(gain);

            for sample in channel_samples.iter_mut() {
                let dry = *sample;
                let wet = dry * gain_linear; // your processing here
                *sample = dry * (1.0 - mix) + wet * mix;
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for Resonance<Name> {
    const CLAP_ID: &'static str = "com.resonance.<name>";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("<Short description>");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        // Pick from: Instrument, AudioEffect, Analyzer, Stereo, Mono,
        // Reverb, Delay, Distortion, Compressor, Limiter, EQ, Filter,
        // Chorus, Flanger, Phaser, Drum, Sampler, Synthesizer, etc.
    ];
}

nih_export_clap!(Resonance<Name>);
```

### 4. MIDI Instrument Plugin Variant

For instruments that respond to MIDI (like resonance-drums):

```rust
const MIDI_INPUT: MidiConfig = MidiConfig::Basic;

const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
    AudioIOLayout {
        // No audio input for instruments
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    },
];

fn process(
    &mut self,
    buffer: &mut Buffer,
    _aux: &mut AuxiliaryBuffers,
    context: &mut impl ProcessContext<Self>,
) -> ProcessStatus {
    resonance_common::flush_denormals();

    // Process MIDI events
    while let Some(event) = context.next_event() {
        match event {
            NoteEvent::NoteOn { note, velocity, .. } => {
                // Trigger voice/sample
            }
            NoteEvent::NoteOff { note, .. } => {
                // Release voice
            }
            _ => {}
        }
    }

    // Render audio into buffer
    for mut channel_samples in buffer.iter_samples() {
        let (out_l, out_r) = self.render_sample();
        if let Some(s) = channel_samples.get_mut(0) { *s = out_l; }
        if let Some(s) = channel_samples.get_mut(1) { *s = out_r; }
    }

    ProcessStatus::Normal
}
```

## Common Libraries

### resonance-common (`../../resonance-common`)
Shared utilities used across all plugins:
- `flush_denormals()` - **Always call at the start of `process()`**. Sets CPU flags (FTZ/DAZ on x86) to prevent denormal floats from tanking performance.
- `decode_wav_stereo(bytes) -> Vec<f32>` - decode WAV to interleaved stereo samples
- `decode_wav_channels(bytes) -> WavChannels` - decode WAV to separate L/R channels
- `linear_resample_mono/stereo(samples, from_rate, to_rate)` - sample rate conversion
- `scan_directory(dir, extension) -> Vec<String>` - list files for file-browser parameters

### resonance-dsp (`../../resonance-dsp`)
Reusable DSP building blocks:
- `DelayLine` - power-of-2 circular buffer. `push(sample)`, `tap(delay)` for integer delay, `tap_linear(delay_f32)` for fractional/modulated delay.
- `OnePole` - single-pole lowpass filter. `set_cutoff(freq, sample_rate)`, `process(sample) -> f32`. Used for damping, smoothing.
- `Lfo` - sine wave oscillator. `set_rate(hz, sample_rate)`, `next() -> f32`. Used for modulation effects.
- `constant_power_pan(pan) -> (f32, f32)` - equal-power stereo panning (pan: 0.0=left, 1.0=right).
- `SimpleRng` - deterministic xorshift32 PRNG. `new(seed)`, `next_u32()`. For randomized delay times, diffusion.

### nih_plug (`nih_plug::prelude::*`)
The plugin framework. Key utilities beyond the trait:
- `nih_plug::util::db_to_gain(db)` / `gain_to_db(gain)` - dB conversion
- `formatters::v2s_f32_rounded(decimals)` - value-to-string formatter
- `formatters::v2s_f32_percentage(decimals)` / `s2v_f32_percentage()` - percent formatters
- `nih_log!()` / `nih_debug_assert!()` - logging and debug assertions

### Additional dependencies (add as needed)
- `parking_lot = "0.12"` - fast Mutex/RwLock (use for shared state between params and DSP)
- `atomic_float = "1"` - atomic f32 for lock-free parameter sharing
- `rustfft = "6"` - FFT for convolution or spectral processing
- `serde = { version = "1", features = ["derive"] }` + `serde_json = "1"` - for loading model/config files
- `crossbeam-channel = "0.5"` - multi-producer multi-consumer channels for background tasks

## Key Patterns and Rules

1. **Always call `resonance_common::flush_denormals()`** at the top of `process()`.
2. **Read smoothed params per-sample** using `param.smoothed.next()` inside the sample loop to avoid zipper noise. Exception: expensive params that recalculate internal state (like reverb size) can be read once per block before the loop.
3. **No allocations in `process()`**. Pre-allocate all buffers in `initialize()` or `Default`. The `assert_process_allocs` feature in nih_plug will catch violations in debug builds.
4. **DSP state initialization** goes in `initialize()` (called with sample rate), not `Default`.
5. **State clearing** goes in `reset()` (called when playback restarts).
6. **Dry/wet mixing** pattern: `output = dry * (1.0 - mix) + wet * mix`.
7. **Stereo processing**: iterate with `buffer.iter_samples()`, access channels via `channel_samples.get_mut(0)` (left) and `channel_samples.get_mut(1)` (right). Always handle mono gracefully.
8. **Background loading** (files, models): use `#[persist]` fields with `Arc<Mutex<>>` and nih_plug's task executor, or a separate thread with `parking_lot::Mutex`.

## Registering in the Workspace

Add the plugin to the root `Cargo.toml`:

```toml
[workspace]
members = [
    # ...existing plugins...
    "plugins/resonance-<name>",
]
```

## Building

```bash
cargo build --release -p resonance-<name>
```

The compiled `.clap` plugin will be in `target/release/`. The host discovers plugins by path and loads them via `libloading`.
