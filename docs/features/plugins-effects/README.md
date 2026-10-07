# Effect plugins

Resonance ships ten first-party effect plugins as CLAP bundles under
`plugins/`, plus two shared pieces that are not plugins of their own: the
mastering assistant engine (`resonance-mastering-assist`) and the de-harsh
resonance suppressor (`resonance_dsp::deharsh`, used by the mastering
plugin). Each plugin is an ordinary CLAP effect built on the in-house SDK
(`resonance-plugin`), with its DSP in the plugin crate and shared building
blocks in `resonance-dsp`. The app links none of them: it loads the bundles
through its CLAP host like any third-party plugin, and knows them only by
the ids in `resonance-plugin/src/first_party.rs`. Instruments (wavetable,
drums, amp) are in [`../plugins-instruments/`](../plugins-instruments/).

| Feature | Doc | One-line summary |
|---|---|---|
| EQ | [eq.md](eq.md) | 8-band parametric EQ with M/S bands, Tilt/Lift+Dip/Air types, dynamic bands, auto-gain and a live analyzer |
| Compressor | [compressor.md](compressor.md) | Feed-forward soft-knee compressor with peak/RMS detector, Auto release, sidechain key and HPF |
| Gate | [gate.md](gate.md) | Noise gate / downward expander with hysteresis, hold, range and an external key |
| Delay | [delay.md](delay.md) | Tempo-synced stereo delay (Stereo/Ping-Pong/Dual) with Digital/Analog character, freeze, wet gate and duck |
| Granular delay | [granular-delay.md](granular-delay.md) | Delay buffer read by up to 64 pitched grains, with freeze, shimmer feedback, pitch-sync and quality tiers |
| Reverb | [reverb.md](reverb.md) | Nine algorithmic reverbs (Classic … Shimmer) with a return channel: wet EQ, ducking, ER/tail balance, tempo sync |
| Stereo | [stereo.md](stereo.md) | Mono-safe width tool: M/S width, mono-maker, four wideners, balance, rotation, goniometer |
| Color | [color.md](color.md) | Saturation in five voicings (Tube, Tape, Transformer, Console, Warm) with Tape HQ hysteresis and auto-gain |
| IR | [ir.md](ir.md) | Convolution loader for cab and room IRs with a user-chosen latency mode |
| Mastering | [mastering.md](mastering.md) | Fixed-order mastering chain of ten stages, loudness metering and an assistant panel |
| Mastering assistant engine | [mastering-assist.md](mastering-assist.md) | Analysis + genre/reference decision engine shared by the mastering plugin and `master.assist` |
| De-harsh | [deharsh.md](deharsh.md) | STFT resonance suppressor in `resonance-dsp`, used as the mastering chain's de-harsh stage |

## What every effect plugin shares

These hold for all ten plugins; the per-plugin docs only note exceptions.

- **SDK and bundle.** `impl ResonancePlugin` plus `resonance_plugin::export_clap!`
  in `src/lib.rs`; the crate is a `cdylib` + `lib`, and `scripts/bundle.sh`
  turns it into a `.clap`. The CLAP id comes from
  `resonance_plugin::first_party` (`tools/arch-invariants` rejects a
  literal id anywhere else).
- **Parameters.** `src/params.rs` (or `src/params/` for mastering) declares
  `FloatParam` / `IntParam` / `BoolParam` with a stable string id. That id
  is the key the control API and the skills use. Choice params carry their
  labels via `IntParam::with_choices`, so hosts and the control API read
  "Ping-Pong", not "1". Every plugin returns a `ParamTextSource` so a host
  reads live values while the plugin is active (pinned per plugin only for
  the gate, `plugins/resonance-gate/tests/live_values.rs`; the mechanism is
  in `resonance-plugin/tests/clap_bridge_params_state.rs`).
