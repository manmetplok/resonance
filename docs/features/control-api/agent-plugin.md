# Agent plugin (`resonance-agent-plugin`)

> Verified against master @ `f9438e38` + `render.stems`, the singing intelligibility script and the 0.6.0
> chord-change melody edit on 2026-10-13.
> Spec history: `docs/history/specs/warmth-width-depth.md` §8 (keys
> blocks, the `spatial` skill); the plugin's own `README.md` is the
> maintained guide and stays in place.

A Claude Code plugin of seven **skills**: written procedures that tell an
agent how to use the MCP tools well for one craft — song structure,
arranging, drumming, singing, mixing, spatial staging, mastering. The tools say
what can be done; a skill says in what order, what a measurement
justifies, and when to stop. Skills load only when a matching task
starts, so they cost no context otherwise.

## Behaviour

### The skills

| Skill | Covers | References |
|---|---|---|
| `song-structure` | choosing a form, sections vs. placements, the `place: false` trap, bar math, tempo and signature tracks | `references/forms.md` |
| `arranging` | chord grids, `generate_part`, hand-written MIDI, density and register | `references/generators.md` (and `forms.md`) |
| `drumming` | the pattern library and hand-written grooves, odd metre | `references/drum-map.md`, `references/grooves.md` |
| `singing` | the vocal synth: voice choice and install (terms, credit), one lane per section, generated or hand-written melody, singable lyrics and the lyric syntax, the `song_vocal` pre-flight, one batched render, expression, an intelligibility check, guide vocals | `references/lyrics.md`, `references/voicebanks.md`, `scripts/intelligibility.py` |
| `mixing` | measure → diagnose → balance → verify: faults, balance, tone, character, dynamics | `references/reading-meters.md`, `character.md`, `roles.md` |
| `spatial` | width and depth: mono-safe width, panning, one shared room, pre-delay, return EQ, ducking | `references/width.md`, `depth.md` (and `reading-meters.md`, `roles.md`) |
| `mastering` | the master chain on `com.resonance.mastering` stage by stage, reference track, delivery targets, stems | `references/delivery.md` (and `reading-meters.md`, `character.md`) |

All seven are model-invocable (`name`, `description`, `when_to_use` in each
`SKILL.md` front matter), and each hands off to the next by pointing at
it. Skills point at each other's references with `${CLAUDE_PLUGIN_ROOT}`.

### The intelligibility script

`skills/singing/scripts/intelligibility.py` is the plugin's one bundled
script, run by the `singing` skill's step 8 as
`${CLAUDE_SKILL_DIR}/scripts/intelligibility.py --text "<lyric>" <wav>…`
on a vocal stem from `render_stems {track_ids: [vocal]}` (or, when the
running app lacks `render.stems`, a soloed `render_mixdown`).

- **Input:** `--text` or `--text-file`; the vocal synth's lyric syntax is
  stripped (`·` / `-` breaks joined, standalone `_` / `+` / `AP` dropped,
  `[..]` phoneme blocks dropped). No manifest.
- **Output:** per file the word accuracy (1 − WER on lowercased,
  punctuation-free words, floored at 0), what Whisper heard and the lyric
  words a minimal alignment misses; `--json` prints
  `{reference, results: [{file, silent, accuracy, heard, missed}]}`.
- **Audio:** Whisper's own 16 kHz load, then leading and trailing silence
  trimmed (20 ms frames within 45 dB of the loudest and above -60 dBFS,
  0.25 s padding); a file with nothing above -60 dBFS is reported silent
  and not transcribed. Long takes go through Whisper's 30 s windowing.
- **Whisper:** `small.en`; when `whisper` is not importable the script
  re-runs itself through `uv` with a CPU PyTorch (the repo script's
  recipe), and says how to install `uv` when it is missing. The model
  downloads to `~/.cache/whisper` on first use.
