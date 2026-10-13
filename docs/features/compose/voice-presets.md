# Voice presets

> Verified against master @ `ad042afb` + voice presets on 2026-10-13.
> Source: the user's decisions of 2026-10-13 (contents; built-ins = every voice scored 3+) and the 41-voice blind listen in [vocal follow-ups](../../intents/compose/vocal-followups.md) item 1.

A voice preset is a singer and a delivery, put on a vocal lane in one
step: the voicebank, speaker, voice type and range, plus the delivery
sliders. It never touches the lane's lyrics, its melody or lyric
generator settings, or its expression curves, so a singer can be swapped
on a finished part. The built-ins are every voice the 41-voice blind
listen of 2026-10-13 scored 3 or more (31 of them), best first, each
carrying its score; the user saves their own from the lane inspector or
over the control API. How a lane sings is in
[vocal rendering](vocal-rendering.md); which banks exist is in the
[voicebank library](voicebank-library.md).

## Behaviour

- **What a preset sets** (`VoiceSettings`): `voicebank`, `speaker`,
  `voice`, `range`, `timbre`, `tension`, `tension_velocity_amount`,
  `tension_contour_amount`, `vibrato`, `vibrato_rate`, `portamento_ms`,
  `articulation`, `consonant_emphasis`, `velocity_dynamics`,
  `rest_threshold_beats`, `bank_timing`, `bank_pitch`. Everything else on
  the lane — `draft`, `theme`, mood / POV / rhyme / line and syllable
  counts, style, contour, syllable mode, the melody sliders, the seed and
  the curves — stays.
- **Built-ins** (`compose/vocal_svs/voice_presets/builtin.rs`).
  `AUDITION_SCORES` is the whole listen (41 voices, 1–5); `ROWS` names
  the 31 voices scored 3 or more, each with a name, a note and a
  `Character`. Derived, so a preset cannot drift from the bank table:
  - *range*: the speaker's comfortable range from the built-in bank table
    (`registry/builtin.rs`), exactly (Allen Crow's `open` has its own
    G3–D5);
  - *voice type*: the `VoiceType` whose default range is centred nearest
    that range's centre;
  - *register tag*: `low` (centre below G3½), `mid` (up to C♯4), `high`;
  - *delivery*: a new lane's defaults — what every audition was rendered
    with — nudged by the mode's character where its name says something
    obvious (below). Timbre stays `Warm` (it moves the model's formant
    input, and the voices were judged without it), vibrato rate and
    portamento stay at their defaults (with `bank_pitch` on, the bank's
    own pitch model sings its own), `bank_timing` / `bank_pitch` stay on.
  - *order*: score, then the bank's mean over the whole listen (TIGER
    4.00, Katyusha / Liam Thorne / Lilia 3.00, Meiji 2.75, Allen Crow
    2.60, Keiro Revenant 2.50), then name.
  - No effect mode (growl, vocal fry, whisper) is a preset: every one
    scored 1. Meiji's `phantom` is a lead mode and is one.
  - Ids are `<bank>-<speaker>` (`tiger-mystic`), or the bank id alone
    for a single-voice bank (`lilia`).
- **Character table** (the defaults it starts from: tension 0.5, tension
  from velocity 0.5 and from contour 0.5, vibrato 0.30, articulation 0.65
  — higher is shorter notes —, consonant emphasis 0.50, velocity →
  loudness 0.5). No slider moves more than 0.4, consonant emphasis stays
  in its 0.4–0.6 intelligible band, and tension is not heard on TIGER
  (its model takes none):

  | Character | Modes | Nudges |
  |---|---|---|
  | Neutral | standard, native, open, lilith, Lilia, every TIGER mode | none |
  | Gentle | gentle, delicate, mellow | tension 0.2, tension from velocity / contour 0.4, articulation 0.55, velocity → loudness 0.4 |
  | Powerful | powerful, strong, heavy, solid, hunter | tension 0.7, tension from velocity / contour 0.6, articulation 0.7, velocity → loudness 0.6 |
  | Aggressive | aggressive, rock | tension 0.8, tension from velocity 0.7 / contour 0.6, vibrato 0.25, articulation 0.75, velocity → loudness 0.65 |
  | Weary | tired | tension 0.3, tension from velocity / contour 0.4, vibrato 0.2, articulation 0.6, velocity → loudness 0.4 |
  | Cool | cold | tension 0.45, vibrato 0.15 |
  | Dark | deep | tension 0.4, articulation 0.6 |
  | Whispery | Meiji's phantom ("whispery and husky") | tension 0.1, vibrato 0.1, articulation 0.6, velocity → loudness 0.4 |
  | Crisp | crispy | articulation 0.7, consonant emphasis 0.58 |

