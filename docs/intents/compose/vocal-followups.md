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
   which banks join the plok.org index.
   - The first blind set (12 voices: TIGER ×7, Lilia, Meiji v160 ×4,
     after the bank-model work) got the verdict "most voicebanks sound
     good now" (2026-10-13).
   - The LUNAI English singers — Katyusha, Keiro Revenant, Liam Thorne and
     Allen Crow (v170) — are installed in the user voicebank folder,
     registered (their own models, ranges and terms in the built-in table)
     and have entries with pinned sha256 in
     [`plok-voicebank-index.json`](../../features/compose/plok-voicebank-index.json).
     The Whisper scores of every voice are in
     [vocal rendering § Measurements](../../features/compose/vocal-rendering.md#measurements).
   - **Blind scores of all 41 voices** (Jorrit, 2026-10-13, 1–5 with
     `scripts/vocal-audition-score.py`, same phrase for all):
     - Bank means: TIGER 4.00 (7), Katyusha 3.00 (4), Liam Thorne 3.00
       (10), Lilia 3.00 (1), Meiji 2.75 (4), Allen Crow 2.60 (5), Keiro
       Revenant 2.50 (10).
     - 5: tiger/mystic. 4: allen_crow/heavy, katyusha/standard,
       keiro_revenant/cold and standard, liam_thorne/deep, rock, standard
       and tired, tiger/disco, electric, fresh, royal and vinyl.
     - 3: allen_crow/open and standard, katyusha/delicate and strong,
       keiro_revenant/aggressive, mellow, native and solid,
       liam_thorne/aggressive, crispy, gentle and powerful, lilia,
       meiji/hunter, lilith and phantom, tiger/glam.
     - 2: allen_crow/gentle, katyusha/lullaby, keiro_revenant/fragile,
       meiji/standard.
     - 1 (effect modes, not lead voices on a plain line): allen_crow/whisper,
       keiro_revenant/growl, vf and whisper, liam_thorne/growl and vf.
   - **Decided and built (2026-10-13):** the default bank moved from
     Lilia (3) to TIGER (4.00 mean): a new lane sings TIGER `mystic` (5)
     whatever its voice type, while stored lanes keep their bank and a
     file without `voicebank` still reads as Lilia. Effect modes (growl,
     vocal fry, whisper) are marked `effect` in the built-in table,
     listed last in the speaker picker as `Effects ‹ …`, flagged in
     `vocal.voicebanks`, and never a bank's default; Meiji's `phantom`
     stays a lead mode. See
     [voicebank library](../../features/compose/voicebank-library.md) and
     [lyrics and melody](../../features/compose/vocal-lyrics-and-melody.md).
   - Every bank has at least one voice scored 4 or more except Lilia and
     Meiji, so all seven can stay in the index.
   - Peiton is not installed: it needs a manual download from Gumroad
     (https://nebm.gumroad.com/l/vvhyg), so it has no index entry and was
     in neither blind set.
   - Meiji v170 is listed in the index but was not auditioned (the dev
     models root holds v160).
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
   curve, renders, and nothing goes through the GUI. Run it with the agent
   plugin's `singing` skill loaded (`resonance-agent-plugin/skills/singing/`,
   plugin 0.4.0) — the session checks the skill's procedure as much as the
   tools, and its findings go to the skill as well as to bugs. Still open.
4. **Licences before any commercial release.**
   - Lilia's ToU forbids "AI-generated music and other generative AI
     contexts" and asks porters to contact the creator. D9 accepted this
     while Resonance is not commercial. She is no longer the default bank
     (TIGER is, since 2026-10-13), but she stays a built-in choice:
     contact julieraptor (liee.immortal.idol@gmail.com) for written
     permission, or drop her before a commercial release. TIGER's terms
     are `paid`: check what a commercial release owes for its default
     voice.
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
   - Per-speaker ranges: only Allen Crow's `open` and `gentle` have
     their own (G3–D5). Vocal fry and growl modes keep their bank's range
     though they hit their low (growl) or any (fry) notes loosely; Meiji's
     range and gender are uncertain (the research notes say male, LUNAI's
     site a girl's voice) — the listen should say whether B2–F♯4 suits
     her.
6. **Intelligibility, what is left** (after the 2026-10-08 fixes, see
   [vocal rendering § Measurements](../../features/compose/vocal-rendering.md#measurements)).
   - The 2026-10-07 audition renders predate the fixes (TIGER sang the
     wrong words, Lilia another language's consonants) — listen to a fresh
     `scripts/vocal-audition.py` run before judging banks.
   - Left after the bank models
     ([vocal rendering § Bank models](../../features/compose/vocal-rendering.md#bank-models)):
     `song.vocal` reports the rule timing, not the model's; short passing
     notes sit off pitch under the bank's pitch model (it glides through
     them); a random op in a model's control-flow body would stay
     unseeded (none of the shipped banks has one).
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

1. Which audition candidates stay in the index — Jorrit, by ear, from the
   41-voice blind set. Recommended: keep every bank whose voices pass the
   listen; keep TIGER and Meiji installable.
2. The default bank if julieraptor does not approve agent use.
   Recommended: the best-rated LUNAI English singer from the listen
   (`permission` terms, but no generative-AI ban), or Peiton if its terms
   are better.
3. Whether to expose a GPU backend setting. Recommended: not until one
   has been measured on hardware.

## Slices

1. Listen to the 41-voice set and pick; drop the entries not chosen from
   the committed index proposal (the LUNAI four are in it and
   `the_committed_index_proposal_parses` checks them); Peiton by hand.
2. Hash, inspect and publish the index; one real install per entry.
3. The live MCP session, driven by the `singing` skill, with its findings
   filed as bugs (and skill fixes).
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
