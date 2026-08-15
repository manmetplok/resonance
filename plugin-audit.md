# Plugin capability-vs-exposure audit

**Date:** 2026-08-15 · **Scope:** all 11 plugin crates under `plugins/`, the shared
framework `resonance-plugin`, the shared widget kit `wayland-plugin-gui`, and the
host app's plugin integration.

The question this audit asked was **not** "is the code correct" but **"what can the
DSP do that the user cannot reach?"** — plus its inverse, controls that are drawn
but bound to nothing.

Nothing in this document has been fixed. It is a findings register, not a changelog.

Severity is by **user impact**:

| | |
|---|---|
| **HIGH** | a shipped capability users cannot use at all, or a control that lies |
| **MEDIUM** | reachable only via automation, the control API, or file editing |
| **LOW** | cosmetic, or a deliberate choice worth recording |

Effort is **EASY** (a self-contained edit, no design decision) or **INVOLVED**
(needs a design call, new UI, or a behaviour change).

---

## The three patterns

Almost everything below is an instance of one of these.

**1. Controls that do nothing.** A knob or toggle is drawn, is preset-covered, and
no DSP reads it. This is the worst kind, because the user has no way to tell — they
conclude the effect is subtle rather than absent. Granular's `diffusion` and
`density_sync`, Delay's `stereo_offset` on 2 of 3 routes, Wavetable's four
mod-matrix options, and eight controls across Drums' GLOBAL/KIT cards.

**2. Capability with no way in.** The DSP implements something real and no
parameter or control reaches it. Mastering's per-band compressor settings, Amp's
A1 model catalogue, the WSOLA grain alignment in `resonance-dsp`.

**3. Backend without UI.** A complete, tested, undoable backend with a control-API
method and **zero** view-layer emitters — so an MCP client can do it and a human
cannot. Chain reorder, aux sends, sidechain routing, save-track-as-preset. This is
the same pattern that drove the 2026-08-14 epic triage, and it is the cheapest
class to fix.

---

## HIGH findings

### Controls that lie

| # | Plugin | Finding | Effort |
|---|---|---|---|
| G2 | granular-delay | **`diffusion` knob is inert.** Declared `src/params.rs:100`, drawn as "Diffuse" `src/editor/controls/mod.rs:358`, written by all six presets — not a field of `BlockParams` and never read in `src/dsp/**`. The param doc still says `TODO(epic-196)`. | INVOLVED (DSP stage must be written) |
| G3 | granular-delay | **`density_sync` (PER-BEAT) is inert.** `src/params.rs:55`, chip at `src/editor/controls/mod.rs:267`, set to `1` in `presets/eighth_triplet_echo.json` — so a shipped preset implies a tempo-locked grain rate that never happens. | INVOLVED |
| W1 | wavetable | **Mod-matrix sources "Mod Wheel" and "Aftertouch" always evaluate to 0.0.** `src/dsp/modulation.rs:146-147`. Offered in the picker at `src/editor/tabs/mod_matrix.rs:19-21`. Root cause is framework-wide: `NoteEvent` carries only NoteOn/NoteOff/Choke, so no CC ever reaches a plugin. | INVOLVED |
| W2 | wavetable | **Mod destinations "Osc Balance" and "Unison Detune" are computed and discarded.** `src/dsp/modulation.rs:160,162` accumulate into `ModState` fields nothing reads. | EASY for Osc Balance; INVOLVED for Unison Detune (detune is baked at note-on) |
| D1 | drums | **All 30 `pad_N_articulation` parameters are inert.** Declared `src/params.rs:54,108`, never read. The DSP reads `bridge.articulations` instead; the editor chip writes both, so clicking works and automation/MCP does not. | INVOLVED |
| D3 | drums | **KIT card ROUTING toggle displays the opposite of the truth.** `src/editor/app.rs:362` renders "Stereo | Multi-out" reading "stereo"; the plugin unconditionally declares all 7 output ports (`src/lib.rs:154-166`), so it is *always* multi-out. Unlike the GLOBAL card, this one carries no "preview" marker. | EASY |

### Capability with no way in