- **The 31 built-ins**, picker order:

  | # | Name | Bank / speaker | Score | Voice, range (MIDI) | Tags | Character |
  |---|---|---|---|---|---|---|
  | 1 | TIGER Mystic | `tiger` / `mystic` | 5 | Tenor 48–69 | mid, mystic, male | Neutral |
  | 2 | TIGER Disco | `tiger` / `disco` | 4 | Tenor 48–69 | mid, disco, male | Neutral |
  | 3 | TIGER Electric | `tiger` / `electric` | 4 | Tenor 48–69 | mid, electric, male | Neutral |
  | 4 | TIGER Fresh | `tiger` / `fresh` | 4 | Tenor 48–69 | mid, fresh, male | Neutral |
  | 5 | TIGER Royal | `tiger` / `royal` | 4 | Tenor 48–69 | mid, royal, male | Neutral |
  | 6 | TIGER Vinyl | `tiger` / `vinyl` | 4 | Tenor 48–69 | mid, vinyl, male | Neutral |
  | 7 | Katyusha — Alto lead | `katyusha` / `standard` | 4 | Alto 55–74 | high, standard, female | Neutral |
  | 8 | Liam Thorne — Baritone lead | `liam_thorne` / `standard` | 4 | Baritone 45–65 | low, standard, male | Neutral |
  | 9 | Liam Thorne — Deep baritone | `liam_thorne` / `deep` | 4 | Baritone 45–65 | low, dark, deep, male | Dark |
  | 10 | Liam Thorne — Rock baritone | `liam_thorne` / `rock` | 4 | Baritone 45–65 | low, aggressive, rock, male | Aggressive |
  | 11 | Liam Thorne — Tired baritone | `liam_thorne` / `tired` | 4 | Baritone 45–65 | low, weary, tired, male | Weary |
  | 12 | Allen Crow — Heavy tenor | `allen_crow` / `heavy` | 4 | Tenor 48–69 | mid, powerful, heavy, male | Powerful |
  | 13 | Keiro Revenant — Cold tenor | `keiro_revenant` / `cold` | 4 | Tenor 48–69 | mid, cool, cold, male | Cool |
  | 14 | Keiro Revenant — Tenor lead | `keiro_revenant` / `standard` | 4 | Tenor 48–69 | mid, standard, male | Neutral |
  | 15 | TIGER Glam | `tiger` / `glam` | 3 | Tenor 48–69 | mid, glam, male | Neutral |
  | 16 | Katyusha — Delicate alto | `katyusha` / `delicate` | 3 | Alto 55–74 | high, gentle, delicate, female | Gentle |
  | 17 | Katyusha — Strong alto | `katyusha` / `strong` | 3 | Alto 55–74 | high, powerful, strong, female | Powerful |
  | 18 | Liam Thorne — Aggressive baritone | `liam_thorne` / `aggressive` | 3 | Baritone 45–65 | low, aggressive, male | Aggressive |
  | 19 | Liam Thorne — Crisp baritone | `liam_thorne` / `crispy` | 3 | Baritone 45–65 | low, crisp, crispy, male | Crisp |
  | 20 | Liam Thorne — Gentle baritone | `liam_thorne` / `gentle` | 3 | Baritone 45–65 | low, gentle, male | Gentle |
  | 21 | Liam Thorne — Powerful baritone | `liam_thorne` / `powerful` | 3 | Baritone 45–65 | low, powerful, male | Powerful |
  | 22 | LIEE Lilia | `lilia` / — | 3 | Tenor 50–70 | mid, female | Neutral |
  | 23 | Gahata Meiji — Hunter | `meiji` / `hunter` | 3 | Baritone 47–66 | low, powerful, hunter | Powerful |
  | 24 | Gahata Meiji — Lilith | `meiji` / `lilith` | 3 | Baritone 47–66 | low, lilith, mature | Neutral |
  | 25 | Gahata Meiji — Phantom | `meiji` / `phantom` | 3 | Baritone 47–66 | low, whispery, phantom, husky | Whispery |
  | 26 | Allen Crow — Open high voice | `allen_crow` / `open` | 3 | Alto 55–74 | high, open, male | Neutral |
  | 27 | Allen Crow — Tenor lead | `allen_crow` / `standard` | 3 | Tenor 48–69 | mid, standard, male | Neutral |
  | 28 | Keiro Revenant — Aggressive tenor | `keiro_revenant` / `aggressive` | 3 | Tenor 48–69 | mid, aggressive, male | Aggressive |
  | 29 | Keiro Revenant — Mellow tenor | `keiro_revenant` / `mellow` | 3 | Tenor 48–69 | mid, gentle, mellow, male | Gentle |
  | 30 | Keiro Revenant — Native tenor | `keiro_revenant` / `native` | 3 | Tenor 48–69 | mid, native, male | Neutral |
  | 31 | Keiro Revenant — Solid tenor | `keiro_revenant` / `solid` | 3 | Tenor 48–69 | mid, powerful, solid, male | Powerful |

