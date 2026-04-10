# Create Plugin

This skill should be used when the user asks to "create a plugin", "add a new effect", "add a new instrument", "scaffold a plugin", "add a plugin editor", "add a GUI to a plugin", or otherwise wants to build a new audio plugin for the Resonance project.

## Overview

Resonance plugins are CLAP audio plugins built with **resonance-plugin** (a thin wrapper over clack-plugin). They live in `plugins/resonance-<name>/` and are compiled as `cdylib` shared libraries that the host loads dynamically.

A plugin can optionally ship with its own **editor**: a floating Wayland window driven by `egui`, hosted inside the plugin dylib via `wayland-plugin-gui`, exposed to the host through `CLAP_EXT_GUI`. Editors can read live audio-thread state (LFO phases, envelope levels, oscilloscope output) via a lock-free `Arc<VizState>` shared between the audio thread and the editor thread. See **Step 6** for the full editor authoring guide and `plugins/resonance-wavetable/` for the canonical reference implementation.

## Project Structure

A plugin has this layout:

```
plugins/resonance-<name>/
  Cargo.toml
  src/
    lib.rs        # Plugin struct, ResonancePlugin trait impl, export_clap!
    params.rs     # Parameter definitions (FloatParam, IntParam, BoolParam)
    dsp.rs        # DSP processing engine (optional, for complex plugins)
    viz.rs        # OPTIONAL: audio→UI shared state (only if you have an editor)
    editor/       # OPTIONAL: egui-based plugin editor (CLAP_EXT_GUI)
      mod.rs
      ...
  examples/
    editor_standalone.rs  # OPTIONAL: harness for iterating on the editor
```

Plugins can be **headless** (just DSP + params, host renders sliders) or come with their own **editor** that opens as a floating Wayland window via `CLAP_EXT_GUI`. The editor lives in the same `cdylib` as the plugin and is gated behind a Cargo feature so headless builds stay small. See **Step 6: Optional Plugin Editor** below.

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

If your plugin will have its own editor window, also add the editor feature gate (see **Step 6** for the full setup):

```toml
[features]
default = ["editor"]
editor = ["dep:wayland-plugin-gui", "dep:egui"]

[dependencies]
# ...
wayland-plugin-gui = { path = "../../wayland-plugin-gui", optional = true }
egui = { version = "0.34", features = ["default_fonts"], default-features = false, optional = true }
```

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

### 6. Optional: Plugin Editor (GUI)

If you want your plugin to ship with its own editor window — sliders, knobs, oscilloscopes, custom visualisations — implement `editor_factory()` on the plugin and put the egui UI inside the same `.clap` dylib. The DAW shows an **Open Editor** button in the mixer for any plugin that reports `has_gui`, and the host opens a floating Wayland window via `CLAP_EXT_GUI`.

The editor stack:

```
ResonanceMyPlugin
  ├─ Arc<MyParams>          (shared with editor thread, atomic-backed)
  ├─ Arc<MyVizState>        (audio→UI lock-free state)
  └─ editor_factory() → Arc<dyn EditorFactory>
        └─ create() → Box<dyn PluginEditor>
              └─ wayland_plugin_gui::Editor
                    └─ runs on its own thread
                          └─ MyEditorApp::ui(&mut egui::Ui)  ← your UI lives here
```

`wayland-plugin-gui` is the runtime that hosts an egui UI in a floating top-level Wayland window from inside a plugin dylib. It owns its own thread with its own `wl_display` connection, an EGL context on a `wl_egl_window`, and `egui_glow` for rendering. Plugin authors only need to implement the `EditorApp` trait — windowing, input, HiDPI, and rendering are all handled inside the runtime.

**Limitations:**
- **Wayland only.** No X11/Cocoa/Win32 backends yet. KDE/GNOME/Hyprland all work.
- **Floating only.** No `set_parent` embedding (Wayland has no XEmbed). The editor is its own top-level window the user can position freely.

#### 6a. Cargo.toml setup

