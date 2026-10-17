# Project file format

> Verified against master @ `cb3b1d6a` on 2026-10-07.
> Also verified: pronunciation persistence.
> Spec history: none as one spec; the format grew field by field. Leads:
> `docs/history/refactor/arch-migration-plan.md` (ARCH-01 A1-2 folded every
> undo side-car into the file), `docs/history/reviews/code-review-todo.md`
> (STATE-06, STATE-08, STATE-11, STATE-12).

A Resonance project is a directory bundle (`<Name>.rproj/`) holding one JSON
document that describes the whole song plus the audio, MIDI and plugin-state
files it points at. The same shape is used for a manual save, an autosave
snapshot, a versioned backup, a user template and every undo snapshot, so
one model (`ProjectFile`) is the single description of "the song".

## Behaviour

### Bundle layout

```text
Song.rproj/
  project.json                  canonical document (manual save)
  project.autosave.json         autosave snapshot (never overwrites project.json)
  autosave/midi/, autosave/plugins/   the autosave's own side files
  audio/clip_<id>.wav           audio clips and audio takes (32-bit float)
  audio/asset_<id>.wav          media-pool assets (imports)
  audio/<rendered vocal takes>  vocal-synth renders
  midi/clip_<id>.mid            Format-0 SMF per MIDI clip (fallback copy)
  plugins/plugin_<id>.bin       opaque CLAP state blob per plugin instance
  backups/project-<ts>.json     versioned backups of project.json
  backups/project-<ts>.files/   hard-linked MIDI/plugin files a backup owns
  .session.lock                 crash-detection marker (see autosave doc)
Song.freeze/                    freeze caches: a SIBLING of the bundle, not inside
```

- Audio WAVs are written by the engine (recordings stream there while
  recording; imports, renders and `PersistClipWavs` write them later) and are
  memory-mapped at load. A clip's WAV is written once and never rewritten.
- MIDI notes are stored **inline** in `project.json` (`ProjectMidiClip.notes`,
  lossless, STATE-06). The `.mid` file is still written and is only read when
  the inline copy is absent (projects saved before it existed).
- Plugin state is an opaque blob per instance, plus a list of parameter
  values that differ from the plugin's defaults (`ProjectPlugin.params`), so
  a reopened project can reproduce its own bounce without re-reading params
  from the plugin.
- Every file write goes through `resonance_common::atomic_write` (unique temp
  name, rename into place).

### What `ProjectFile` holds

Transport scalars (rate, BPM, meter, metronome, loop range, loop-record
mode), master volume/chain/bypass, tracks (with plugin chains, routing,
freeze state, colour, external-instrument config), audio clips (with fades,
gain, warp), MIDI clips (notes, vocal lyrics), busses, aux sends, sidechain
routes, compose section definitions and placements, the derived-clip map,
tempo and signature events, MIDI clock settings, drum pattern bank, track
groups, references and their settings, arrangement markers, pool assets,
groove library and quantize settings, automation lanes, MIDI bindings,
performance-mode footer state, take groups, the chord track and the vocal
pronunciation dictionary + per-syllable overrides (`pronunciation`, left out
when empty; see [pronunciation](../compose/pronunciation.md)). The model
types for takes, automation, MIDI maps, freeze, track groups and external
instruments come from `resonance-common` (behind its default `model`
feature).

### Versioning

- `PROJECT_FORMAT_VERSION = 2`. The loader refuses a file whose `version` is
  **higher** ("newer than this build"). It does not check for older versions;
  v1 (inline `.raw` samples) was a hard break with no migration.
- Forward growth is additive: new fields are `#[serde(default)]` (and a few
  `alias`es, e.g. `punch_in` → `loop_in`), so older v2 files load with the
  default. There is no migration framework; a legacy shape is converted on
  load by the domain that reads it (e.g. legacy `drum_groups` are promoted
  into the `drum_patterns` bank).
