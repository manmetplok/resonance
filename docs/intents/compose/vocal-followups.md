# Vocals: what is left after the MCP-control and quality work

> Status: **partly built**
> Area: compose (vocals) — [vocal rendering](../../features/compose/vocal-rendering.md), [voicebank library](../../features/compose/voicebank-library.md), [lyrics and melody](../../features/compose/vocal-lyrics-and-melody.md)
> Source: the leftovers of [`vocal-mcp-control-and-quality.md`](../../history/specs/vocal-mcp-control-and-quality.md) (all 14 slices built, 2026-10-07; its §11 decision table stays there), the builders' known gaps and the 2026-10-07 code review.

## Why

Every slice of the vocal spec is built: an agent sets every lane setting,
installs banks from the plok.org index, writes melismas and breaths, edits
expression curves and knows whether a lane's audio is current. What is not
done needs a person, a server or a live session rather than code in this
repo — which banks users should get, the index they come from, whether the
default bank's licence holds — plus a few small gaps the builders left.

## What it would do

1. **Bank choice (Jorrit).** A blind listen of the audition renders decides
   which banks join the plok.org index: Peiton, LUNAI Katyusha, Keiro
   Revenant and Liam Thorne (and maybe Allen Crow), and Meiji v170, against
   the current TIGER, Lilia and Meiji v160. Their index entries (with
   terms, credit and commercial) are then added to
   [`plok-voicebank-index.json`](../../features/compose/plok-voicebank-index.json).
2. **A published index.** `https://resonance.plok.org/index.json` serves
   `voicebanks[]` and `vocoders[]` (server work, outside the repo):
   - compute the missing sha256 of TIGER's `TIGER_DS_v106_PACK.zip` and of
     NSF-HiFiGAN 2024.02's `.oudep` (GitHub publishes no digest for
     either) and pin them;
   - check that TIGER's pack holds the bank folder the installer expects
     (one wrapper stripped, files outside the `dsconfig.yaml` branch
     dropped) and that both `.oudep` files hold `vocoder.yaml` at the top
     or one folder down;
   - run one real install of each entry (hundreds of MB from GitHub) — so
     far only a loopback server and fixture banks have been installed.
3. **The live MCP session** (the vocal spec's §13 "Live"): an agent
   installs a bank, sets params, writes a melisma and breaths, edits a
   curve, renders, and nothing goes through the GUI.
4. **Licences before any commercial release.**
   - Lilia's ToU forbids "AI-generated music and other generative AI
     contexts" and asks porters to contact the creator. D9 accepted this
     while Resonance is not commercial. Contact julieraptor
     (liee.immortal.idol@gmail.com) for written permission, or move the
     default bank off Lilia.
   - The openvpi vocoders (PC-NSF-HiFiGAN 2025.02, NSF-HiFiGAN 2024.02)
     are CC BY-NC-SA 4.0, and Lilia ships a fine-tune of one: commercial
     use needs permission or another vocoder.
5. **Small gaps the builders left.**
   - The bulk lyric editor drops ordinary `·` breaks when it writes text
     back (only those next to an in-word hold survive).
   - Breaths are rendered but not drawn in the vocal roll.
   - The `speedup` measurement has no Meiji row: the
     `render_resonance_vocal` example's G2P copy cannot phonemise Meiji
     (`en/` prefix, language ids).
   - The GPU backends are untested on hardware: the app hard-codes CPU;
     `cuda` and `migraphx` build, but MIGraphX needs an ONNX Runtime built
     with it (`ORT_LIB_LOCATION`), and neither has run on a GPU.
   - The energy path (velocity and the Dynamics curve → energy) runs on no
     shipped bank and has not been heard.
   - Comfortable ranges are per bank; `speaker_ranges` can hold a
     speaker's own, but none is known (Meiji's modes might differ).
6. **Intelligibility, what is left** (after the 2026-10-08 fixes, see
   [vocal rendering § Measurements](../../features/compose/vocal-rendering.md#measurements)).
   - The 2026-10-07 audition renders predate the fixes (TIGER sang the
     wrong words, Lilia another language's consonants) — listen to a fresh
     `scripts/vocal-audition.py` run before judging banks.
   - Done 2026-10-08: the banks' own duration, pitch and variance models
     run inside the render (`bank_timing` / `bank_pitch`, default on) and
     every model's noise is seeded, so renders repeat bit for bit
     ([vocal rendering § Bank models](../../features/compose/vocal-rendering.md#bank-models)).
     The held-out phrase went from 85 / 76 / 76 % (TIGER / Meiji / Lilia)
     to the figures in § Measurements there.
   - Left: a process-wide session cache for the bank models (they load
     per render); the voice card shows the bank-model toggles on banks
     without the models; `song.vocal` reports the rule timing, not the
     model's; short passing notes sit off pitch under the bank's pitch
     model (it glides through them).
   - Check every candidate bank's `dsdict-en.yaml` for `replacements:` —
     Lilia's were ignored until now, and a bank that respells English
     sings garbage without them.

## What exists to build on

- The audition: `scripts/vocal-audition.py` renders every installed bank ×
  speaker blind and `--reveal`s the key, each voice transposed into its
  range; `scripts/vocal-intelligibility.py` scores a set with Whisper
  ([vocal rendering § bank audition](../../features/compose/vocal-rendering.md#bank-audition)).
- The proposed index entries and their research notes:
  [`plok-voicebank-index.json`](../../features/compose/plok-voicebank-index.json),
  [`singing-voicebank-options-2026`](../../history/research/singing-voicebank-options-2026.md).
- Install, registry and overlay: [voicebank library](../../features/compose/voicebank-library.md).
- `vocal_dynamics_render.rs` and `voicebank_curve_support.rs` (real-model
  tests, run with `RESONANCE_SVS_MODELS_DIR`) for checking a new bank.

## Open decisions

1. Which audition candidates join the index — Jorrit, by ear.
   Recommended: add every candidate that renders the audition phrase
   cleanly and passes the listen; keep TIGER and Meiji v160 installable.
2. The default bank if julieraptor does not approve agent use.
   Recommended: the best-rated LUNAI English singer from the listen
   (`permission` terms, but no generative-AI ban), or Peiton if its terms
   are better.
3. Whether to expose a GPU backend setting. Recommended: not until one
   has been measured on hardware.

## Slices

1. Listen and pick; add the chosen entries (with terms) to the committed
   index proposal; extend `the_committed_index_proposal_parses`.
2. Hash, inspect and publish the index; one real install per entry.
3. The live MCP session, with its findings filed as bugs.
4. The licence follow-up (mail; a decision recorded in the feature docs).
5. The small gaps, each a vertical slice of its own.
6. The intelligibility leftovers, measured with
   `scripts/vocal-intelligibility.py` before and after.

## How we'd know it works

- `vocal_voicebanks` on a fresh machine lists the chosen banks as
  `available`, each with a pinned sha256, and `vocal_voicebank_install`
  installs each one; the audition renders every installed bank cleanly.
- A live agent session completes the §13 flow without the GUI.
- The licence decision is written into the voicebank library doc.
