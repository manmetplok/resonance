# Drum kit preview, folder-of-WAVs import and an editable note map

> Status: **designed**
> Area: plugins-instruments
> Source: `docs/history/specs/drums-plugin-rework.md` slice K10 (deferred,
> §6.5, §12); related request
> `docs/intents/control-api/drum-pad-map-mcp.md`.

## Why

Users can only use kits that already come with a `drum_samples.json`
manifest (plok.org kits or a copy of one). A folder of one-shot WAVs, the
most common form of a free drum kit, cannot be imported. Choosing between
installed kits means loading each one, which for a multi-GB kit takes a
while. And the pad ↔ note map is read-only: a kit whose pieces should sit on
other notes, or a user who wants a piece on a different pad, has no way to
change it, in the editor or over MCP.

## What it would do

- **Preview ▶** in the Library overlay's detail pane decodes only the kit's
  kick, snare and closed hat (top velocity, first take) and plays a short
  pattern through the existing audition hook, without loading the kit.
- **Import a folder of WAVs** generates a manifest: one piece per file or
  per file-name group, velocity layers and round robins guessed from name
  patterns, pieces mapped to GM pads by name, with a review step before the
  copy into the library.
- **Editable note map** on the Setup tab: move a piece to another pad or
  note, saved as `_meta.pads` overrides in the kit's sidecar or the plugin
  state, and readable by agents (the pad-map feature request).

Done when each of the three works from the editor, survives a project
reload, and the generated manifest loads like a plok.org kit.

## What exists to build on

- `resonance-common/src/drumkit_library.rs` and `drumkit_library/install.rs`
  (staging copy with progress, cancel and disk check; `find_manifest`
  requires an existing manifest today).
- `resonance-common/src/drumkit_library/manifest.rs` (`_meta.pads` with
  `note`, `port`, `choke`), `plugins/resonance-drums/src/pad_map.rs`
  (resolution order `_meta` → Drummica table → absent).
- The audition hook in `plugins/resonance-drums/src/lib.rs` and the editor's
  Setup tab pad table (`editor/setup_tab.rs`, read-only).
- `docs/features/plugins-instruments/drums-plugin.md`,
  `docs/features/plugins-instruments/drum-kit-library.md`.

## Open decisions

1. Where a user's note-map edits live: in the kit's sidecar (shared by every
   project) or in plugin state (per instance). Recommended: plugin state,
   with "save to kit" as an explicit action, so editing one project cannot
   change another.
2. How much the WAV importer guesses. Recommended: name-pattern rules plus
   a review table the user can correct before import, no audio analysis in
   v1.
3. Whether Preview needs its own voice path. Recommended: reuse audition
   with a temporary three-piece kit.

## Slices

1. Preview ▶ (overlay button, decode three pieces off-thread, audition).
2. Editable note map in plugin state + `kit_info` reporting the resolved
   map + `drum_kits.list` / a pad-map field for agents (the feature
   request's proposal 1).
3. Folder-of-WAVs import with manifest generation and review.

## How we'd know it works

Tests in `plugins/resonance-drums/tests/` (library overlay, kit pads,
state) for each slice; a generated manifest round-trips through
`Library::import` and `pad_map`; an app control test reads the map.
