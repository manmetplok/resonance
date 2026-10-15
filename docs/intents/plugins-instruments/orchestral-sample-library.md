# Orchestral instruments from open sample libraries (an SFZ sampler)

> Status: **idea**
> Area: plugins-instruments
> Source: conversation 2026-10-15 — research into openly licensed orchestral
> sample libraries.

## Why

Resonance has no way to play a real-sounding acoustic instrument. The only
sampler is `resonance-drums`, which reads only its own `drum_samples.json`
manifest. Strings, winds, brass, harp, orchestral percussion, organ and
choir can only be faked on the wavetable synth. Agents feel it too: the
genre skills write violin and string GUIDE lines and "strings beds", but
those lines play on a synth patch that a listener cannot take for the
instrument the user will record over it. Chamber, cinematic and
singer-songwriter arrangements have nothing to sit on.

The free orchestral libraries all ship as **SFZ**, a plain-text format that
maps WAV regions to keys, velocities, keyswitches and CCs. A first-party
SFZ player is enough to open them all, and two of them are CC0 and could
ship with the app.

## What it would do

- **A sampler instrument plugin** (`com.resonance.sampler`, working name)
  that loads an `.sfz` instrument and plays it with streaming, velocity
  layers, round robins, keyswitched articulations, CC-driven dynamics
  crossfades, loop points and envelopes. The subset of SFZ it supports is
  the subset the libraries below actually use, not the whole spec.
- **An instrument library**, built on the same `library_marks` /
  BrowserModel foundation as drum kits, NAM models and voicebanks. It lists
  installed SFZ instruments by family (strings, woodwinds, brass,
  percussion, keys, choir), offers marks and search, and installs the
  default libraries.
- **Default content:** VSCO 2 CE and VCSL, both CC0. SSO 4.0 is offered as
  a one-click download, never bundled (see Open decisions).
- **Control surface:** agents can list instruments, load one onto a track
  and pick an articulation (a keyswitch or a CC) from note data, so an
  arranging skill can write a string section that sounds like one.

Done when an agent can build a four-section string, wind and brass
arrangement with keyswitched shorts and longs and CC1 dynamics, entirely
over MCP, and the render plays every articulation it asked for.

## Survey of open libraries (2026-10-15)

