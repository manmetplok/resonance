# Preset library: what round 2 left out

> Stage: **intent**
> Status: **partly built**
> Area: plugins-instruments
> Source: `docs/history/specs/plugin-preset-library.md` — P4 ("Not built:
> `.rpreset`"), P8 ("Not built: soundpacks, timestamps and extra info";
> `IS_USER_CONTENT` listed as factory), P9 (more kinds) and P10 (publish
> `preset-discovery`).

## Why

The library covers first-party and third-party plugin presets, amp models
and drum kits. Four things a user or another host would expect are
missing:

- **Exporting several presets at once.** Export writes one `.json` per
  preset; the spec's `.rpreset` bundle (zip with a manifest) for a selection
  or filter does not exist, so sharing a bank is manual.
- **Discovered-preset details.** For third-party presets found through CLAP
  `preset-discovery`, soundpacks, timestamps and extra info are dropped,
  and presets the plugin marks as user content are shown as read-only
  factory presets.
- **More kinds.** IR files and track presets do not share stars, tags and
  search with the rest (`library_marks::kind` has `amp-model`,
  `plugin-preset`, `drumkit` only).
- **Other hosts.** Our plugins do not publish their factory presets through
  CLAP `preset-discovery`, so other DAWs show none.

## What it would do

- Export selection/filter as `.rpreset`; import accepts it.
- Discovered presets keep soundpack and timestamps, and user-content
  presets are editable.
- An `ir` kind (IR plugin library) and a `track-preset` kind in the shared
  marks and browser model.
- A `preset-discovery` factory in the SDK serving `FACTORY_PRESETS`.

## What exists to build on

- `resonance-plugin/src/presets/{library,browser,format}.rs`,
  `resonance-plugin/src/library_view.rs` (`LibraryRows` implemented three
  times already: presets, NAM, kits).
- `resonance-audio/src/clap_host/discovery.rs` (indexer and cache).
- `resonance-common/src/library_marks.rs` (`kind`).
- `docs/features/plugins-instruments/plugin-preset-library.md`.

## Open decisions

1. Whether external files (an IR wav, a NAM model) go inside `.rpreset`.
   Recommended: no in v1; list them by path and content hash, as the spec
   says.
2. Whether IRs get a content library like NAM before they become a kind.
   Recommended: yes; reuse `content_index` so slots and identity work the
   same.
3. P10 is optional. Recommended: do it last, only if another host is
   actually used with these plugins.

## Slices

1. `.rpreset` export/import in the plugin browser and the app.
2. Discovery details and user-content flag.
3. IR library + `ir` kind (IR plugin editor and a list/marks control
   method).
4. Track presets as a kind.
5. `preset-discovery` provider (optional).

## How we'd know it works

Round-trip tests for `.rpreset` in `resonance-plugin/tests/preset_library.rs`;
discovery fixtures with soundpacks in
`resonance-audio/tests/clap_host/preset_discovery.rs`; control tests for
the new kinds; for P10, a clack-host test that enumerates the factory.
