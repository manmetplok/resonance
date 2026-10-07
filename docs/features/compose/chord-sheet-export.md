# Chord sheet PDF export

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none archived; code cites ba todo #1390 (header).

The user can export a printable A4 PDF of the song's chords with
vector-drawn diagrams for guitar, bass and keyboard, to take to rehearsal
or hand to a player. It reads the Compose section chords.

## Behaviour

- **Content.** One page group per instrument, in this order: Guitar
  6-string, Guitar 8-string, Bass 4-string, Bass 5-string, Keyboard. Each
  group shows, per section, a header (`"{name} ({bars} bars) - {scale}"`),
  a scale diagram (12-fret fretboard, or a two-octave keyboard), and the
  section's chord diagrams eight per row.
- **Which chords.** Sections in arrangement order (first placement), then
  unplaced sections; per section, its distinct chords in beat order
  (duplicates dropped). Sections with no chords are skipped. With no chords
  at all the PDF is one page saying "No chords to display."
- **Voicings.** Fretboard shapes come from
  `resonance_music_theory::fretboard_voicing` over the tunings `GUITAR_6`,
  `GUITAR_8`, `BASS_4`, `BASS_5` (four-fret window, open/muted markers,
  root dot highlighted).
- **Header.** "Chord Sheet" title and an info line `Tempo: N BPM | n/d`
  from the first tempo and signature events, with " (changes)" appended
  when the song changes tempo or meter (`SongHeader::from_song`).
- **Saving.** A native save dialog (`rfd`) defaulting to `chords.pdf` in
  the projects directory. A write failure shows an error banner.
- The global [chord track](chord-track.md) is not read.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| PDF build | `resonance-app/src/chord_sheet_pdf.rs` | `build_chord_sheet_pdf`, `SongHeader`, `collect_section_chords`, `render_fretboard_pages`, `render_keyboard_pages` (printpdf) |
| Diagram layout | `resonance-app/src/chord_box.rs` | `layout`, `Dims`, `Marker`, `Nut` — backend-agnostic, also used by the Performance view (`view/performance/center_stage.rs`, `next_lane.rs`) |
| Fretboard theory | `resonance-music-theory/src/fretboard.rs` | `voicing`, `Tuning`, `ALL_TUNINGS` |
| Trigger + save | `resonance-app/src/update/project_io/mod.rs`, `dialogs.rs` | `ProjectIoMessage::ExportChordSheet`, `ChordSheetPathSelected`, `chord_sheet_header`, `chord_sheet_dialog` |

## Surfaces

- **GUI:** Compose tab section strip "Export chords" button (shown once a section exists); command
  palette "Export Chord Sheet…" (aliases "pdf", "lead sheet").
- **Control API / MCP:** none.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Header line | `resonance-app/tests/io/chord_sheet_header.rs` | `./scripts/run-tests.py -p resonance-app` |
| Diagram geometry | `resonance-app/tests/compose/chord_box_layout.rs` | same |
| Fretboard voicings | `resonance-music-theory/tests/fretboard.rs` | `./scripts/run-tests.py -p resonance-music-theory` |

Not covered: the PDF bytes / page contents as a whole (no golden PDF), and
the save dialog.

## Decisions and deviations

- `chord_box` was factored out of the PDF code so the Performance view
  draws the same diagrams on an Iced canvas; the PDF output was kept
  byte-for-byte identical through that refactor.
- The header prints the opening tempo/meter and flags changes instead of
  listing them.

## Known gaps

- No control method, so agents cannot produce a chord sheet.
- Pinned chord-track harmony is not reflected in the sheet.
