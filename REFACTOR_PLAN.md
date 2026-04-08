# Resonance Refactoring Plan

## Phase 1 — Fix Bugs
1. **Fix disconnected `file_list` in amp and IR params** — The `file_select` display closure captures a fresh `Arc<Mutex<Vec<String>>>` never connected to the actual file list.
2. **Fix freeze toggle in reverb** — Only calls `set_freeze(true)`, never unfreezes.
3. **Remove dead `settings.rs`** — Empty struct, never imported.

## Phase 2 — Create Shared Crates
1. **Create `resonance-common` crate** with:
   - Denormal flush utility (currently copy-pasted in all 4 plugins)
   - `scan_directory` function (duplicated in amp + IR)
   - WAV decode + linear resample (duplicated in drums + IR)
   - Gain parameter builder helper
2. **Create `resonance-dsp` crate** with:
   - `DelayLine` (from reverb)
   - `OnePole` filter (from reverb)
   - `Lfo` (from reverb)
   - `SimpleRng` (from reverb)
   - Constant-power pan law function
3. **Update all plugins** to use shared crates instead of inline copies.

## Phase 3 — Split `main.rs` (future)
- Extract sub-state structs (TransportState, TimelineViewState, PunchState, PluginUiState)
- Split update() into modules: transport, clips, plugins, punch
- Split view() into modules: transport, mixer, plugins
- Extract utility functions: format_db, format_pan
- Deduplicate button styles into theme.rs

## Phase 4 — Split `engine.rs` (future)
- Extract RecordingState struct from loose locals
- Split into: command_handler, mixer, pipewire/platform, recording, plugin_scanner
- Deduplicate monitor/playback mixing paths
- Extract pan law, pick_sample_rate helpers

## Phase 5 — Safety Cleanup (future)
- Fix unsafe impl Send on convolver (use proper trait bounds)
- Remove Clone from AudioClip or use Arc<[f32]>
- Strengthen unsafe impl Send safety comment on AudioEngine
- Replace magic numbers with named constants
