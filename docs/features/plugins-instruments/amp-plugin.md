# Amp plugin (NAM inference)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/nam-model-library.md` (library and
> editor; merged as `e28ca519`, although its status line still says
> "unmerged"). The A2 and LSTM loaders were epic #197 work, recorded in the
> commit history rather than a spec.

`com.resonance.amp` ("Resonance Amp") plays Neural Amp Modeler (`.nam`)
captures of amps and pedals. The user picks a model from the shared model
library; the plugin runs it on the input with input and output gain and a
built-in tuner. It has no tone stack, cab/IR or noise gate of its own
(the IR plugin and the gate do those). Model management is described in
[nam-model-library.md](nam-model-library.md).

## Behaviour

- **Params (3).** `file_select` "Model Select" (int 0..=999, a library
  slot; its text is the model name, or "Missing: …", "External: …",
  "(empty)", "slot N" while the library is busy; text input takes a number,
  "slot N", a name or an id prefix). `input_gain` (0.01–4, shown in dB,
  default 1.0), `output_gain` (0.001–4, default 0.5). `file_select` is
  excluded from plugin presets as a param; presets carry the model by
  content id instead.
- **Architectures.** WaveNet A1, WaveNet A2 (gating modes including
  blended and secondary activation, bottleneck, grouped convolutions,
  layer and head 1×1, eight FiLM sites, windowed head, `condition_dsp`
  sub-network) and LSTM, plus the slimmable container. Anything else is
  rejected as "Unsupported architecture". Slimmable models load only at
  full size; runtime A2-Lite size selection is not implemented and a
  smaller selection is refused.
- **Loading.** A persistent `amp-loader` thread watches the slot request,
  resolves it through the library, parses, primes the model with silence,
  samples a transfer curve for the editor, and posts the result through a
  `Mailbox`. The audio thread swaps models with a 1024-sample crossfade
  (`SwapFader`) and retries a swap it could not take; retired models are
  freed on a janitor thread.
- **Processing.** Stereo input is summed to mono to drive the model, DC is
  blocked per channel, gains are smoothed, and long blocks are chunked. An
  identity model or no model plays the dry signal.
- **Missing models.** Project state stores `model_path` (always) plus
  `model_id` (sha256), `model_name` and `model_source` (Tone3000 ids).
  `resolve_model` returns Load, Relinked (the file moved but the id is in
  the library, so the path is rewritten), Missing (the reference is kept
  byte-for-byte and the plugin passes audio through) or Nothing. The editor
  shows a missing-model banner.
- **Tuner.** A YIN pitch tracker on the dry, pre-gain mono sum.
- **Editor.** 960×620 (760×520 minimum): header with `Library…`, preset
  bar, ◀/▶ over the library view, ★ and the model name; tuner strip; scope,
  transfer curve and meters; gain knobs. `Library…` opens the overlay with
  Installed and Tone3000 tabs.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Plugin + state | `plugins/resonance-amp/src/lib.rs`, `model_ref.rs` | `AmpExtraState`, `resolve_model`, `ModelStatus` |
| Params | `src/params.rs` | `PARAM_COUNT = 3`, `slot_text`, `slot_from_text` |
| Loader | `src/loader.rs` | `amp-loader` thread, `prime_model`, `apply_reference` |
| DSP | `src/dsp/processor.rs` | `AmpProcessor` |
| Inference | `src/nam/{mod,activations,gemm,lstm}.rs`, `nam/wavenet/{params,film,slimmable,conv_layer,head,history}.rs`, `nam/wavenet/model/{build,forward,validate,weights}.rs` | `NamInference`, `WaveNetFullConfig`, `LstmModel` |
| Parsing | `src/nam/parse/{mod,schema,engine_config,config,container,weights}.rs` | `load_model_from_file`, `build_model` |
| Library glue | `src/library.rs`, `library_rows.rs`, `models.rs` | `SharedLibrary` |
| Tone3000 | `src/tone3000/{auth,client,worker}.rs` | see the library doc |
| Tuner + viz | `src/tuner.rs`, `src/viz.rs` | |
| Editor | `src/editor/{app,header,controls,scope_view,curve_view,meters,tuner_view,missing_banner,library_panel,tone3000_panel,actions,jobs,factory}.rs` | |
| Host-read keys | `resonance-plugin/src/first_party.rs` | `first_party::amp` |

Invariant: inference allocates nothing in `process_sample` for any model
kind (`tests/nam_rt_safety.rs`).

## Surfaces

- **GUI:** add "Resonance Amp" as an effect; editor from the plugin slot.
- **Control API / MCP:** pick a model with `track_set_plugin_param` on
  "Model Select" by slot or name (resolved through the plugin's
  param-from-text); list and mark models with `amp_models.list` /
  `amp_models.set_marks` (see [nam-model-library.md](nam-model-library.md)).
- **Agent skills:** `resonance-agent-plugin/skills/mixing` uses
  `amp_models_list`, `amp_models_set_marks` and `track_set_plugin_param` for
  amp tone.

## How it's tested

| What | Where | Run |
|---|---|---|
| Reference parity with upstream C++ renders | `plugins/resonance-amp/tests/nam_a1_reference_parity.rs` (tol 2e-6), `nam_a2_reference_parity.rs` (1e-6), `nam_lstm_reference_parity.rs` (1e-6); fixtures and READMEs in `tests/fixtures/{a1,a2,lstm}/` | `./scripts/run-tests.py -p resonance-amp` |
| A2 features | `tests/{nam_a2_parse,nam_slimmable,nam_wavenet_film,nam_wavenet_gating,nam_wavenet_groups,nam_wavenet_bottleneck,nam_wavenet_head1x1,nam_wavenet_windowed_head,nam_wavenet_condition_dsp,nam_config_surface_agreement}.rs` | same |
| Parsing and robustness | `tests/{nam_parse,nam_hostile_input,nam_activations,nam_reference_semantics,matvec_null}.rs` | same |
| RT safety, determinism, swap | `tests/{nam_rt_safety,nam_reset_determinism,swap_retry}.rs` | same |
| State, library, editor | `tests/{state,model_library,model_selector,editor_param_binding,tuner,tuner_view}.rs` | same |
| Golden | `tests/dsp_golden.rs` → `tests/golden/dsp_golden.u32` (no model, A1, LSTM, A2, gain ramps, model swap crossfade) | `RESONANCE_BLESS=1` or `RESONANCE_BLESS_DSP_GOLDEN=1` |
| Benches | `benches/{amp_dsp,matvec}.rs` | `cargo bench -p resonance-amp` |

One `#[ignore]` diagnostic in `nam_a2_reference_parity.rs` compares any
model against a reference render given by `NAM_DIAG_MODEL`/`REF`/`INPUT`.
Not covered: A2-Lite reduced sizes (reference vectors are kept for later).

## Decisions and deviations

- D5: a missing model plays dry rather than silence.
- The parser rejects unknown architectures instead of guessing.
- `condition_dsp` is held as raw JSON in `WaveNetFullConfig` and built into
  a nested WaveNet in `nam/wavenet/model/build.rs`; the field's comment
  ("until the condition_dsp inference todo") predates that inference, which
  is now parity-tested.

## Known gaps

- Runtime A2-Lite (slimmable) size selection, and slimming with a head
  kernel size above 1:
  [nam-a2-lite-sizes](../../intents/plugins-instruments/nam-a2-lite-sizes.md).
- Loudness normalisation across models (`loudness_db` is read from the file
  but unused), and moving the IR plugin onto a shared library, are spec
  non-goals that remain open.