```toml
[features]
# editor on by default so the shipped cdylib has a GUI; --no-default-features
# gives a headless build for tests.
default = ["editor"]
editor = ["dep:wayland-plugin-gui", "dep:egui"]

[dependencies]
resonance-plugin = { path = "../../resonance-plugin" }
resonance-common = { path = "../../resonance-common" }
resonance-dsp = { path = "../../resonance-dsp" }

# Editor deps — only pulled in when the `editor` feature is on.
wayland-plugin-gui = { path = "../../wayland-plugin-gui", optional = true }
# default_fonts is REQUIRED — without it text has zero-width metrics and
# every label/widget collapses to 0 size.
egui = { version = "0.34", features = ["default_fonts"], default-features = false, optional = true }

[[example]]
name = "editor_standalone"
path = "examples/editor_standalone.rs"
required-features = ["editor"]
```

#### 6b. Make params shareable: `Arc<MyParams>`

The audio thread reads/writes params via `&MyParams`; the editor thread does the same from another thread. Wrap params in `Arc<MyParams>` in the plugin struct so both threads can hold a reference. All `FloatParam`/`IntParam`/`BoolParam` use atomic storage internally so concurrent `&MyParams` access is safe — no `Mutex` needed.

```rust
use std::sync::Arc;

pub struct ResonanceMyPlugin {
    params: Arc<MyParams>,        // <-- Arc, not bare struct
    viz: Arc<MyVizState>,         // see step 6c
    engine: MyEngine,
}

impl ResonancePlugin for ResonanceMyPlugin {
    fn new() -> Self {
        Self {
            params: Arc::new(MyParams::new()),
            viz: Arc::new(MyVizState::new()),
            engine: MyEngine::new(),
        }
    }

    fn param(&self, index: usize) -> &dyn Param {
        self.params.param_at(index)
    }

    // ... rest of process()/initialize() etc. just deref Arc as &MyParams
}
```

#### 6c. Audio→UI bridge: `viz.rs`

For live visualisations (LFO phase markers, envelope playheads, oscilloscope, modulated values), the audio thread needs to publish state at audio-block boundaries that the editor thread can read tear-free at ~60Hz. The pattern:

- **Scalar values** (env levels, LFO phases, post-mod cutoff): `AtomicU32` storing `f32::to_bits()`. Relaxed loads/stores, safe to write from audio and read from UI.
- **Sample buffers** (oscilloscope frame): seq-lock'd double buffer — bump the seq counter to odd, write the front buffer, bump to even. Reader retries the snapshot until two consecutive reads of the seq match.

Skeleton:

```rust
// src/viz.rs
use std::sync::atomic::{AtomicU32, Ordering};

pub const SCOPE_FRAMES: usize = 256;
pub const SCOPE_LEN: usize = SCOPE_FRAMES * 2; // stereo interleaved

pub struct MyVizState {
    pub lfo_phase: AtomicU32,           // f32 in 0..1
    pub env_value: AtomicU32,           // f32 in 0..1
    pub filter_cutoff_live: AtomicU32,  // f32 Hz (post-modulation)
    pub scope_seq: AtomicU32,
    pub scope_front: [AtomicU32; SCOPE_LEN],
}

impl MyVizState {
    pub fn new() -> Self { /* ... initialize all atomics ... */ }

    // -- audio thread writers ----------------------------------------------
    pub fn store_lfo_phase(&self, v: f32) {
        self.lfo_phase.store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn store_env_value(&self, v: f32) {
        self.env_value.store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn publish_scope(&self, samples: &[f32; SCOPE_LEN]) {
        self.scope_seq.fetch_add(1, Ordering::Release); // odd = mid-write
        for (dst, src) in self.scope_front.iter().zip(samples.iter()) {
            dst.store(src.to_bits(), Ordering::Relaxed);
        }
        self.scope_seq.fetch_add(1, Ordering::Release); // even = published
    }

    // -- UI thread reader (called via read_snapshot) -----------------------
    pub fn read_snapshot(&self) -> VizSnapshot {
        let mut scope = [0.0f32; SCOPE_LEN];
        loop {
            let seq_before = self.scope_seq.load(Ordering::Acquire);
            if seq_before & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            for (dst, src) in scope.iter_mut().zip(self.scope_front.iter()) {
                *dst = f32::from_bits(src.load(Ordering::Relaxed));
            }
            let seq_after = self.scope_seq.load(Ordering::Acquire);
            if seq_before == seq_after { break; }
            std::hint::spin_loop();
        }
        VizSnapshot {
            lfo_phase: f32::from_bits(self.lfo_phase.load(Ordering::Relaxed)),
            env_value: f32::from_bits(self.env_value.load(Ordering::Relaxed)),
            filter_cutoff_live: f32::from_bits(self.filter_cutoff_live.load(Ordering::Relaxed)),
            scope_samples: scope,
        }
    }
}

#[derive(Clone)]
pub struct VizSnapshot {
    pub lfo_phase: f32,
    pub env_value: f32,
    pub filter_cutoff_live: f32,
    pub scope_samples: [f32; SCOPE_LEN],
}
```

