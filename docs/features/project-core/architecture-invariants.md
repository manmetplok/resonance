# Architecture invariants test

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/refactor/arch-migration-plan.md` (ARCH-10),
> `docs/history/reviews/code-review-2026-10-02.md` (ARCH2-02/03/05/07/12).
> The rules themselves: [`ARCHITECTURE.md`](../../../ARCHITECTURE.md) (stays in
> the repo root as the source of the layering rules) and
> [`CLAUDE.md`](../../../CLAUDE.md) → Tests.

`tools/arch-invariants` turns the workspace's structural rules into failing
tests: the crate layering, which crates may use which GUI stack, the test
layout, logging and locking rules, and a few grep-shaped boundaries inside
the app. A rule that is only prose erodes one PR at a time; these fail the
suite instead.

## Behaviour

- The crate links nothing from the workspace. It reads `cargo metadata
  --no-deps --offline` and walks the source tree, so it builds in seconds.
  `src/lib.rs` is empty; everything is in `tests/architecture.rs` (31 tests).
- Source rules run on comment-stripped lines (a small lexer that honours
  string and char literals; it has its own self-test).
- Each test quotes the sentence it enforces and records how it was
  exercised once (rule broken → test failed → reverted).

### What it enforces

| Area | Tests |
|---|---|
| Crate layering | `crate_dag_matches_architecture_md` (every internal dependency must be an edge in `allowed_internal_deps`, the table mirroring ARCHITECTURE.md's diagram); `every_crate_is_named_in_the_layering_section`; `test_support_crates_are_dev_dependencies_only` |
| GUI stacks | `framework_crates_stay_where_the_diagram_puts_them` (`iced_*`, `egui_*`, `winit`, `wgpu`, `eframe` only in their crates); `plugins_never_name_a_platform_runtime` |
| Plugins | `plugins_reach_only_common_utilities` (`PLUGIN_COMMON_ITEMS`), `only_listed_plugins_depend_on_resonance_common` (`PLUGINS_ON_COMMON`), `plugins_disable_default_features_on_resonance_common`, `plugin_dirs_and_workspace_members_agree`, `plugins_draw_knobs_only_through_the_param_binding`, `first_party_clap_ids_are_spelled_once` |
| Test layout | `app_test_binaries_are_the_known_groups` (exactly compose, control, io, midi, mixer, performance, plugins, timeline, vocal + `e2e_compose_via_control`, `hermetic_construction`); `audio_test_binaries_are_the_known_groups` (7 groups + 5 process-global standalones); `test_binary_counts_only_grow_deliberately` (per-crate ratchet `TEST_BINARY_CAPS`); `no_inline_test_modules_outside_the_documented_exception` |
| Logging | `library_crates_log_through_tracing_not_stderr`; `audio_callback_never_logs` |
| Engine | `engine_and_mixer_take_no_state_lock` (no `RwLock` state in `engine/`/`mixer/`); `engine_common_public_fns_dont_return_result_string`; `every_audio_command_has_an_app_caller`; `plugin_chain_commands_and_events_take_a_chain_owner` |
| App boundaries | `view_layer_never_reads_the_engine`; `state_module_imports_nothing_from_view_update_or_the_socket`; `control_handlers_never_send_engine_commands`; `wire_protocol_types_stay_in_the_control_layer`; `control_view_model_has_no_wildcard_arms`; `undo_classification_has_no_catch_all_arms` |
| View style | `view_text_never_shrinks_below_the_9px_floor`; `view_layer_never_hardcodes_pure_white_or_black` |
| File size | `mod_rs_files_only_shrink` (a `mod.rs` over 200 lines must be on `MOD_RS_ALLOWANCE`, whose caps only come down) |
| Self-test | `comment_stripper_honours_string_literals` |

Ratchets (`TEST_BINARY_CAPS`, `MOD_RS_ALLOWANCE`) fail in both directions: a
count above its cap fails, and a count below it asks you to lower the cap.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Tests | `tools/arch-invariants/tests/architecture.rs` | the 31 tests above; `allowed_internal_deps`, `PLUGIN_COMMON_ITEMS`, `PLUGINS_ON_COMMON`, `TEST_BINARY_CAPS`, `MOD_RS_ALLOWANCE`, `code_lines`, `strip_line_comment` |
| Crate | `tools/arch-invariants/{Cargo.toml,src/lib.rs}` | empty lib, `serde_json` dev-dependency |

A new crate needs a row in `allowed_internal_deps` and a place in
ARCHITECTURE.md's layering section; making that decision is the point.

## Surfaces

- Developers and agents only: `cargo test -p arch-invariants`, and it runs
  as part of `./scripts/run-tests.py`.

## How it's tested

| What | Where | Run |
|---|---|---|
| The rules | `tools/arch-invariants/tests/architecture.rs` | `cargo test -p arch-invariants` |
| The lexer | `comment_stripper_honours_string_literals` | same |

Not covered: the "build test apps with `new_for_test`" rule and the "goldens
are blessed on this machine" rule from `CLAUDE.md` are not checked.

## Decisions and deviations

- Grep-shaped, not type-aware: rules match source text, so a determined
  workaround (a re-export, a macro) can slip past. The comment stripper was
  added after a `//` inside a URL string hid the rest of a file.
- The MCP/skill lockstep rule lives in
  `resonance-mcp/tests/agent_plugin_lockstep.rs`, not here.

## Known gaps

- The Epic D done-when check (no `next_*_id` counters in
  `resonance-audio/src/engine`) is not a test yet; it lands with D-7f. See
  [finish-app-owned-ids](../../intents/project-core/finish-app-owned-ids.md).