| Library | Licence | Size | What it covers | Usable as |
|---|---|---|---|---|
| [VSCO 2 Community Edition](https://versilian-studios.com/vsco-community/) (Versilian Studios) | **CC0** | ~3 GB, ~3,000 samples, ~70 packs | Chamber orchestra subset: strings, winds, brass, much percussion. Official SFZ and WAV, 44.1k 16/24-bit | **Bundle.** Uneven depth (subset of the paid Pro edition) |
| [VCSL](https://github.com/sgossner/VCSL) (Versilian Studios) | **CC0** | — | Concert and folk harp, pipe and renaissance organ, recorders, harpsichords, pianos, tenor sax, world instruments, percussion. SFZ | **Bundle.** Fills VSCO CE's harp, organ and keyboard gaps |
| [Sonatina Symphonic Orchestra 4.0](https://github.com/peastman/sso) (Westlund, maintained by Peter Eastman since 2017) | CC Sampling Plus 1.0 | ~1.4 GB | Full orchestra: section and solo strings, woodwinds, brass, timpani, pitched percussion, harp, piano, harpsichord, 14-stop organ, choir. Sustain, marcato, staccato, legato, pizzicato, col legno, tremolo, harmonics. "Notation" and "performance" variants. 16-bit 44.1k, minor-third sampling, ≥2× RR on shorts, little baked reverb | **Download on demand** |
| [Virtual Playing Orchestra 3.3](https://virtualplaying.com/virtual-playing-orchestra/) (Paul Battersby) | Mixed per source: CC0, CC BY-SA 3.0/4.0, Sampling Plus | ~600 MB WAV + scripts | Full orchestra with keyswitch patches. The "Performance" edition uses mod wheel for dynamics and velocity to choose the articulation. Compiled from SSO, Iowa, VSCO 2 and others | Download on demand, licence checked per instrument. Mixed rooms, so it needs a shared reverb |
| [University of Iowa MIS](https://theremin.music.uiowa.edu/mis.html) | "Use without restriction" (no formal licence) | — | Anechoic chromatic scales at pp/mf/ff, one KM 84 | Raw material for our own SFZ mappings; no patches |
| [FreePats](https://freepats.zenvoid.org/licenses.html) | CC0 for new work (older items GPL with exception) | small | Harp, timpani and others, several built from VCSL. SFZ/FLAC | Bundle the CC0 items |
| [MuseScore General](https://musescore.org/en/node/317991) | MIT | SF2/SF3 | General MIDI set, strings taken from VSCO 2 CE | Not an orchestral library; skip |
| [Philharmonia samples](https://philharmonia.co.uk/resources/sound-samples/) | Custom: no redistribution "as is" or as a sampler instrument | — | Every orchestral instrument, short notes | User's own download only; we cannot ship mappings with the samples |

**Out of scope:** Spitfire Symphony Orchestra Discover (Nov 2025), BBC
Discover, LABS, Orchestral Tools' Berlin Free Orchestra and the
MuseSounds. They are free but locked to Kontakt, SINE or Muse Hub and
licensed against redistribution or third-party playback.

**Licence notes.** CC Sampling Plus 1.0 has been
[retired](https://creativecommons.org/licenses/sampling+/1.0/). Music made
with it can be sold. The library itself may only be redistributed whole
and non-commercially, and it may not be used to promote anything except
the work made from it. Shipping it inside an MIT OR Apache-2.0 app is
therefore the wrong default. Having the user's machine download it from
upstream sidesteps the question. A credits line listing the libraries in
use belongs in `THIRD_PARTY_LICENSES.md` for anything we bundle.

## What exists to build on

- **The drums streaming engine:** `plugins/resonance-drums/src/stream/`
  (disk streaming with preload), `kit_loader/` (background loader thread,
  scoped decode workers, `cache.rs` process-wide decoded-take cache,
  progress reporting, one-slot mailbox to the audio thread), `voice.rs`,
  `velocity.rs`, round-robin selection (`rr_display.rs`) and
  `dsp/janitor.rs`. Pitched multi-sampling, loops and release triggers are
  new, but the loading and streaming machinery is the hard part, and it
  already exists and is RT-tested
  (`docs/features/plugins-instruments/drums-plugin.md`).
- **Library and install plumbing:** `resonance-common/src/drumkit_library/`
  (staging copy with progress, cancel and disk check),
  `resonance-common/src/plok_download/` (HTTP fetch, parts, timeouts), the
  voicebank installer driven over the control API
  (`resonance-app/src/update/voicebank_library.rs`,
  `resonance-app/src/control_jobs.rs`) and `library_marks`
  (`docs/features/plugins-instruments/drum-kit-library.md`,
  `docs/features/compose/voicebank-library.md`).
- **Third-party CLAP hosting**
  (`docs/features/plugins-instruments/third-party-plugin-hosting.md`): a
  user can already load any CLAP SFZ player. This intent is about a
  first-party one the agent can drive and the app can ship content for.
- **Plugin SDK and editor runtimes**
  (`docs/features/plugins-instruments/plugin-sdk.md`,
  `plugin-editor-runtimes.md`). Follow the new-plugin checklist (crate and
  workspace member, `first_party` id, `TEST_BINARY_CAPS`, `fleet_*`
  lists).

## Open decisions

1. **Engine: native Rust or sfizz.**
   [sfizz](https://github.com/sfztools/sfizz) is a mature C++ SFZ engine
   under BSD-2, which is licence-compatible. Using it means C++ in the
   build, FFI and a second streaming implementation beside the drums one,
   and I have not confirmed it ships a CLAP build. The pure-Rust crates
   ([`sofiza`](https://lib.rs/crates/sofiza),
   [`soundfont`](https://lib.rs/crates/soundfont),
   [`vibelang_sfz`](https://docs.rs/vibelang-sfz)) are parsers or hobby
   samplers without the needed opcode coverage. **Recommended: native.**
   Factor the drums streaming and loader core into a shared crate
   (`resonance-sampler-core`, needing a row in the ARCHITECTURE layering
   table) and write our own SFZ-subset parser. Treat sfizz as the reference
   implementation when checking behaviour.
2. **SFZ subset.** **Recommended:** the opcodes the three target libraries
   use, found by grepping their `.sfz` files before writing the parser
   (slice 0). Expected: `<region>/<group>/<control>/<global>`, `#define`,
   `#include`, `default_path`, `sample`, `key`/`lokey`/`hikey`/
   `pitch_keycenter`, `lovel`/`hivel`, `sw_lokey`/`sw_hikey`/`sw_last`/
   `sw_default`, `seq_length`/`seq_position`, `lorand`/`hirand`,
   `trigger=release`, `loop_mode`/`loop_start`/`loop_end`, `xfin_*`/
   `xfout_*` and `xf_cccurve`, `amp_veltrack`, `ampeg_*`, `volume`, `pan`,
   `tune`/`transpose`, `group`/`off_by`. Anything unknown is reported
   once per instrument, not silently dropped.
3. **SSO distribution.** **Recommended:** an install job that downloads
   the upstream GitHub release, the way voicebanks install today. Never
   bundle it, and record its licence in the library entry so the UI can
   show it.
4. **Bundling VSCO CE and VCSL.** At ~3 GB they are too large for the
   repo or the app package. **Recommended:** the same on-demand install as
   SSO, offered at first use of the sampler, with the CC0 status shown.
5. **Articulation surface.** Keyswitch notes in the clip work with every
   SFZ instrument but clutter the piano roll. **Recommended:** expose the
   instrument's keyswitches as a named articulation list (read from
   `sw_label`, or derived when absent). The control API sets the
   articulation per note, and the plugin turns it into the keyswitch. The
   piano roll shows the articulation as a lane rather than as notes.
6. **Mixed-room libraries.** VPO and SSO blend sources recorded in
   different rooms. **Recommended:** leave this to the mixing and spatial
   skills, which already put strings "back" and wetter. Add no per-library
   reverb inside the sampler.

## Slices

0. **SFZ survey.** Parse every `.sfz` in VSCO CE, VCSL and SSO 4.0 with a
   throwaway tool and list opcode frequencies. This fixes decision 2 and
   becomes the parser's acceptance corpus.
1. **Shared sampler core.** Extract streaming, decode cache, loader thread
   and voice pool from `resonance-drums` into a shared crate. The drums
   goldens and RT tests must hold unchanged.
2. **SFZ parser and region map.** Cover headers, includes, defines and the
   opcode subset, with key, velocity, round-robin and random selection.
   Test against the slice-0 corpus: every file parses, and each library's
   regions resolve to existing WAVs.
3. **Sampler plugin.** Pitched playback, envelopes, loops, release
   triggers, keyswitches and CC crossfades. Params, state and a minimal
   editor showing the instrument name, articulation and a keyboard with
   keyswitch zones. Goldens per articulation on a VSCO CE instrument, each
   with a guard against silent output.
4. **Library and install.** Browser entries by family, marks, install
   jobs for VSCO CE, VCSL and SSO with progress and cancel, licence shown.
5. **Control API and MCP tools.** List instruments, load one onto a track,
   list and set articulations. One tool per method, landed with its
   handler. Add an `arranging`/`orchestration` note to the agent plugin,
   kept in lockstep by the existing lockstep test.

## How we'd know it works

- Every `.sfz` in the three libraries parses, and every region's sample
  resolves (slice 0 corpus test).
- Per-articulation golden renders of short and long notes at pp/mf/ff.
  Each must be non-silent, and round robins must differ across repeated
  notes.
- `RESONANCE_RENDER_THREADS=8` renders are bit-identical to serial ones.
- Drums goldens and RT tests unchanged after slice 1.
- Live: a string quartet passage with keyswitched staccato → sustain and
  a CC1 swell plays without dropouts at 48k / quantum 128 with SSO
  streaming from disk.
- Agent: the "done when" arrangement above, built and verified over MCP
  alone.