In the audio engine, publish once per `process()` block (NOT per sample — the atomic stores are cheap but they aren't free, and the UI only needs ~60Hz updates):

```rust
fn process(&mut self, left: &mut [f32], right: &mut [f32], frames: usize, events: &mut EventIterator<'_>) {
    resonance_common::flush_denormals();
    // ... per-sample render loop fills `left`/`right` and `scope_collector` ...
    self.engine.publish_viz(&self.viz);   // once per block
}
```

For multi-voice instruments, pick a "representative voice" (most recent or loudest) at publish time and copy its current envelope/LFO/filter values into the atomics. When no voices are active, hold the previous values so the UI doesn't snap to zero between notes.

A two-thread atomicity test belongs in `#[cfg(test)] mod tests`:

```rust
#[test]
fn viz_bridge_no_tearing() {
    // spawn writer + reader threads, verify pair invariants on the scope
    // buffer (e.g. each (L, R) pair satisfies L == -R), and assert the
    // sample counter is monotonic.
}
```

#### 6d. The editor module: `editor/mod.rs`

```rust
// src/editor/mod.rs
use std::sync::Arc;
use resonance_plugin::gui::{EditorFactory, PluginEditor};
use wayland_plugin_gui::{egui, Editor as RuntimeEditor, EditorApp, EditorOptions};

use crate::params::MyParams;
use crate::viz::{MyVizState, VizSnapshot};

// ----- Factory: held by the plugin, asked to create editors on demand ------

pub struct MyEditorFactory {
    params: Arc<MyParams>,
    viz: Arc<MyVizState>,
}

impl MyEditorFactory {
    pub fn new(params: Arc<MyParams>, viz: Arc<MyVizState>) -> Self {
        Self { params, viz }
    }
}

impl EditorFactory for MyEditorFactory {
    fn supports(&self, api: &str, is_floating: bool) -> bool {
        is_floating && api == "wayland"
    }
    fn preferred(&self) -> Option<(&'static str, bool)> {
        Some(("wayland", true))
    }
    fn preferred_size(&self) -> (u32, u32) {
        (960, 560)
    }
    fn create(&self, api: &str, is_floating: bool) -> Option<Box<dyn PluginEditor>> {
        if !self.supports(api, is_floating) { return None; }
        let app = MyEditorApp::new(self.params.clone(), self.viz.clone());
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance MyPlugin".to_string(),
                initial_size: (960, 560),
                min_size: (640, 400),
                resizable: true,
            },
        ).ok()?;
        Some(Box::new(EditorHandle { runtime: Some(runtime), size: (960, 560) }))
    }
}

// ----- Handle: bridges wayland-plugin-gui::Editor to resonance_plugin::PluginEditor

struct EditorHandle {
    runtime: Option<RuntimeEditor>,
    size: (u32, u32),
}

impl PluginEditor for EditorHandle {
    fn show(&mut self) { if let Some(r) = &self.runtime { r.show(); } }
    fn hide(&mut self) { if let Some(r) = &self.runtime { r.hide(); } }
    fn size(&self) -> (u32, u32) { self.size }
    fn set_size(&mut self, w: u32, h: u32) -> bool {
        if let Some(r) = &self.runtime {
            if r.set_size(w, h).is_ok() { self.size = (w, h); return true; }
        }
        false
    }
    fn can_resize(&self) -> bool { self.runtime.as_ref().map(|r| r.is_resizable()).unwrap_or(false) }
}

impl Drop for EditorHandle {
    fn drop(&mut self) {
        if let Some(r) = self.runtime.take() { r.destroy(); }
    }
}

// ----- The egui app that runs on the editor thread -------------------------

pub struct MyEditorApp {
    params: Arc<MyParams>,
    viz: Arc<MyVizState>,
    snapshot: VizSnapshot,
}

impl MyEditorApp {
    pub fn new(params: Arc<MyParams>, viz: Arc<MyVizState>) -> Self {
        let snapshot = viz.read_snapshot();
        Self { params, viz, snapshot }
    }
}

impl EditorApp for MyEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        // Refresh audio-thread state for this frame.
        self.snapshot = self.viz.read_snapshot();
        // Drive continuous ~60 fps so live views animate.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));

        // Lay out using show_inside on Panels (NOT show; show is deprecated).
        egui::Panel::top("topbar").exact_size(32.0).show_inside(ui, |ui| {
            ui.heading("MY PLUGIN");
        });
        egui::CentralPanel::default().show_inside(ui, |ui| {
            // Read params via atomic getters; write via set_value/set_plain.
            let mut gain = self.params.gain.value();
            if ui.add(egui::Slider::new(&mut gain, 0.0..=1.0).text("Gain")).changed() {
                self.params.gain.set_value(gain);
            }

            // Live visualisation reading from snapshot.
            ui.label(format!("LFO phase: {:.2}", self.snapshot.lfo_phase));
        });
    }
}
```

Then split the UI by tab/section into submodules — `editor/tabs/*.rs` for layout, `editor/viz/*.rs` for canvas-style painters that take a `&mut egui::Ui` + `egui::Rect` and call `ui.painter()` directly. See `plugins/resonance-wavetable/src/editor/` for a complete reference implementation.

#### 6e. Wire it into the plugin

```rust
// src/lib.rs
use std::sync::Arc;
use resonance_plugin::*;

#[cfg(feature = "editor")]
mod editor;
mod params;
mod viz;
// ... other modules

use params::MyParams;
use viz::MyVizState;

pub struct ResonanceMyPlugin {
    params: Arc<MyParams>,
    viz: Arc<MyVizState>,
    // ... DSP state
}

impl ResonancePlugin for ResonanceMyPlugin {
    // ... CLAP_ID, NAME, etc.

    fn new() -> Self {
        Self {
            params: Arc::new(MyParams::new()),
            viz: Arc::new(MyVizState::new()),
            // ...
        }
    }

    // ... param_count/param/initialize/process

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::MyEditorFactory::new(
            self.params.clone(),
            self.viz.clone(),
        )))
    }
}

resonance_plugin::export_clap!(ResonanceMyPlugin);
```

The default `editor_factory()` returns `None`, so plugins without the editor feature get the standard host-rendered slider panel.

#### 6f. Standalone editor harness (fast iteration)

Building, bundling, loading the DAW, adding a track, adding the plugin, and clicking "Open Editor" every iteration is slow. Drop a tiny example that opens just the editor:

```rust
// examples/editor_standalone.rs
use resonance_plugin::plugin::ResonancePlugin;
use resonance_my_plugin::ResonanceMyPlugin;

fn main() {
    let plugin = ResonanceMyPlugin::new();
    let factory = plugin.editor_factory().expect("plugin should have a factory");
    let mut editor = factory
        .create("wayland", true)
        .expect("factory should build a wayland floating editor");
    editor.show();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}
```

Run with `cargo run -p resonance-<name> --example editor_standalone`. Edit a viz painter or tab, save, re-run — about ~1s rebuild and the editor pops open with the change.

#### 6g. Editor authoring rules

These are real lessons from building the wavetable editor:

1. **Don't disable egui's `default_fonts` feature.** Without it, all glyph metrics are zero-width — labels collapse, buttons are 1×1 px, text is invisible. Always include `features = ["default_fonts"]` when adding egui.
2. **Use `Panel::top` / `CentralPanel` `show_inside`, not `show`.** The `show(&Context, ...)` form is deprecated; pass `&mut Ui` instead. The runtime gives you a top-level `&mut Ui` from the start.
3. **Use `ui.columns(N, |cols| ...)` for side-by-side layouts.** `ui.horizontal` with nested `ui.vertical` chains will silently wrap when sliders try to fill width. Columns guarantee equal-width side-by-side placement.
4. **Don't run audio-thread synthesis on the UI thread.** The wavetable plugin's `wavetable::generate_all(48000.0)` does ~1B sin() calls and effectively hangs the editor. Write dedicated lightweight display generators that produce visually-faithful (not cycle-accurate) waveforms in milliseconds. UI only needs to look right, not match the audio bit-for-bit.
5. **Read viz state once per frame, store it in app state.** Call `self.snapshot = self.viz.read_snapshot()` at the top of `ui()` and pass references through the tab functions. Don't read atomics from individual painters — it can produce inconsistent values across one frame.
6. **Cache expensive display data behind `OnceLock`.** Wavetable buffers, font glyph layouts, computed curves — anything deterministic should be computed once and cached for the lifetime of the editor process.
7. **Request continuous repaint for live views**: `ui.ctx().request_repaint_after(Duration::from_millis(16))` at the top of `ui()`. Without it the editor only re-renders on input, and live markers stop animating.
8. **The runtime closes the editor automatically when the plugin is destroyed.** `ClapInstance::Drop` calls `close_gui()` before destroying the plugin. You don't need to handle this in your factory.

#### 6h. What the runtime handles for you

You don't need to worry about any of these — `wayland-plugin-gui` deals with them internally:
- Wayland connection, XDG shell, window decorations
- HiDPI: KDE/GNOME `wl_output` scale → `wl_surface.set_buffer_scale` + physical-pixel `wl_egl_window`
- egui `pixels_per_point` plumbing (must be passed via `RawInput.viewports[ROOT].native_pixels_per_point`, not `Context::set_pixels_per_point`)
- EGL context creation (GL 3.0 compatibility profile — egui_glow's GLSL 140 shaders won't compile under Core profile)
- Pointer/keyboard input translation from SCTK to `egui::Event`
- Frame loop, swap buffers, configure-event resize

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
- `gui::PluginEditor` trait - editor handle (`show`/`hide`/`set_size`/`size`/`can_resize`/`set_title`)
- `gui::EditorFactory` trait - produces `Box<dyn PluginEditor>` on demand from the host (`supports`/`preferred`/`preferred_size`/`create`)
- `ResonancePlugin::editor_factory()` - the optional hook plugins implement to expose `CLAP_EXT_GUI` (default: `None`)

### wayland-plugin-gui (`../../wayland-plugin-gui`)
Wayland-native plugin GUI runtime. Hosts an egui UI in a floating top-level Wayland window from inside a plugin dylib.
- `Editor::new(app, options) -> Result<Editor, EditorError>` - spawn an editor thread with an `EditorApp`
- `EditorOptions { title, initial_size, min_size, resizable }` - construction parameters
- `EditorApp` trait - one method: `fn ui(&mut self, ui: &mut egui::Ui)`. Called every frame on the editor thread
- `Editor` methods: `show()`, `hide()`, `set_size(w, h)`, `get_size()`, `is_resizable()`, `request_repaint()`, `destroy()`
- Re-exports `egui` so consumers don't pin a separate version: `wayland_plugin_gui::egui::*`
- Wayland-only, floating-only. Handles HiDPI, EGL, glow, input translation internally

### Additional dependencies (add as needed)
- `parking_lot = "0.12"` - fast Mutex/RwLock (use for shared state, file paths, background loading)
- `rustfft = "6"` - FFT for convolution or spectral processing
- `serde = { version = "1", features = ["derive"] }` + `serde_json = "1"` - for loading config files and custom state
- `egui = { version = "0.34", features = ["default_fonts"], default-features = false }` - if writing a plugin editor; **always include `default_fonts`** (see Step 6g)

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

### Editor-specific rules (only if you implement Step 6)

12. **Wrap params in `Arc<MyParams>`**, not bare. The editor thread holds a clone and reads/writes via the same atomic-backed `&MyParams` the audio thread uses.
13. **Publish viz state once per `process()` block**, not per sample. Atomic stores are cheap but not free, and the UI reads at ~60Hz.
14. **Use the seq-lock pattern for buffered viz state** (oscilloscope frames). Bump the seq counter to odd, write the front buffer, bump to even. Reader retries on mismatch.
15. **Read viz state once per UI frame** at the top of `EditorApp::ui()` and stash it in `self.snapshot`. Don't call `read_snapshot()` from individual painters.
16. **Always include `egui` `default_fonts` feature.** Without it text has zero-width metrics and the entire UI collapses to invisible widgets.
17. **Use `Panel::top` / `CentralPanel` `show_inside`, not `show`.** The `show(&Context, ...)` form is deprecated; the runtime gives you `&mut Ui` directly.
18. **Use `ui.columns(N, |cols| ...)` for side-by-side sections.** `ui.horizontal` with nested `ui.vertical` chains containing sliders silently wraps.
19. **Don't run audio-thread synthesis on the UI thread.** Write dedicated lightweight display generators that look right but aren't cycle-accurate.
20. **Cache deterministic display data behind `OnceLock`.** Anything computed at startup that doesn't change should be computed once for the editor process lifetime.
21. **Request continuous repaint for live views**: `ui.ctx().request_repaint_after(Duration::from_millis(16))` at the top of `ui()`. Without it the editor only re-renders on input.
22. **Provide a `cargo run -p <name> --example editor_standalone` harness.** It bypasses the DAW and gives you a ~1s edit-test loop.

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
# Build a single plugin (editor included if `editor` is in default features)
cargo build --release -p resonance-<name>

# Headless build (skip the editor feature)
cargo build --release -p resonance-<name> --no-default-features

# Iterate on the editor only — opens just the editor window, no DAW
cargo run -p resonance-<name> --example editor_standalone

# Bundle all plugins as .clap files
./scripts/bundle.sh
```

The compiled plugin is `target/release/libresonance_<name>.so`. The bundle script copies it to `target/bundled/resonance-<name>.clap`. The host discovers plugins in `target/bundled/`, `~/.clap/`, and `/usr/lib/clap/`.

When the DAW loads a plugin that exposes `CLAP_EXT_GUI`, the mixer panel header shows an **Open Editor** button alongside the existing close button. Clicking it opens the plugin's floating Wayland window. The DAW handles `gui_create` / `gui_show` / `gui_destroy` automatically — the plugin doesn't need to know about the host.

## Reference Implementation

`plugins/resonance-wavetable/` is the canonical example of a full-featured instrument plugin with an editor:

- **Audio side**: 2 oscs × 10 wavetables, 2 ADSR envs, SVF filter, 3 LFOs, 8-slot mod matrix, chorus/delay/distortion. ~87 parameters via `WavetableParams`.
- **Viz bridge**: `src/viz.rs` — `WavetableVizState` with atomic scalars + seq-lock'd scope ring, `ScopeCollector` helper, two-thread atomicity test.
- **Editor**: `src/editor/` — 5 tabs (OSC / ENV-FILTER / LFO / MOD / FX), 6 canvas-style visualisations (waveform with morph blend, frame strip, ADSR with playhead, LFO shape with phase marker, log-frequency filter response, stereo oscilloscope), shared theme, lightweight purpose-built display generators, standalone editor harness.

When in doubt, copy the relevant pattern from there.