- **User presets** (`voice_presets/store.rs`): one JSON file per preset,
  `{name, description, tags, settings}` (`settings` with `VocalParams`'
  own field names; a missing field reads as a new lane's default), in
  `$XDG_DATA_HOME/resonance/voice-presets/`, `$RESONANCE_VOICE_PRESET_DIR`
  when set; a hermetic test app gets a private folder of its own
  (`user_dirs::hermetic_subdir("voice-presets")`), so tests never see the
  machine's presets or each other's. File names use the track presets'
  injective encoding. A user preset's id is `user:<name>`; it has no
  score. A file that does not parse or fails `VocalParams::validate` is
  quarantined as `.corrupt`. A built-in's name is refused (either
  surface); an existing user name is replaced only after a confirm
  (GUI: the button reads **Overwrite**; control: `overwrite: true`).
  Save, rename and delete are machine state: no undo entry, no revision.
- **Apply** is one lane edit: the lane inspector's
  `LaneInspectorMsg::ApplyVoicePreset`, one undo entry, from either
  surface. The lane's render state follows from its content: a lane with
  audio reads `stale`; a GUI pick arms the debounced re-render like any
  voice setting, a control apply never renders. A preset whose bank is not
  installed still applies: the lane then shows `<id> (not installed)` in
  the Voicebank picker and the card says so, and the control result has
  `installed: false` and a `warning` naming the install routes.
- **Matching.** The card shows the first preset (picker order) whose
  settings the lane sings exactly (`VoiceSettings::matches`, delivery
  within 1e-4), else **Custom**; moving one slider makes the lane Custom.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Model and built-ins | `resonance-app/src/compose/vocal_svs/voice_presets/{mod,settings,builtin,store}.rs` | `VoicePreset`, `VoiceSettings` (`from_params`, `apply_to`, `matches`, `validate`), `builtin_presets`, `AUDITION_SCORES`, `ROWS`, `Character`, `nearest_voice_type`, `register_tag`, `find`, `matching`, `capture`, `VoicePresetStore`, `machine_dir`, `VOICE_PRESET_DIR_ENV` |
| App state | `state/voice_presets.rs`, `lib.rs` | `VoicePresetState` (`store`, `user`, `prompt`, `confirm_delete`, `error`), `VoicePresetPrompt`, `PromptMode`, `Resonance::voice_presets` |
| GUI | `update/voice_presets.rs`, `view/compose/lane_inspector/vocal/voice_group/presets.rs`, `state/picks.rs`, `state/ui_caches.rs` | `VoicePresetMessage` (`Pick`, `OpenSave`, `OpenRename`, `NameChanged`, `Confirm`, `Cancel`, `Delete`), `preset_block`, `VoicePresetChoice`, `voice_preset_choices`, `UiViewCaches::voice_preset_choices`, `VocalRailContext::{voice_presets, preset_ui}`, `Resonance::refresh_vocal_choices` |
| Lane edit | `compose/messages.rs`, `update/compose/lane_inspector/mod.rs` | `LaneInspectorMsg::ApplyVoicePreset` (undo `Record`) |
| Control | `update/control/vocal/presets.rs`, `view_model/voice_presets.rs`, `gate.rs` | `list`, `apply`, `save`, `voice_preset_view` |
| Wire | `resonance-control/src/methods/vocal.rs` | `VOICE_PRESETS`, `APPLY_VOICE_PRESET`, `SAVE_VOICE_PRESET`, `VoicePresetList`, `VoicePresetView`, `VoicePresetSettings`, `ApplyVoicePresetParams` / `Result`, `SaveVoicePresetParams` / `Result` |
| MCP | `resonance-mcp/src/tools/vocal.rs` | `vocal_voice_presets`, `vocal_apply_voice_preset`, `vocal_save_voice_preset` |

## Surfaces

- **GUI:** the vocal lane inspector's Voice & delivery card opens with a
  **Voice preset** block: **Save…** on the label's row and a picker over
  the cached options (`UiViewCaches::voice_preset_choices`, rebuilt with
  the voicebank choices and after every save, rename or delete; the rail
  is `lazy`, so the match is computed only when it rebuilds). Built-ins
  read `5/5 · TIGER Mystic`, user presets `User ‹ <name>` (the UI font has
  no ★; the card draws the matched preset's score as five icon-font stars
  beside its note). A user preset adds **Rename…** and **Delete** (a
  second press confirms: "Delete? Press again"). Save… and Rename… open an
  inline name field with **Save** / **Overwrite** / **Rename** and
  **Cancel**; Overwrite adds "Replaces the voice preset “…”". A lane on a
  bank that is not installed shows "<id> is not installed — Library…
  downloads it; the lane renders once it is".
