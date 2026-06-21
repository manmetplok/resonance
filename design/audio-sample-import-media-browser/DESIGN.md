# Design: Audio & sample import with a media browser (epic #35)

Prototype: `design/audio-sample-import-media-browser/index.html` (self-contained; open
directly). State switcher at top toggles every screen/state.

## Intent

Give Resonance its first path for bringing external audio into a project — drum loops,
one-shots, vocal takes, stems, reference tracks. The surface is a **docked media-browser
side panel** in the Arrange view (not a modal), plus **drag-to-timeline** placement, an
**open-file import** path, and the supporting **transcode/copy** and **missing-file relink**
flows. Built to the existing dark, lavender-accented pro-audio language in `theme.rs` /
`ux-guidelines.md`: audio domain uses **WARM amber**, lavender stays reserved for
selection/brand, in-clip waveforms reuse the existing clip rendering vocabulary.

## Key decisions

- **Browser is a docked left panel**, mirroring the Compose right-rail / Mixer inspector
  pattern (fixed-width column, `--browser-w` 312px, `BG_2`, `LINE` right border). Toggled
  from a "Media" chrome button (active = `ACCENT_DIM`). It is a peer of the timeline, so a
  user can browse, audition, and drag without losing the arrangement context.
- **Two tabs: Files / Pool.** *Files* browses the filesystem (breadcrumb + favourites/recent
  shelf + per-folder filter). *Pool* is the project's set of already-imported assets, each
  with a usage count (`used ×N` / `unused`) so the user can see what's referenced. The Pool
  tab hides the filesystem breadcrumb (it's a project-level list, not a path).
- **Favourites & recent** shelf as pill chips (favourite = WARM star, recent = clock),
  satisfying "remember recent folders / favourite locations." Star toggle also lives on the
  breadcrumb row to favourite the current folder.
- **Audition transport pinned to the bottom of the panel** — play/stop, scrub waveform with
  playhead, time readout, and three toggles: *Auto-play on select*, *Loop*, *Sync to tempo*
  (time-stretch a loop to project BPM for preview). Auditioning plays through the engine; the
  active row also shows a WARM `playing` highlight.
- **Audio domain = WARM.** File rows show a type glyph, a mini waveform thumbnail, a format
  chip (wav/flac/mp3/ogg, lightly color-coded), and duration. Imported clips on the timeline
  reuse the existing audio-clip card (WARM wash + in-clip waveform).
- **Drag-to-timeline** is the primary placement gesture (per "direct manipulation"): a drag
  pill follows the cursor, the target lane lights with `ACCENT_LINE`, a dashed **ghost clip**
  previews the drop position (snapped to grid), and a tooltip states the target track + bar +
  any conversion (`→ 48 kHz`). A dashed **"create a new audio track"** zone appears below the
  last lane so a drop can spawn a track.
- **Open-file import** path via the chrome "Import audio…" button (and OS file dialog) for
  multi-file import, complementing drag-and-drop.
- **Transcode/copy on import.** Imported files are copied into `*.rproj/audio/` and converted
  to the engine format (channel up/down-mix + resample to the project rate) so projects stay
  self-contained and relocatable, and mismatched sample rates play at correct pitch/speed.
  Progress modal shows per-file status (done / working / queued) with an explanatory note.
- **Missing-file relink.** On reload, unresolved references surface (a) inline in the Pool as
  a `missing` row with a `relink` chip, and (b) a relink modal listing all missing files with
  per-file *Locate…* plus a one-shot **"Search a folder…"** that resolves every missing file
  by name. Clips are preserved offline; relinked audio is copied back into the project folder.

## Screens & states (all in the prototype switcher)

| State | Covers |
|-------|--------|
| **Browser · files** | Populated filesystem browse: breadcrumb, favourites/recent, folders + audio rows w/ waveform thumbnails, format chips, durations. |
| **Auditioning** | A row playing through the engine; audition transport active with scrub playhead + time. |
| **Project pool** | Imported assets with usage counts, an unused asset, and a missing/relink row. |
| **Empty folder** | Folder with no audio — empty-state copy + "Choose files…". |
| **Drag → timeline** | Drag pill, lit target lane, dashed ghost clip + drop tooltip, new-audio-track drop zone. |
| **Import / transcode** | Loading: per-file copy + decode + resample/channel progress, project-folder note. |
| **Missing files** | Error/recovery: relink modal, per-file Locate + batch folder search. |

Empty / loading / populated / error are all represented.

## Notes for the architect / developer

- This is a **design artifact**, not the shipped feature. The real panel is Iced; reuse
  `theme.rs` tokens and layout constants (add a `BROWSER_WIDTH` constant rather than inlining
  312px), the collapse-caret pattern (`view::controls::collapse_caret`), and the existing
  clip-waveform renderer (`timeline/draw.rs::draw_clip_waveform`, `AudioClip.waveform_peaks`).
- Engine already exposes `transcode_to_wav` (wav/flac/mp3/ogg) per the epic — wire copy +
  resample to the project rate through it.
- Persistence: pool references + clip placement go through replay-diff + undo; auditioning is
  transient UI state (not undoable, not persisted), same rule as collapse state.
- Per the view-performance rules: the audition scrub/playhead is a live readout — keep it out
  of any `lazy` region whose fingerprint omits it; cache the file-list / pick_list options.
