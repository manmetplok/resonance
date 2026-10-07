# Lane generators and derived clips

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/refactor/A-6-derived-clips.md` (derived-clip map), `docs/history/specs/melody-generation-research.md` + `todo.md` (generator quality).

Each `(section definition, track)` pair is a **lane**. A lane can carry a
generator — Bass, Melody (lead), Pad or Vocal — that turns the section's
chord grid into MIDI. The result is written as a **derived clip**: one MIDI
clip per placement of the section, owned by the compose model and rebuilt
wholesale whenever the lane regenerates. A lane with no generator is
Manual: the user writes its clips by hand.

## Behaviour

- **Kinds.** `LaneGeneratorKind::{Bass(BassParams), Melody(MelodyParams),
  Pad(PadParams), Vocal(VocalParams)}` plus a legacy `Drum(DrumLaneConfig)`
  that only old projects carry (nothing creates it; drums come from the
  [drum arrangement](drum-arrangement.md)). Vocal lanes are covered in
  [vocal lyrics and melody](vocal-lyrics-and-melody.md).
- **Track rules.** Bass/Melody/Pad need a synth instrument track; Vocal
  needs a vocal track; drum tracks are refused (`require_instrument_track`,
  `require_vocal_track`). Frozen tracks are refused by `generate.part`.
- **Styles.**
  - Bass: `RootHold`, `RootPulse`, `RootFifth`, `Octave`, `Walking`,
    `Motif` (with `BassMotifMode` SameIntervals/Augmented/RhythmOnly/
    FirstNoteOnly and `BassMotifPhrase` Simple/MirrorMelody/Restricted).
  - Melody: `ArpUp`, `ArpDown`, `ArpUpDown`, `Motif`; register, note value,
    rest density, velocity, articulation, `ContourPreference`
    (Auto/Arch/Descending/Ascending/Wave), phrase length, and
    `fill_vocal_gaps` (replaces the style with a chord-tone arp that only
    plays in the section's vocal silences, `derive_melody_fill_vocal`).
  - Pad: register and velocity; voiced by the SATB pass
    (`satb_voicings`), see [melody and motif engine](melody-and-motif-engine.md).
- **Shared motif.** Every section has one `motif_source` (generated from
  `MotifParams` or hand-drawn in the chord inspector's manual motif canvas).
  Motif-style bass and melody lanes read it, so lanes in a section share
  one motif identity. "Regenerate motif" re-derives every motif lane
  (`propagate_motif_change`).
- **When lanes regenerate.**
  - Any chord edit (add/edit/move/resize/delete/replace, or a new generated
    progression) regenerates every Bass/Melody/Pad/Vocal lane of the
    section (`propagate_chord_change`). Vocal lanes get new MIDI here but no
    new audio; the user re-renders.
  - The lane inspector's Regenerate bumps the lane seed and re-derives that
    lane. Changing a parameter in the inspector does **not** regenerate by
    itself.
  - A section resize re-derives lanes that already have clips.
  - A lane with no chords produces nothing (`regenerate_lane` returns early).
- **Chord-track overlay.** Before deriving, `apply_chord_track_harmony`
  replaces the symbol of any section chord that falls inside a **pinned**
  global chord-track region, and replaces the section scale with the chord
  track's key at the section's first placement whenever a key exists. So
  once a song key is set (`transport.set_key`), lane regeneration uses it
  instead of `section.set_scale`. See [chord track](chord-track.md).
- **Derived clips.** `ComposeState::derived_clips` maps
  `(definition_id, placement_id, track_id)` → `ClipId`.
  `install_derived_midi_clip` tears down the old clip and installs the new
  one **under the same clip id** (ba doc #275 P1.7), so cached ids stay
  valid across regenerates. Clip name is `"{section} · {track}"`; length is
  the section's bars in the meter at that placement.
- **Hand edits are not protected.** A derived clip is an ordinary MIDI clip:
  `notes.*`, the piano roll and the expanded compose editor can edit it, but
  the next regenerate of the lane replaces its notes without warning.
- **Persistence.** Lane configs persist in `ProjectSectionDefinition::
  lane_generators`; the map persists as `ProjectFile::derived_clips`.
  Projects saved before that field use the positional
  `rebuild_derived_clips` heuristic (a clip starting exactly on a
  placement bar, on a lane-generator track, or a drum clip with id ≥
  `CLIP_ID_BASE`).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Lane config model | `resonance-app/src/compose/lane_generator.rs` | `LaneGeneratorConfig`, `LaneGeneratorKind`, `LaneGeneratorKindTag` |
| Generator bridge | `resonance-app/src/compose/generate.rs` | `derive_notes`, `DeriveKind`, `overlay_pinned_chords`, `to_timed_chords` |
| Regeneration | `resonance-app/src/update/compose/regenerate.rs` | `regenerate_lane`, `propagate_chord_change`, `propagate_motif_change`, `apply_chord_track_harmony` |
| Clip install | `resonance-app/src/update/compose/mod.rs` | `install_derived_midi_clip`, `ClipVisibility`, `control_generate_part`, `control_set_lane_generator` |
| Inspector handlers | `resonance-app/src/update/compose/lane_inspector/` | `set_generator`, `bass_params`, `melody_params`, `pad_params`, `common::bump_lane_seed` |
| Derived-clip map | `resonance-app/src/compose/state.rs` | `derived_clips`, `rebuild_derived_clips`, `derived_clips_for_file` |
| Generators | `resonance-music-theory/src/derive/` | `derive_bass`, `derive_bass_motif`, `derive_melody`, `derive_motif_melody_with_section`, `derive_pad` |
| Views | `resonance-app/src/view/compose/lane_inspector/instrument/`, `tracks/`, `expanded_editor/`, `manual_motif_canvas.rs` | right-rail inspector, lane rows, expanded piano roll |
| Control handlers | `resonance-app/src/update/control/generate.rs`, `section.rs` | `part`, `build_kind`, `generated_clips`, `set_lane_generator`, `build_lane_config` |

Invariant: anything that deletes a placement, a track or a section must
also drop its `derived_clips` entries and the clips (see
`purge_placement_outputs`, `forget_track`).

## Surfaces

- **GUI:** Compose tab → select a track lane → right-rail inspector:
  generator pick list (Manual/Bass/Melody/Pad/Vocal), per-kind params,
  Regenerate. Double-click a lane to open the expanded editor.
- **Control API / MCP:**
  - `generate.part` → `mcp__resonance__generate_part`: `role` pad/bass/lead,
    optional `seed` (default `seed_from_id(section)`) and `options`
    (deserialized into the role's params). Installs the generator **and**
    derives now; returns `clip_id` + `clip_ids` (one per placement, in
    arrangement order). Refuses `chord_count`, `beats_per_chord`,
    `sevenths` and sections with no chords.
  - `section.set_lane_generator` → `section_set_lane_generator`: install or
    clear (`manual`) a generator without deriving; default seeds match the
    GUI's per-kind seeds.
  - Read: `song.tracks`, `song.notes`, `song.sections`.
- **Agent skills:** `arranging`, `song-structure`.

## How it's tested

| What | Where | Run |
|---|---|---|
| `generate.part` / lane generator over control | `resonance-app/tests/control/control_generate.rs`, `control_generate_after_load.rs`, `control_lane_generator.rs` | `./scripts/run-tests.py -p resonance-app` |
| Generator fields on sections | `tests/compose/generator_section.rs` | same |
| Pinned chord overlay | `tests/compose/compose_pinned_chords.rs` | same |
| Derived clip ids | `tests/compose/compose_derived_clip_ids.rs` | same |
| Resize re-derive | `tests/compose/section_resize_rederive.rs` | same |
| Expanded editor | `tests/compose/expanded_editor_clip_offset.rs`, `expanded_editor_key_focus.rs` | same |
| Generators themselves | `resonance-music-theory/tests/` (see [music-theory library](music-theory-library.md)) | `./scripts/run-tests.py -p resonance-music-theory` |

Not covered: what happens to hand edits on a derived clip at regenerate
(they are lost, untested either way); `fill_vocal_gaps` end to end through
the app.

## Decisions and deviations

- Inspector param changes do not auto-regenerate; only Regenerate and
  chord edits do. This keeps slider drags from rewriting clips per tick.
- `generate.part` uses `seed_from_id(section)` for every role, while the
  GUI and `section.set_lane_generator` use per-kind multipliers; the same
  section can therefore give different material from the two paths unless
  a seed is passed.
- `generate.part` replaces the lane config wholesale: omitted `options`
  reset the params to defaults.
- `SectionDefinitionState::generate_params` is kept only to load old
  projects (`migrate_old_generate_params`); new code reads
  `lane_generators`.

## Known gaps

- A new placement stays empty until the lane regenerates —
  [section-placement-editing](../../intents/compose/section-placement-editing.md).
- Hand edits to derived clips are silently overwritten by the next
  regenerate (no lock/detach). No intent yet.
- Melody `EmbellishmentStyle` has no GUI control —
  [embellishment-style-ui](../../intents/compose/embellishment-style-ui.md).
- Lead and vocal climaxes are not coordinated —
  [cross-lane-climax-separation](../../intents/compose/cross-lane-climax-separation.md).