- **Presets.** Factory presets are full JSON snapshots baked in with
  `include_str!` from `plugins/<name>/presets/*.json` and listed in
  `src/presets.rs` (`FACTORY_PRESETS`). User presets, the loaded-preset
  identity and its "edited" flag go through
  `resonance_plugin::presets::PresetSession`, saved with the plugin state.
  The editor's preset bar is `resonance_plugin::preset_ui::preset_bar`.
- **Editor.** egui via `plugin-gui-core`, hosted by the platform runtime
  (Wayland or Cocoa) through `resonance_plugin::editor_host`. The
  `editor` cargo feature is on by default. Every `editor/theme.rs`
  re-exports `plugin_gui_core::theme::lavender::*`. Knobs come from
  `resonance_plugin::editor_widgets` and read range, skew and default from
  the param they edit. No plugin editor has pixel goldens. Layout and
  binding are pinned by headless editor tests instead.
- **Tests.** Each crate has its own `tests/`. `dsp_golden.rs` uses
  `resonance-dsp-test-support` (bit-exact `f32` or `u32` blobs in
  `tests/golden/`, re-bless with `RESONANCE_BLESS=1`). Plugins that a skill
  names have `tests/skill_keys.rs`
  (`resonance_dsp_test_support::skill_keys_test!`): every key, label and
  preset inside a `<!-- keys: com.resonance.<id> -->` block of a skill
  must exist. Run one plugin with `./scripts/run-tests.py -p resonance-<name>`.

## Shared surfaces

- **GUI:** Mixer → inspector → CHAIN group (`resonance-app/src/view/mixer/inspector/chain.rs`)
  on a track, bus or the master: "+ Add to chain", bypass dot, `↗` opens
  the editor, `☰` menu (parameters, presets, replace, move, remove), drag
  to reorder. See [`../mixer/`](../mixer/).
- **Control API / MCP:** generic per-owner plugin methods, the same for
  every plugin (`resonance-control/src/methods/{track,bus,master}.rs`,
  `plugin_preset.rs`). For a track:
  `track.add_effect` → `mcp__resonance__track_add_effect`,
  `track.set_plugin_param` → `mcp__resonance__track_set_plugin_param`,
  `track.plugin_params` → `mcp__resonance__track_plugin_params`,
  `track.plugin_presets` / `track.load_plugin_preset` / `track.save_plugin_preset`,
  `track.set_plugin_bypass`, `track.replace_effect`, `track.move_effect`,
  `track.remove_effect`, `track.set_sidechain` / `track.clear_sidechain`.
  `bus_*` and `master_*` mirror them. `presets.search` →
  `mcp__resonance__presets_search` finds factory presets by metadata;
  `meter.probe` → `mcp__resonance__meter_probe` measures an insert chain's
  harmonic signature. See [`../control-api/`](../control-api/).
- **Agent skills** (`resonance-agent-plugin/skills/`): `mixing` has keys
  blocks for EQ, compressor, color and reverb (and explains
  `master_assist`); `spatial` for stereo, reverb, delay and EQ;
  `mastering` for mastering and reverb. Gate, granular delay and IR are
  named by no skill's keys block.

## Open intents for this area

| Intent | Why |
|---|---|
| [deharsh-cut-readout](../../intents/plugins-effects/deharsh-cut-readout.md) | The de-harsh cut is computed but nobody can see it |
| [deharsh-track-insert](../../intents/plugins-effects/deharsh-track-insert.md) | The resonance suppressor is master-only |
| [effects-live-agent-session](../../intents/plugins-effects/effects-live-agent-session.md) | The skills + effects loop was never run end to end on a real project |
| [plugin-editor-visual-checks](../../intents/plugins-effects/plugin-editor-visual-checks.md) | No effect editor has a pixel golden; reverb/Color/Stereo visuals unchecked |

## By-hand editor checks

`plugins/resonance-gate/tests/editor_open.rs` (Wayland) and
`editor_open_cocoa.rs` (macOS) are the `#[ignore]`d lifecycle guards for
the editor host every plugin shares; see CLAUDE.md for the commands.
