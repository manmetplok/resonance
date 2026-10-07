# Global chord track and song key

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none archived; code cites epic #33 / doc #168 (todos #439–#445).

Next to the per-section chord grids there is one song-wide **chord
track**: chord regions placed on the timeline in samples, plus a key
context (a song key and optional key changes). Its job is to let the user
**pin** harmony that Compose regeneration must follow, and to hold the
song key that `transport.set_key` sets. It is metadata only; nothing in it
reaches the audio engine.

## Behaviour

- **Model.** `ChordTrack { regions, key_changes }`. A `ChordRegion` has an
  id, a `Chord`, `[start_sample, end_sample)` and `pinned` (default false).
  A `KeyChange` has a start sample and a `Scale`; the first one is the song
  key. Both lists stay sorted; regions never overlap. Ids come from
  `ComposeState::fresh_id`.
- **Edits.** `ChordTrackMessage` covers add-at-playhead, add region,
  set symbol, move start, set end, delete, toggle pin, set song key, insert
  / move / delete key change. Positions snap to the timeline grid; moves
  clamp against neighbours. Each edit records one undo entry.
- **Add at playhead.** Snaps the playhead; refuses a second region at the
  same start; inside an existing region it splits it; in free space it
  spans one bar or up to the next region. The chord is the tonic triad of
  the key in effect there (major / minor / diminished by mode), C major
  with no key.
- **Effect on Compose.** Before any lane derives (`apply_chord_track_harmony`):
  1. each section chord whose start beat lies inside a **pinned** region
     takes that region's chord (rhythm unchanged); unpinned regions never
     override;
  2. if the chord track has any key, the key at the section's first
     placement **replaces the section's own scale** for that derive.
  A section placed several times is read at its first placement; an
  unplaced section is left alone. Progression generation in the chord
  inspector still uses the section's own scale.
- **Display.** The Arrange view's global-tracks shelf draws the chord lane
  (section chords as blocks, chord-track regions on top, overridden chords
  marked) — `view/timeline/draw/chord_lane.rs`.
- **Persistence.** `ProjectFile::chord_track` (`ProjectChordTrack`); undo
  restores it through the `ChordTrack` reconcile domain.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Model | `resonance-app/src/chord_track.rs` | `ChordTrack`, `ChordRegion`, `KeyChange`, `key_at`, `song_key`, `region_at`, `resort` |
| Update | `resonance-app/src/update/chord_track.rs` | `ChordTrackMessage`, `handle`, `tonic_chord` |
| Compose overlay | `resonance-app/src/compose/generate.rs`, `update/compose/regenerate.rs` | `overlay_pinned_chords`, `apply_chord_track_harmony` |
| Palette commands | `resonance-app/src/commands/{mod,resolve}.rs` | `AddChordAtPlayhead`, `DeleteChordAtPlayhead`, `ToggleChordPinAtPlayhead`, `chord_at_playhead` |
| View | `resonance-app/src/view/timeline/draw/chord_lane.rs`, `view/track_header/shelf.rs` | `draw_chord_lane` |
| Control | `resonance-app/src/update/control/transport.rs` | `transport.set_key` → `ChordTrackMessage::SetSongKey` |

## Surfaces

- **GUI:** command palette "Add Chord at Playhead", "Delete Chord at
  Playhead", "Pin / Unpin Chord at Playhead"; read-only drawing in the
  Arrange global-tracks shelf. There is no GUI to change a region's
  symbol, move or resize it, or edit key changes.
- **Control API / MCP:** `transport.set_key` →
  `mcp__resonance__transport_set_key` sets the song key. No method reads
  or edits chord regions or pins.
- **Agent skills:** none use the chord track directly.

## How it's tested

| What | Where | Run |
|---|---|---|
| Model invariants | `resonance-app/tests/compose/chord_track.rs` | `./scripts/run-tests.py -p resonance-app` |
| Update handlers | `tests/compose/chord_track_messages.rs` | same |
| Pinned overlay + key adoption | `tests/compose/compose_pinned_chords.rs` | same |
| `transport.set_key` | `tests/control/control_transport.rs` | same |
| Goldens | `tests/snapshots/global_tracks_shelf_chord_track_regions-wgpu.png`, `global_tracks_shelf_chord_track_error-wgpu.png` | `RESONANCE_BLESS=1` |

Not covered: the interaction between a song key and `section.set_scale`
from an agent's point of view (setting both silently uses the song key).

## Decisions and deviations

- Positions are samples, not bars, to match the playhead, loop range and
  tempo/signature tracks; bar/beat is derived for display.
- Only pinned regions constrain regeneration, so a chord track can be
  sketched without rewriting existing parts.
- The chord sheet PDF ignores the chord track and reads section chords only
  (see [chord sheet export](chord-sheet-export.md)), although
  `ChordTrack::is_empty`'s comment says the PDF falls back to sections
  "while the track is empty".

## Known gaps

- Regions can only be added as tonic chords, deleted and pinned; no GUI or
  control path edits symbols, spans or key changes —
  [chord-track-editing](../../intents/compose/chord-track-editing.md).
- A song key overrides every section's scale for lane regeneration with no
  warning; agents setting `section.set_scale` after `transport.set_key`
  see no effect on generated parts. Covered by the same intent.