- **Control API / MCP:**
  - `vocal.voice_presets` → `mcp__resonance__vocal_voice_presets` (read,
    above the mutation gate, no project needed): `{presets: [{id, name,
    description, tags, score?, builtin, voicebank, speaker?, installed,
    settings {voicebank, speaker?, voice, range {low, high}, timbre,
    tension, tension_velocity_amount, tension_contour_amount, vibrato,
    vibrato_rate, portamento_ms, articulation, consonant_emphasis,
    velocity_dynamics, rest_threshold_beats, bank_timing, bank_pitch}}]}`.
  - `vocal.apply_voice_preset {track_id, section_id?, preset}` →
    `vocal_apply_voice_preset` (edit: one undo entry, one revision):
    `preset` is an id or a name (case-insensitive; an unknown one is
    `not_found` listing the ids); returns `{track_id, section_id,
    preset_id, preset_name, params (the vocal.get_params view), installed,
    warning?, revision}`.
  - `vocal.save_voice_preset {track_id, section_id?, name, description?,
    tags?, overwrite?}` → `vocal_save_voice_preset` (library: gated
    because it reads the open project's lane, listed in
    `reads_open_project_only`, no undo entry, no revision): a built-in's
    name is `invalid_params`, an existing user name without `overwrite`
    is `needs_confirmation`; returns `{preset, replaced}`.
  - Optional new methods: no protocol bump.
- **Agent skills:** none name the tools yet.

## How it's tested

| What | Where | Run |
|---|---|---|
| The listen covers every offered voice once, bank means; built-ins are exactly the voices scored ≥ 3, best first (score, bank mean, name), unique ids and names; no effect mode; ranges are the comfortable ranges and voice types the nearest; deliveries are defaults with a modest character fit; apply sets exactly the voice + delivery fields (draft, curves, generator settings kept) and `matches` tells one slider; `find` by id or name; user presets save / overwrite / list / rename / delete, corrupt and invalid files quarantined, a bare file reads as defaults; the machine folder override | `resonance-app/tests/vocal/voice_presets.rs` | `./scripts/run-tests.py -p resonance-app` |
| `vocal.voice_presets` without a project (31, best first, `installed`); apply: exactly the preset's fields, one undo entry, one revision, the lane `stale` with no render queued, one undo restores it; a not-installed bank applies with `installed: false` and a warning, and `vocal.render` then answers `voicebank_missing`; unknown preset / track `not_found`; save: file in the app's own folder, no undo, no revision, listed last, `needs_confirmation` then `overwrite`, built-in name refused, applied by name to another lane; the GUI pick is one undo entry and arms the debounced re-render | `resonance-app/tests/control/control_voice_presets.rs`; `control_mutation_gate.rs` (`vocal.voice_presets` runs with no project) | same |
| Picker options and labels; the matched preset, Custom after a slider move; the not-installed note; Save (built-in name refused) / Overwrite / Rename / Delete (two presses) from the card, no undo entry; goldens of the matched, not-installed and user-preset-with-overwrite states | `resonance-app/tests/compose/compose_voice_presets.rs` → `compose_voice_preset_{matched,not_installed,user_overwrite}-wgpu.png` | `RESONANCE_BLESS=1` to re-bless |
| MCP round trip: typed params out in wire shape, structured result back | `resonance-mcp/tests/translation.rs` (`voice_preset_tools_round_trip`) | `./scripts/run-tests.py -p resonance-mcp` |

## Decisions and deviations

- **Contents** (the user's call): voice + delivery only, so a preset can
  swap the singer on a finished part. `bank_timing` / `bank_pitch` are
  included (default true).
- **Built-ins** (the user's call): every voice scored 3 or more, with its
  score; ties broken by the bank's mean, then name.
- **Voice type by range centre.** LIEE Lilia comes out *Tenor* and Allen
  Crow's `open` *Alto*: their comfortable ranges (D3–A♯4, G3–D5) sit
  there. The voice type is a tessitura for the melody generator, not a
  gender; the tags carry `male` / `female` where the bank is known.
- **No "(not installed)" in the picker rows.** The rail is about 240 px
  wide and a long name plus the suffix ran into the arrow; the card says
  so once the preset is on the lane, and the Voicebank picker already
  reads `<id> (not installed)`.
- **Save is a library write** (decided here): no undo entry and no
  revision, like the other per-user libraries; unlike them it needs the
  open project, because it reads a lane.
- Rename and delete are GUI-only; over the wire an agent saves (and
  overwrites) but does not rename or delete.

## Known gaps

- The character nudges are by name, not by ear: nobody has listened to a
  preset's delivery against the plain mode.
- No skill points agents at the presets yet (the agent-plugin skills are
  being rewritten; a `singing` skill is planned).
- User presets have no score, stars or favourites (`library_marks`).