- **One implementation:** the repo's `scripts/vocal-intelligibility.py`
  (the bank audition's scorer) imports this file by path and uses its
  `word_accuracy`, `transcribe` and `load_model`, so both score a file
  identically. `--self-test` checks the stripping, normalising, scoring,
  missed-word alignment and trimming without Whisper.

### Rules the skills follow

- **Per craft, never per project.** A skill must not name a track, a
  track count or a genre; it reads the song (`song_summary`,
  `song_tracks`, `master_summary`) and branches on what it finds.
- **Preflight with `control_hello`.** Every `SKILL.md` has a
  `## 0. Preflight` that calls `control_hello` and checks the control
  methods it needs against `capabilities`. A missing required method
  means the app is older than the plugin: say so and stop. A missing
  method for a later pass means skip that pass and say why.
- **Name plugin keys inside keys blocks.** A parameter key, choice label
  or factory preset name may appear only inside
  `<!-- keys: <plugin id> --> … <!-- /keys -->`, where it is checked
  against that plugin's real table.

### Installing

The plugin contains **no `.mcp.json`**. Register the server separately
with `claude mcp add --transport stdio resonance -- <path>/resonance-mcp`
so the tools stay `mcp__resonance__<tool>`; bundling the server would
rename every tool to `mcp__plugin_resonance-agent-plugin_resonance__<tool>`.
Load the plugin with `claude --plugin-dir <repo>/resonance-agent-plugin`.

### Versioning

Three things version separately: the skills (`version` in
`.claude-plugin/plugin.json`, 0.6.1 today), `resonance-mcp` (this
checkout), and the running app (whatever the user has open). Living in one
repo pins the first two; `control_hello` covers the third at run time.
Bump the plugin `version` whenever the skills change, or installed copies
keep the old text.

### The lockstep test

`resonance-mcp/tests/agent_plugin_lockstep.rs` fails the suite when:

- `lockstep.json`'s `control_protocol_version` differs from
  `resonance_control::PROTOCOL_VERSION` (so a bump forces a re-read);
- a skill or reference names an `mcp__resonance__<tool>`, or a bare
  `<namespace>_<name>` token, that the router does not publish (a few
  wire field names of that shape are allow-listed and checked against the
  schemas);
- a `${CLAUDE_PLUGIN_ROOT}` / `${CLAUDE_SKILL_DIR}` path does not exist;
- a `SKILL.md` does not call `control_hello`, or its preflight names a
  method not in `capabilities()`;
- a keys block is malformed, or names a plugin that lacks a
  `tests/skill_keys.rs` calling `skill_keys_test!` on that crate and type;
- outside keys blocks, an inline-code span is a param key or preset name,
  or a single word that is not a tool, method, schema/description word,
  plugin id, skill, crate or listed `PROSE_TOKENS` entry;
- decay guidance verifies against bare `t30_seconds` instead of naming
  the 1 kHz band (field report 2026-10-06 §4).

The key tables themselves are checked in each plugin crate, because
`resonance-mcp` may not link plugins:
`plugins/{color,compressor,delay,eq,gate,mastering,reverb,stereo}/tests/skill_keys.rs`
via `resonance-dsp-test-support/src/skill_keys.rs`.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Manifest | `resonance-agent-plugin/.claude-plugin/plugin.json` | `name`, `version` |
| Protocol pin | `resonance-agent-plugin/lockstep.json` | `control_protocol_version` |
| Skills | `resonance-agent-plugin/skills/<skill>/SKILL.md`, `references/*.md` | front matter, `## 0. Preflight`, keys blocks |
| Guide for authors | `resonance-agent-plugin/README.md` | keys-block rules, versioning, bundled scripts |
| Intelligibility script | `resonance-agent-plugin/skills/singing/scripts/intelligibility.py` (shared by `scripts/vocal-intelligibility.py`) | `strip_lyric_syntax`, `word_accuracy`, `missed_words`, `sounding_bounds`, `transcribe`, `self_test` |
| Lockstep test | `resonance-mcp/tests/agent_plugin_lockstep.rs` | `PROSE_TOKENS`, `KEY_SHAPED_WIRE_WORDS` |
| Per-plugin key check | `resonance-dsp-test-support/src/skill_keys.rs`, `plugins/*/tests/skill_keys.rs` | `skill_keys_test!`, `WIRE_WORDS` |

## Surfaces

- **Claude Code:** `/resonance-agent-plugin:<skill>`, or invoked by the
  model when a request matches.
- **Control API / MCP:** any tool; the mixing, spatial and mastering
  skills rely on `meter_*`, `*_set_plugin_param`, `automation_*`; the
  singing skill on `vocal_*`, `song_vocal` and the vocal curve target of
  `automation_*`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Skills match the tool surface and protocol | `resonance-mcp/tests/agent_plugin_lockstep.rs` | `./scripts/run-tests.py -p resonance-mcp` |
| Keys blocks match each plugin's real params and presets | `plugins/<name>/tests/skill_keys.rs` | `./scripts/run-tests.py -p resonance-<name>`; dump with `RESONANCE_SKILL_KEYS_DUMP=1` |
| Intelligibility script: lyric stripping, normalising, WER, missed words, silence trim (no Whisper) | `intelligibility.py --self-test` | `resonance-agent-plugin/skills/singing/scripts/intelligibility.py --self-test` (by hand; not in the cargo suite) |

Not covered: whether a procedure produces a good mix. That needs a live
agent session against a real project, done by hand. The intelligibility
script's Whisper path was checked by hand: on an audition WAV it reports
the same accuracy as `scripts/vocal-intelligibility.py`.

## Decisions and deviations

- Separate sibling skills, not nested ones: Claude Code has no sub-skill
  notion, and "write me a jazz drum part" should not have to load the
  whole arranging procedure.
- The protocol pin lives in `lockstep.json` rather than `plugin.json`,
  because `claude plugin validate` warns on unknown manifest keys.
- Other Resonance skills exist outside this repo, in the user's
  `~/.claude/skills/` (`resonance-studio`, `resonance-synth-design`,
  `industrial-post-metal`, `singer-songwriter`). The lockstep test does
  not see them, so a tool rename can silently break them. The vocal
  procedure moved into the plugin's `singing` skill (0.4.0);
  `resonance-studio`'s `references/vocals.md` now defers to it and keeps
  only studio-specific notes.
- The lockstep test's inline-code check skips spans with a `·` syllable
  break: they are lyric text to type (`re·mem·ber`), not names.

## Known gaps

- Out-of-repo skills are not lockstep-checked (see above).
- The drumming skill's `drum-map.md` documents the built-in GM-ish map;
  an agent cannot read a loaded kit's real pad map over MCP:
  [drum-pad-map-mcp](../../intents/control-api/drum-pad-map-mcp.md).