- Hand-edited values are sanitised where it mattered in practice: a `null`
  or out-of-range BPM deserialises to a valid tempo; warp markers are
  clamped, sorted and capped.

### Loading

`load_project(path)` accepts a bundle directory, or a `*.json` file directly
(`project.json`, `project.autosave.json`, or a backup in `backups/` — a
backup resolves its relative paths against the bundle root). Missing MIDI
files load as empty clips; missing plugin blobs are logged and skipped. The
result is a `LoadedProject { file, project_dir, midi_notes, plugin_states }`.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Model | `resonance-app/src/project/model.rs` | `ProjectFile`, `ProjectTrack`, `ProjectClip`, `ProjectMidiClip`, `ProjectPlugin`, `LoadedProject`, `SaveCollector`, `PROJECT_FORMAT_VERSION`, `PROJECT_JSON`, `AUTOSAVE_JSON`, `clip_audio_file` |
| Sections | `resonance-app/src/project/sections.rs` | `ProjectSectionDefinition`, `ProjectSectionPlacement`, `ProjectDerivedClip` |
| Disk I/O | `resonance-app/src/project/io.rs` | `save_project`, `save_autosave`, `write_project_metadata`, `load_project`, `write_backup`, `list_backups` |
| App state → file | `resonance-app/src/update/project_io/serialize.rs` | `build_project_file`, `plugin_states_for_save`, `add_session_plugin_params` |
| File → app state | `resonance-app/src/update/project_io/reconcile/` | see [reconcile.md](reconcile.md) |
| Shared model types | `resonance-common/src/{take,automation,midi_map,freeze,track_group,external_instrument}.rs` | `TakeGroup`, `AutomationLane`, `MidiBinding`, `TrackFreezeState`, `TrackGroup` |
| Atomic write | `resonance-common/src/atomic_file.rs` | `atomic_write` |

Invariants: `build_project_file` must stay pure (no I/O, no engine
commands). Anything persisted and undoable must live in `ProjectFile`; there
is no side-car undo state any more (guarded by the undo fixed-point test).

## Surfaces

- **GUI / Control API:** the file is written and read through
  [save-open-new.md](save-open-new.md) and
  [autosave-and-recovery.md](autosave-and-recovery.md).
- **Agent skills:** none read the file directly.

## How it's tested

| What | Where | Run |
|---|---|---|
| Atomic writes | `resonance-app/tests/io/project_atomic_write.rs` | `./scripts/run-tests.py -p resonance-app` |
| Lossless MIDI round-trip | `resonance-app/tests/io/midi_clip_lossless_roundtrip.rs` | same |
| Per-domain persistence | `tests/io/{take_lanes_persistence,clip_warp_persistence,chord_track_persistence,pool_persistence,loop_record_mode_persistence,project_track_groups_persist,track_color_persist,reference_disk_round_trip}.rs`, `tests/control/control_vocal_pronunciation_persistence.rs` | same |
| File ⇄ state fixed point | `resonance-app/tests/io/undo_snapshot_fixed_point.rs` | same |

Not covered: loading a file with a lower `version` than 2, and a fuzz /
corpus test of hand-edited or truncated `project.json` files.

## Decisions and deviations

- Notes inline in JSON and also as `.mid`: the `.mid` reader is lossy, so the
  inline copy is the truth; the `.mid` stays for interchange.
- The freeze cache sits beside the bundle so saves never copy large freeze
  WAVs and the cache can be deleted as a build artefact.
- The reference A/B monitor state is saved with the project but is not undo
  state (`ProjectReferenceSettings::clear_monitor_state` strips it from undo
  snapshots).

## Known gaps

- No explicit migration step for old versions; growth relies on serde
  defaults.
- Untitled projects render vocal takes into `std::env::temp_dir()/resonance_vocal`
  (`vocal_audio_dir(None)`), not into the session's scratch dir.
