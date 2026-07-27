# Design: Reference track & A/B monitoring (epic #43)

A **Reference & A/B monitor panel** docked on the right of the Mix view that lets
the user load pro reference tracks, instantly A/B against their mix at matched
loudness, and read comparative loudness side-by-side. The reference plays through
the monitor path only and is excluded from every export.

Prototype: `design/reference-track-ab-monitoring/index.html` (self-contained).

## Where it lives
A right-rail panel in the **Mix** view, sitting where the channel inspector sits
(360px, between `INSPECTOR_WIDTH` 320 and a touch wider for the dual-column
loudness readout). The mixer (channel strips + master strip) stays visible behind
it for context — A/B is a mixing decision, so it belongs next to the meters and
the master/mastering chain it bypasses.

## On-brand foundation
- Tokens mirrored verbatim from `resonance-app/src/theme.rs` (BG_0–BG_3, LINE,
  TEXT_1–4, ACCENT lavender, WARM amber, GOOD/BAD).
- **Domain colour split per ux-guidelines**: the **Mix (A)** source is **lavender
  ACCENT** (the user's own material / selection domain); the **Reference (B)**
  source is **WARM amber** (audio-clip / external-audio domain). This carries
  through the A/B toggle, the comparative columns, and the loudness bars so the
  user always knows which side is which by colour alone.
- Section labels, segmented controls, switches, sliders, and the dashed drop zone
  reuse the patterns already established in the export and import prototypes.

## Screens & states (all in the prototype's state switcher)
1. **Empty** — dashed drop zone: "Drop a reference track", file picker, supported
   formats (WAV/AIFF/FLAC/MP3/M4A), and the promise that it never enters exports.
   Mixer dims behind.
2. **Analyzing (loading)** — after import: decode → waveform overview →
   integrated-LUFS (BS.1770) measurement → match-offset, shown as a checklist with
   a pulsing progress bar. Loudness analysis is what makes loudness-match possible,
   so it's surfaced as an explicit step.
3. **Monitor: Mix (A)** — populated, listening to the user's mix. Full panel:
   reference list, A/B toggle (A lit lavender), waveform, level controls,
   comparative loudness readout.
4. **Monitor: Reference (B)** — same panel, A/B flipped to B (lit amber), now
   auditioning the reference. Clicking the A/B control in any populated state flips
   between 3 and 4.
5. **Error** — undecodable / unsupported file (e.g. a `.mid`), with the offending
   filename and recovery actions (dismiss / choose another).

## Key components & decisions
- **A/B toggle** — large two-button control (A = Your mix, B = Reference) with a
  "● Listening" indicator on the active side. Caption documents the **momentary
  hold-`X`** key and states plainly that the reference **bypasses the master &
  mastering chain** — the whole point of the feature.
- **Reference list** — supports **one or more** references (scope says "one or
  more"); each row shows name + measured integrated LUFS + remove, plus "Add
  reference…". The active reference is highlighted; A/B auditions the active one.
- **Loudness match** — a toggle that level-aligns the active reference's integrated
  LUFS to the mix's, showing the computed offset (e.g. −9.8 → −14.0 LUFS = −4.2 dB)
  so comparisons aren't fooled by volume. Separate manual **Ref trim** slider for
  taste on top of the match.
- **Reference position** — waveform overview with playhead/scrub, user **markers**
  (Drop, Chorus…), and a "Loop to mix" chip to keep both playing in lockstep for
  comparison.
- **Comparative loudness readout** — a Mix-vs-Ref-vs-Δ table over the existing
  metering snapshot fields (`integrated_lufs`, `short_term_lufs`,
  `momentary_lufs`, `true_peak_max_dbtp`, `lra_lu`). True-peak over-0 is flagged in
  BAD pink; meaningful deltas in WARM. A compact dual loudness-bar pair gives an
  at-a-glance level comparison against a shared target line.
- **Exclusion guarantee** — a persistent "Not in exports" header badge plus a
  closing GOOD-toned note reassuring that the slot persists in the project but is
  excluded from renders/bounces/stems/recorded mix (directly serving the
  acceptance criterion).

## Interactions
- Drag-drop or pick → analyzing → populated.
- Click A/B (or hold X) to switch source; active reference selectable from the list.
- Toggle loudness-match; drag Ref trim; scrub waveform; add/jump markers.
- All reference state saves with the project; nothing routes to export/record.

## Notes for the developer
This is a design artifact, not the shipped feature. Reuse the real `theme.rs`
tokens and the existing metering snapshot for the comparative readout. The 360px
right-rail can reuse the inspector container; the meters should read from the same
`MeterSnapshot` pipeline, with a parallel snapshot computed on the reference
monitor tap.
