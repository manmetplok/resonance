# DSP goldens and skill-key checks (`resonance-dsp-test-support`)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: ba todo #1373 follow-up (golden plumbing);
> `docs/history/specs/warmth-width-depth.md` §8.1 (skill ↔ param lockstep).

A test-only crate with two jobs. It holds the plumbing every plugin's
`dsp_golden.rs` uses to render scenarios, compare them against a binary file
on disk and re-bless that file; and it holds the plugin half of the check
that every param key, choice label and factory preset a skill names is
real. It is a dev-dependency of the 13 plugins; ARCHITECTURE.md forbids it
as a normal dependency.

## Behaviour

### DSP goldens

- **Files.** `plugins/<p>/tests/golden/<name>.f32` (little-endian `f32`
  samples) or `<name>.u32` (raw words; the drums, amp and granular delay
  fold per-block digests into the stream). `golden_path(manifest_dir,
  file)` builds the path from the caller's `CARGO_MANIFEST_DIR`.
- **Bless.** `blessed(&[vars])` is true when any listed variable is
  exactly `"1"`. Every caller includes `RESONANCE_BLESS` and usually a
  narrower one (`RESONANCE_BLESS_DSP_GOLDEN`, `RESONANCE_BLESS_NULL_TEST`,
  `RESONANCE_BLESS_RENDER_BLOCK`, …), so one golden can be re-blessed in a
  wider run. `bless_f32` / `bless_words` write the file and print
  "blessed golden: N samples -> path".
- **Compare.** `load_golden_f32` / `load_golden_words` panic with a re-bless
  hint when the file is missing, and assert the length (a length change
  means the scenario table changed). `compare_f32` returns the differing
  count, first difference, peak and RMS error; `compare_words` returns
  exact differences only. `residual_fraction` measures how much of an
  output is not a scaled copy of the dry input, for "this scenario is not a
  bypass" guards.
- **What stays per crate:** scenario tables, thresholds, bit-exact versus
  epsilon budgets (FFT-based DSP rounds differently per CPU) and extra
  guards such as silence or tail-energy checks. A scenario that renders
  silence would match its golden forever, so each crate guards silence per
  scenario.
- **Goldens on master:** amp `dsp_golden.u32`; color `dsp_golden`,
  `legacy_state`, `tape_hq`; compressor `dsp_golden`, `auto_release`,
  `legacy_state`; delay, gate, IR, stereo `dsp_golden`; drums
  `dsp_golden.u32`; EQ `dsp_golden`, `dynamic_bands`, `legacy_state`,
  `warmth_bands`; granular delay `dsp_regression.u32`; mastering
  `dsp_golden`, `legacy_state`, `w9_golden`; reverb `dsp_golden`,
  `legacy_state`, `return_channel` and one per algorithm (ambience,
  chamber, hall, nonlinear, plate, room, shimmer, spring); wavetable
  `null_test`, `render_block_regression`.
- **Bless policy.** This machine is canonical for goldens. Re-bless only
  when the change is intended; a refactor's goldens are checked against the
  pre-refactor code.

### Skill keys

A skill in `resonance-agent-plugin/skills/` marks a key map with
`<!-- keys: com.resonance.<plugin> -->` … `<!-- /keys -->`. Inside it,
every inline code span must be a param key, a value label of the nearest
key before it in the same table row, list item or paragraph, or a factory
preset name. `{n}` stands for a band index; a declared range `n = A-B`
must match the plugin's band count exactly. Fenced code and nested blocks
are refused. Each plugin runs this against its real param table through
`skill_keys_test!(PluginType)`; `resonance-mcp/tests/agent_plugin_lockstep.rs`
checks the other half (every block names a plugin that carries the test,
and keys appear only inside blocks).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Golden plumbing | `resonance-dsp-test-support/src/lib.rs` | `blessed`, `golden_path`, `bless_f32`, `bless_words`, `load_golden_f32`, `load_golden_words`, `compare_f32`, `compare_words`, `F32Diff`, `WordDiff`, `residual_fraction` |
| Skill keys | `resonance-dsp-test-support/src/skill_keys.rs` | `OPEN`, `CLOSE`, `Surface`, `skill_keys_test!`, `WIRE_WORDS` |
| Users | `plugins/*/tests/dsp_golden.rs` (and `null_test.rs`, `render_block_regression.rs`, per-algorithm reverb tests), `plugins/*/tests/skill_keys.rs` | |

## Surfaces

- **GUI / Control API / MCP:** none.
- **Agent skills:** the keys blocks in `mixing`, `mastering`, `spatial` and
  other skills are what `skill_keys` checks.

## How it's tested

| What | Where | Run |
|---|---|---|
| The skill-key parser | `resonance-dsp-test-support/tests/skill_keys.rs` | `./scripts/run-tests.py -p resonance-dsp-test-support` |
| Per-plugin key checks | `plugins/{color,compressor,delay,eq,gate,mastering,reverb,stereo}/tests/skill_keys.rs` | per plugin |
| A plugin's goldens | e.g. `RESONANCE_BLESS=1 cargo test -p resonance-eq --test dsp_golden` to re-bless | |

The golden helpers themselves have no direct tests; they are exercised by
every plugin's golden test. Not covered: drums, amp, granular delay, IR and
wavetable have no `skill_keys` test, so a skill cannot use a keys block for
them until one is added.

## Decisions and deviations

- The crate holds only mechanism. Nine plugins had copy-pasted the same
  IO, bless check and diff loop; the policy stayed with each plugin
  because it differs on purpose.
- The skill-key check runs in the plugin crates, not in `resonance-mcp`,
  because reading a real param table means linking the plugin, and
  `resonance-mcp` may only depend on `resonance-control`.

## Known gaps

None tracked.