| # | Plugin | Finding | Effort |
|---|---|---|---|
| M1 | mastering | **Multiband per-band attack / release / knee / mix are hardcoded** (`src/stages/multiband/mod.rs:285-294`) while each band runs a full `GlueCompressor` that honours all of them — and the single-band glue stage exposes all seven as params. The defining multiband move (fast low release, slow high) is unreachable by any means, automation included. | INVOLVED |
| A3 | amp | **Tone3000 browser is pinned to Architecture-2.** `const ARCHITECTURE_A2: &str = "2"` at `src/tone3000/client.rs:14`, on both endpoints. The engine runs A1 correctly (reference-exact since epic #197) and `Load Model…` opens A1 files happily — but most of the tone3000.com catalogue is invisible in-plugin, with no indication a filter is applied. | EASY |
| A1 | amp | **Top-level windowed head is parsed, validated, then rejected.** `HeadParams::Windowed` is a full typed parse target (`src/nam/wavenet/params.rs:165-175`); `src/nam/parse/config.rs:185-189` refuses it. Models with a windowed output head fail to load entirely. | INVOLVED |
| G1 | granular-delay | **WSOLA onset alignment is implemented, metered, documented — and reachable only from a test.** `GrainParams::align` + `align_window_seconds` (`resonance-dsp/src/granular.rs:182-198`), with correlation search and metering. `align: true` appears only in `resonance-dsp/tests/granular_align.rs`. No param, no control. | EASY (a bool param + a chip) |

### Framework gaps that block plugin authors

| # | Finding | Effort |
|---|---|---|
| F1 | **No output parameter events, no gestures.** Both `flush` impls ignore `_output_parameter_changes` (`resonance-plugin/src/clap_bridge/params.rs:85,115`) and `process()` never writes `events.output`. A host cannot record automation from a plugin's own GUI, cannot group a drag into one undo step, and its display goes stale — the app calls `query_params()` exactly once at instance creation, so the mixer panel shows the value the plugin had when it loaded, forever. | INVOLVED |
| F2 | **No plugin can report a latency change.** The bridge holds host handles but marks them `#[allow(dead_code)]` (`clap_bridge/shared.rs:34-35,108-109`); `ResonancePlugin` exposes no host handle. The *host* side is fine — `clap_host_latency` is served and polled since epic #198 — so this is purely a plugin-side gap. Consequence: a lookahead or oversampling control is unimplementable; the DSP would delay by the new amount and every other track would stay compensated for the old one. **No current plugin is affected** (mastering and IR both have constant-per-activation latency). | INVOLVED |
| F3 | **No MIDI CC / aftertouch / pitch-bend anywhere.** `clap_bridge/process.rs:102-127` handles notes and ends with `_ => {}`. This is the root cause of W1. | INVOLVED |
| F4 | **`float_knob` takes range/default/display as arguments instead of reading the `FloatParam` it is handed** (`resonance-plugin/src/editor_widgets.rs:59-82`), so editor and params drift silently. Found drifted in reverb (4 knobs — **fixed 2026-08-15**), compressor (4), amp (1), IR (1). The pattern to copy already exists: `plugins/resonance-granular-delay/src/editor/widgets.rs:112-140`. | EASY |
| F5 | **No way to type an exact value into any parameter.** Zero `DragValue` in the fleet; `Param::parse` and `text_to_value` are fully implemented and unit-tested, and nothing calls them. A user cannot set the compressor to exactly −18.0 dB. (`resonance-eq` gets it accidentally via raw `egui::Slider`.) | EASY |

### App integration

| # | Finding | Effort |
|---|---|---|
| P1 | **"Save track as preset" is dead code with no trigger.** `presets.rs:205 save_user_preset` and the whole capture pipeline (`engine_events/presets.rs:64-103`) work; `pending_preset_save` is declared, initialised to `None`, and `take()`n — **nothing ever sets it**, and no `SaveTrackAsPreset` message exists. The GUI can apply and delete user presets, so the menu lists presets the user has no way to create except by hand-writing JSON. | EASY |
| P2 | **No plugin-chain reorder in the GUI, on any chain.** Backend complete for track/bus/master, including undo classification and the instrument-floor rule; `track.move_effect` / `bus.move_effect` / `master.move_effect` all exist. Zero view-layer emitters. A user must delete and re-add a plugin — losing every parameter — to change insert order. | EASY as ▲▼ buttons; INVOLVED for drag-and-drop |
| P3 | **Aux sends have a complete backend and no UI whatsoever.** `MixerMessage` carries eight send/return variants and documents itself as raised from the Mixer inspector's ROUTING group; grep outside `update/`, `message.rs` and `undo/` returns **zero** emitters. What the GUI shows is two hardcoded read-only rows: `"Send A" → "(none)"`, `"Send B" → "(none)"` (`inspector/routing.rs:73-74`). | INVOLVED |
| P4 | **Sidechain routing is control-API only.** `grep -rni sidechain resonance-app/src/view/` returns nothing. The engine even echoes `SidechainRouteChanged` with the comment "so a GUI-side route view can hang off it". Aggravating: sidechain routes are **not persisted at all**, and a plugin on a bus or master can never be keyed (`SetSidechainParams` is keyed by `track_id`). | INVOLVED |
| P5 | **A missing plugin becomes an indistinguishable dead slot.** Replay pushes a placeholder `PluginSlotState` unconditionally; if the `.clap` is gone the engine emits a generic error and nothing removes or marks the placeholder. No missing/offline state, no relocate, no replace. **Includes a real data-loss path:** save-as and template capture write only cached blobs, so the missing plugin's opaque state is silently lost on the first Save As. | INVOLVED |
| P6 | **resonance-gate is the only plugin with no theme at all.** Never calls `theme::apply`; has no `editor/theme.rs`. Opening it beside any other Resonance plugin shows a stock-grey egui window that reads as a different vendor's product. | EASY |
| P7 | **resonance-delay factory presets are partial recalls.** Every preset carries 14 of 22 param ids; the eight gate/duck ids appear in none, and the shared loader only writes ids present in the map. Loading "Slapback" leaves a trance gate running from the previous patch. All five other preset-carrying plugins ship full snapshots. | EASY |
| P8 | **Eight mastering params ship with no unit and no formatter.** The `q` FloatParam (`src/params/eq_stage.rs:60-69`) ends its builder chain without `.with_unit` or `.with_value_to_string`, instantiated 4 bands × 2 stages. The only such FloatParam in all 11 plugins. Reads "0.71" in a host automation lane with no indication what it is. | EASY |
| P9 | **Every choice/enum parameter in the fleet displays as a bare integer.** `IntParam` has no `with_value_to_string` builder at all. Affects wavetable 26, eq 16, mastering 10, granular 9, delay 4, IR 1 (an IR index 0–999, so an automation lane reads "37" instead of a cab name). Label tables exist but live only inside the editors. | INVOLVED (framework builder + ~7 crates) |

---

## MEDIUM findings

### Per-plugin

**resonance-amp**
- **A2** — Slimmable / A2-Lite size selection is implemented and tested (`src/nam/wavenet/slimmable.rs`, 599 lines of tests) but hardwired to `FULL_SIZE`; anything else is refused at `src/nam/parse/container.rs:51-56`. The user cannot trade model fidelity for CPU. *INVOLVED*
- **A4** — Tone3000 pagination is implemented (`page`, `page_size`, `total` all decoded) and never requested; the worker always passes page 1. The user only ever sees the first 25 results, and the panel shows `Tones (25)` with no hint more exist. *EASY*
- **A6** — `file_select` is `.hidden()` (`src/params.rs:40`), so model switching is invisible to the host param list and to the app's own plugin-param API. The identical param in resonance-ir is **not** hidden — the inconsistency looks unintentional. *EASY*

**resonance-ir**
- **I1** — Convolution block size (= the plugin's reported latency) is derived from sample rate with no user control (`src/dsp.rs:13-21`). No low-latency tracking mode, and no way to see the ~2.7 ms imposed. *INVOLVED*

**resonance-mastering**
- **M2** — Band "Gain" is silent unless that band's compressor is enabled: gain is routed as compressor makeup and the compressor returns before applying gain when disabled. The multiband cannot be used as a static 4-band tone balancer. *EASY*
- **M3** — The whole-plugin `bypass` param has no control in the plugin window (`grep -rn bypass src/editor/` is empty), despite a purpose-built latency-matched dry path backing it. The user cannot A/B the master chain from the mastering window. *EASY*
- **M4** — `target_lufs` is drawn in three places and has no control; the sole writer is the assistant. The user cannot move their own loudness reference line. *EASY*
- **M5** — Per-band gain reduction is measured by each band compressor and never exposed, so setting per-band thresholds is blind. *INVOLVED*
- **M6** — Limiter lookahead is the constant `LOOKAHEAD_MS = 5.0`, and the UI prints "Lookahead: 5 ms" as static text exactly where the knob would go. The standard second knob on any mastering limiter. *INVOLVED* (ring sizing and `Chain::latency` assume it is constant — see **F2**) 

**resonance-compressor / resonance-gate**
- **C1** — External sidechain key is unreachable from any GUI, for both plugins. Both declare `SIDECHAIN_INPUT = Some(2)` and fully implement key-driven detection. A mouse-driven user cannot duck a pad from a kick at all. (Same root as **P4**.) *INVOLVED*
- **C2** — The compressor editor gives no indication a key is connected: `lib.rs` receives `key: Option<KeyBuffer>` and discards the presence bit, while the IN meter silently changes meaning. *EASY to publish the flag; INVOLVED to add key metering*
- **C3** — The gate's `key_connected` flag is computed, documented as UI-facing ("surfaced so the editor can tell the user which detector is actually running"), and reaches nothing but a test. *INVOLVED (gate has no viz object yet)*
- **C4** — The gate ships no presets and no preset picker, while EQ and compressor each ship 11. Supporting evidence it was planned and dropped: the gate depends on `serde_json`, which no gate source or test uses. *EASY*
- **C5** — Gate knobs ignore the parameters' declared skew: attack, hold, release, ratio and key HPF are all `FloatRange::Skewed`, and the editor hardcodes `logarithmic: false` for every knob. A 1 ms attack is undialable. (The compressor passes `true` for its equivalents.) *EASY — caveat: `key_hpf` has min 0.0 and the log path clamps to 0.001, so that one needs a special case*

**resonance-delay**
- **D1** — `stereo_offset` is ignored on 2 of the 3 routing modes with no UI hint, and the shipped "Lo-Fi Tape" preset sets a value that does nothing. *EASY for a disable-hint; INVOLVED if the offset should apply on all routes*
- **D2** — `gate_rate` shows a raw integer with no division label anywhere, unlike the delay's own division which the header labels. The user drags a knob reading "7" and must know from the source that it means 1/8. *EASY*

**resonance-wavetable**
- **W3** — The LFO mode segmented control has a dead third segment: labels are `["Free", "Sync", "Env"]` over a single bool, so clicking "Env" visibly snaps back. "Sync" is retrigger, not tempo sync — there is no tempo sync in the DSP at all (`_tempo` is ignored). *EASY to fix the labels; INVOLVED to implement sync*
- **W4** — Every editor knob ignores its parameter's declared skew. Filter cutoff is linear across 20–20000 Hz, so everything below ~2 kHz lives in the first 10% of the arc; sub-100 ms envelope times are effectively unsettable by mouse. *EASY (add `FloatRange::denormalize`, route `float_knob` through it)*

**resonance-drums**
- **D2** — GLOBAL card: POLYPHONY, VELOCITY CURVE and ROUND ROBIN are drawn and bound to nothing. `MAX_VOICES = 64` is a hard constant with no param — an asymmetry with the wavetable synth, which does expose max voices. Velocity curve and random round-robin are implemented nowhere. *EASY for polyphony; INVOLVED for the other two*
- **D4** — Four of five editor tabs are "Coming soon", and two of them hide features that already work: Mics and Articulations are both fully implemented inside the Pads inspector. A user looking for mic selection clicks "Mics", is told it doesn't exist, and never finds the pickers one tab over. *EASY for the two redundant tabs*
- **D5** — "▶ Audition" is drawn as an active control and discards its click. The sampler can already trigger any pad from a note-on; what's missing is an editor→audio-thread trigger channel. *INVOLVED*
- **D6** — Round-robin index and depth are packed and published by the audio thread explicitly "so the editor can show per-pad round-robin indicators", and both consumers reduce it to a boolean. The user cannot see which take of how many fired. *EASY (unpack the two halves)*

### Cross-cutting

| # | Finding | Effort |
|---|---|---|
| X1 | **No plugin lets a user save a preset — 0 of 11.** The shared helper exposes only `load`; there is no `save`, no user-preset directory, no save dialog. A user who dials in a sound can only keep it inside that one project. | INVOLVED (do it once in `resonance-plugin`) |
| X2 | **Preset combos behave three different ways and none survives closing the window.** Four plugins hardcode "— select —" and never show what is loaded; granular tracks and displays it; wavetable adds ◀/▶ steppers. No plugin includes preset identity in `save_state`. | EASY to unify display; INVOLVED to persist |
| X3 | **Plugin-level bypass exists in exactly one plugin, and no bypass anywhere is click-free.** Only mastering has a bypass param. The host-level fallback simply skips the FX chain with no ramp, so bypassing a reverb or delay mid-playback truncates the tail. | INVOLVED (a short crossfade in `run_fx_chain` fixes it fleet-wide) |
| X4 | **The generic param panel is unreachable for any plugin that declares a GUI.** `view/mixer/mod.rs:237-249` routes to `TogglePluginPanel` only when `has_gui == false`. Combined with an optimistic `editor_open = true` set before the engine replies, a plugin whose window fails to open leaves a slot reading "Close Editor" with no window and no fallback. The control API has no such restriction. | EASY |
| X5 | **Plugin/chain latency is invisible.** PDC is solid and covers tracks, sub-tracks, busses and sends — but no add-echo event carries a latency field, so the app has nothing to draw. A user cannot tell that a lookahead limiter just added 4096 samples. | INVOLVED (needs a new event field first) |
| X6 | **CLAP feature strings are silently swallowed.** The bridge maps features through a hand-written whitelist and `filter_map`s the rest away. `compressor`, `equalizer`, `delay`, `mastering`, `analyzer`, `gate` are all standard CLAP constants that never reach the host, and IR has a plain typo (`cabinet_simulator` vs `cabinet-simulator`). 7 of 11 plugins land in the wrong browser category in Bitwig/Reaper/Carla. | EASY (better: make `FEATURES` a `&[&CStr]` so an unmapped string is a compile error) |
| X7 | **No parameter groups.** `module` is hardcoded empty, so mastering's ~60 params across 8 stages reach a host as one flat list. The plugin's own editor tabs them; the host's automation-lane picker cannot. | EASY (add `Param::module()`) |
| X8 | **Custom formatters are used everywhere and called nowhere in Resonance.** 100+ `with_value_to_string` call sites; `query_params` never asks for text, so the app's generic panel prints `{:.2}`. A mix param reads `0.40`, not `40 %`. All that formatter work only pays off in third-party hosts. | EASY (add a `text` field to `ParamInfo`) |
| X9 | **`FloatParam::smoother` / `with_smoother()` is a silent no-op.** `Smoother::next()` needs `&mut self` but params live behind `Arc`, so the field can never advance; every plugin owns a parallel smoother struct instead. **14 `with_smoother(...)` calls** across wavetable and mastering read like configured smoothing and do nothing. | EASY (delete the field and builder) |
| X10 | **No plugin rescan.** `ScanPlugins` is sent once at startup, so installing a plugin requires an app restart — for the GUI and `plugins.catalog` alike. | INVOLVED |

---

## LOW findings

Recorded for completeness; several are deliberate choices.

**Widget forks that `#1266` did not reach.** The knob fork is genuinely gone and drag feel is unified — but drums and wavetable each carry a private near-duplicate of the same three widget files (`chip.rs` differs only in a doc comment; `segmented.rs` differs only by dead code; `slider.rs` is a superset fork that has already diverged once on bipolar fill colour). Granular carries a *third*, unrelated chip implementation with different geometry. resonance-eq uses raw `egui::Slider`, a fourth idiom. Both editors' own `widgets/mod.rs` admit the duplication. *EASY — promote the trio into the shared kit.*

**Eleven byte-identical copies of the editor host bridge.** The ~55-line `RuntimeEditorHandle` is duplicated verbatim in all 11 factories. *EASY.*

**`PluginEditor::size()` goes stale after any user resize, in all 11.** Compositor-driven resizes are never fed back, so the host persists and restores the wrong editor size. *EASY.*

**Wavetable uses both knob families in one window** — 25 classic call sites alongside 12 themed ones. No other plugin mixes them. *EASY–INVOLVED depending on layout fallout.*

**The fleet is split across two palettes** — classic blue (7 plugins) vs canonical lavender (3), plus gate on nothing. Documented in `wayland-plugin-gui/src/theme.rs` as an in-progress migration, so this is tracked, not disputed.

**Presets never demonstrate five shipped granular modes** — across all six presets, `quality` is always Normal, `filter_type` always LP, `time_mode` never Repitch, `fb_route` never Output-only. All reachable, never showcased. *EASY (preset JSON only).*

**Smaller items:** amp's tuner is hardcoded to A=440 and cannot be switched off (its YIN tracker runs on every block); amp's Tone3000 gear filter is hardcoded to amp+cab; IR's frequency-response plot analyses only the left channel truncated to 4096 samples; IR's `file_select` caps at index 999; mastering's crossover slope, dither noise-shaping curve, saturator tape voicing, imager side-HPF Q and assistant capture window are all fixed constants; mastering's assistant genre and reference track are not persisted; wavetable's glide knob is labelled in seconds and valued in milliseconds; wavetable's Filter Key Track is drawn as a bipolar knob over a unipolar param; wavetable's LFO shape shows a bare integer; wavetable's status-bar SR and buffer size are literals; wavetable's LFO routing labels describe a macro system that does not exist; drums' status bar shows invented CPU/RAM telemetry and a permanently dead OUT meter; drums' choke groups are general in the engine and hardcoded to hi-hats; drums fakes the loaded sample's filename and waveform; delay's echo view mirrors the L lane so ping-pong and stereo offset never show; the EQ's Q control is display-only on cut bands; the EQ publishes both pre and post spectra and can only display one; the inspector chain row renders a dead "BYP" label for a per-plugin bypass that exists nowhere.

---

## Test-coverage outliers

DSP golden files exist in **2 of 11** (granular-delay, wavetable). A
`save_state`→`load_state` round-trip exists in **5 of 11** and is missing in amp,
delay, gate, granular-delay, reverb and **wavetable** — the 87-param, 23-preset
crate. Editor tests exist in **1½ of 11**. `resonance-gate` is the floor: one test
file and none of the three categories.

In the framework, `process()` — the audio path, containing the crate's only
`unsafe` block — is never driven through the ABI, and the load-vs-editor-pushback
CAS (the most carefully reasoned code in the crate) has no concurrency test. State
has no version field, so renaming a param's string id silently restores that
parameter to its default across every saved project and factory preset.

---

## Where the plugins are consistent

Worth recording, so it is not re-litigated:

- **Latency is correct everywhere.** Only IR and mastering have real algorithmic
  delay and both report it accurately. The other nine are genuinely zero-latency.
  No plugin silently smears the mix's timing.
- **Bundling is consistent** — all 11 in the workspace members array, and
  `scripts/bundle.sh` cross-checks both directions plus a cdylib guard.
- **Every plugin has an editor**, declares `CLAP_EXT_GUI` through the same path,
  ships it as a default feature, and declares `resizable: true`.
- **Stepped declaration is correct** — no plugin encodes a choice as a FloatParam.
- **Param-ID derivation is stable by design** and pinned by test against exactly
  the failure it fears (renaming a display name or changing a range must not move
  a host's automation lane).
- **State save/load across the active/inactive boundary** is the hardest thing in
  the framework and it is correct, deeply commented, and thoroughly tested.
- **`set_scale` being refused** is not a stub — it is the CLAP-correct answer for a
  Wayland-only runtime.
- **The sidechain port design** (non-main port, uncollidable id, `process_with_key`
  defaulting to `process`) is clean and backwards-compatible.

---

## Suggested order

Ranked by value-per-unit-effort, not by severity.

1. **P1 save-track-as-preset** — HIGH, EASY. Fully-built backend; one message and one menu item.
2. **A3 Tone3000 A1 filter** — HIGH, EASY. One constant hides most of the model catalogue from a fully capable engine.
3. **P2 chain reorder as ▲▼ buttons** — HIGH, EASY. Backend, undo and control API already exist on all three chains.
4. **F4 param-bound knob helper** — HIGH, EASY. Makes the whole class of editor/param drift impossible rather than fixing four instances.
5. **W2 Osc Balance**, **D6 round-robin readout**, **M3 bypass**, **M4 target_lufs**, **D3 drums routing label**, **P6 gate theme**, **P7 delay presets**, **P8 mastering units** — MEDIUM/HIGH, all EASY, all one-widget or one-line.
6. **F5 type-an-exact-value** — HIGH, EASY. The parse side is already built and unit-tested; only the widget is missing.
7. **G1 grain alignment** — HIGH, EASY. A bool param exposes a finished feature.
8. Then the INVOLVED work: **M1** multiband controls, **P3** aux-send UI, **P4** sidechain UI (with persistence), **P5** missing-plugin handling, and the framework trio **F1/F2/F3** (output events + gestures, host handle, MIDI CC).

---

## Provenance

Seven parallel read-only reviews, 2026-08-14/15, each scoped to a disjoint slice:
amp/ir/mastering · eq/compressor/gate · delay/reverb/granular · wavetable/drums ·
cross-plugin consistency matrix · app-side integration · framework and GUI kit.

Findings were reported with `file:line` evidence for both the capability and the
place it should have been exposed. Deliberately-internal items (test hooks,
smoothing coefficients, algorithm constants) were excluded by instruction rather
than reported as gaps.
