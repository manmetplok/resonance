# DiffSinger / OpenUtau English voicebank ecosystem (as of 2026-10-07)

Method note: release dates below come from the GitHub API (`gh api repos/<repo>/releases`), which is authoritative. Two web summaries (a WebFetch of the TIGER and openvpi release pages) shifted some years by one; where they disagree with the API, the API wins and the conflict is noted. Local facts about the copies Resonance already has come from the shipped config files under `resonance-svs/models/` (cited as "local file").

## 1. What are TIGER, LIEE Lilia and Meiji exactly, and are there newer versions?

### Takeaway
TIGER has not changed since v106 (2024-04-29). The repo is dormant, but nothing marks it deprecated. Lilia MM 2.8 "JubiLIEE 2025" (2025-08-07, re-packed as 1.1) is still the newest LIEE release. Meiji has a newer build than the one Resonance ships: LUNAI's bulk "v170" release (2025-12-25) supersedes the v160 (2025-08-07) that Resonance bundles. None of the three has a named successor.

### Cited Findings
**TIGER**
- By tigermeat (also "neutrogic"). Voice provider tigermeat, illustrator AquiBoni, design by staticoceans. Male, range A2–C#5 — [DiffSinger Wiki: TIGER](https://diffsinger.miraheze.org/wiki/TIGER); [TIGER character TOS](https://github.com/spicytigermeat/tiger_diffsinger)
- Concept dates from 2021. DiffSinger development started in spring 2023. Beta b03 came out 2023-11-23 and the first stable release, v100, in late December 2023 — [UTAU wiki via search / DiffSinger Wiki](https://diffsinger.miraheze.org/wiki/TIGER)
- GitHub release dates: v101 2024-01-13, v102 2024-01-22, v103 2024-02-10, v106 2024-04-29. The repo was last pushed 2024-04-29, so nothing has happened since — [GitHub API, spicytigermeat/tiger_diffsinger releases](https://github.com/spicytigermeat/tiger_diffsinger/releases). (The wiki gives v106 as 2024-04-23 and v103 as 2024-02-06. A WebFetch summary wrongly put these in 2025.)
- v106 notes: "Korean XLS … Trained with 'Reflow' instead of 'ddsp' … Better pitch models" — [TIGER v106 release](https://github.com/spicytigermeat/tiger_diffsinger/releases/tag/v106)
- v103 removed the Cetera mode, added Glam and renamed Falsetto to Royal. The final mode set is Fresh, Disco, Electric, Vinyl, Mystic, Glam and Royal. There are three pitch models (standard, soft, power) — [DiffSinger Wiki](https://diffsinger.miraheze.org/wiki/TIGER); local file `singer/extracted/character.yaml` maps Fresh/Vinyl/Royal to standard, Disco/Mystic to soft and Electric/Glam to power.
- Languages: English, Japanese and Mandarin are primary; Spanish and French secondary; Italian, PT-BR, Thai, Russian, Korean, Polish and German work through XLS — [DiffSinger Wiki](https://diffsinger.miraheze.org/wiki/TIGER)
- Best results in English come from the "DIFFS EN X" phonemizer, which uses `[ax]` and `[dx]` (an ARPAbet-plus set) — [TIGER repo / search snippet](https://github.com/spicytigermeat/tiger_diffsinger)
- The GitHub README is stale: it still says "latest v102" — [README](https://github.com/spicytigermeat/tiger_diffsinger)
- Local file: the TIGER v106 acoustic model is a **shared multi-speaker model** whose `speakers:` list holds about 36 embeddings. Besides the 7 tiger_* modes there are alex_*, canary_*, leif_*, miyo, pjs, kitane_sno, mimosa_fr and others. `character.yaml` notes it was trained alongside the Millefeuille (UFR), TISD, PJS and Kitane Sno corpora — local file `resonance-svs/models/singer/extracted/dsacoustic/dsconfig.yaml`.

**LIEE "Immortal Idol" / Lilia**
- By julieraptor, who also voices it. The character LIEE (also "LIEE REY") is Filipino, non-binary and an adult. The voice is alto, optimal range C3–G5, described as "calm, elegant and mature" — [UTAU Repository: LIEE](https://utaurepository.miraheze.org/wiki/LIEE); [LIEE ToS](https://docs.google.com/document/d/1t13WKc7VA2Q3G_MppznbAwhl3vLIL2MNPyjVN3kWDDg/edit)
- Releases:
  - v0.0.4(3) "Proof of Concept 1" — 2023-12-23
  - MM2.6 "November 2024 (Test Version)" — 2025-02-22
  - MM2.8 "JubiLIEE 2025" — 2025-08-07, the newest; repo last pushed 2025-08-12
  - [GitHub API releases](https://github.com/julieraptor/DIFFSINGER-LIEE-Immortal-Idol/releases)
- MM2.8 notes: "trained on more English high tension data". Supports Japanese, Polish, French, Vietnamese, Mandarin and Spanish, and includes a Filipino phonemizer. The "JubiLIEE 2025 1.1" zip (about 609 MB, 485 downloads) is the current one and fixes `dsdict-fil.yaml` — [MM2.8 release](https://github.com/julieraptor/DIFFSINGER-LIEE-Immortal-Idol/releases/tag/MM2.8)
- The current version has no dedicated voice modes. The README says "use the TENSION capabilities to achieve expressiveness". Only PoC1 had 7 modes (Standard, Bone, Breath, Heart, Mouth, Lung, Teeth). The README also says "the DIFFS EN phonemizer doesn't support [dx] or [ax]", so those must be entered by hand. `[hh]` doubles as an exhale and `[m]` alone as a hum — [README](https://github.com/julieraptor/DIFFSINGER-LIEE-Immortal-Idol)
- PoC1 had no `[oy]` phoneme (substitute `[o i]`) — [v0.0.4 release](https://github.com/julieraptor/DIFFSINGER-LIEE-Immortal-Idol/releases)
- The missing English `v` in MM2.8 (a known Resonance quirk, substituted with `f`) is **not mentioned anywhere upstream**. I found no fix and no newer release.

**Gahata Meiji (歌幡メイジ) DiffSinger**
- A 2015-era UTAU character, revived as a DiffSinger bank made with the **LUNAI Project** (art: rev_bmp; portrait by CuromiSaccu in the shipped files) — [UTAU fandom (via search)](https://utau.fandom.com/wiki/Meiji_Gahata); [VocaDB](https://vocadb.net/Ar/166455)
- GitHub release `Gahata_Meiji_v160` came out 2025-08-07 (UTC). The community announcement followed on 2025-08-17 — [GitHub API lunaiproject/lunai_singers](https://github.com/lunaiproject/lunai_singers/releases); [VocaVerse thread](https://vocaverse.network/threads/gahata-meiji-diffsinger-release.10205/)
- **Newer: release "170" (2025-12-25)** ships `Gahata_Meiji_v170.zip` (412 MB, 3,389 downloads, the most-downloaded LUNAI bank) alongside 19 other LUNAI singers at v170. The repo was last pushed 2026-01-16. The v170 release notes are empty, so the changes are undocumented — [GitHub API](https://github.com/lunaiproject/lunai_singers/releases/tag/170)
- There are four modes:
  - Standard: from the original "Fairy" UTAU bank, sweet
  - Phantom: whispery and husky; an unreleased UTAU whisper bank turned into a mode
  - Lilith: mature and stronger
  - Hunter: powerful
  - [VocaVerse](https://vocaverse.network/threads/gahata-meiji-diffsinger-release.10205/)
- "16 languages": JA, EN, RU, FR, KO, TH, ZH, ES, IT, PT, DE and PL with phonemizers, plus Arabic and Ukrainian without phonemizers, plus Tagalog and Vietnamese indirectly — [VocaVerse](https://vocaverse.network/threads/gahata-meiji-diffsinger-release.10205/). Local v160 `dsconfig.yaml` has `num_lang: 14` and `use_lang_id: true`, and its phonemes are language-prefixed (`en/…`).

### Inferences
- TIGER is frozen, not deprecated. It has had 2.5 years without an update, and later openvpi improvements (LYNXNet backbone, pc-nsf-hifigan 2025.02, the v2.5.1 language-embedding fix) never reached it.
- Moving Meiji from v160 to v170 is the only upgrade available among the three. Whether it changes sound or phonemes is unknown and would need a diff of the zip.
- The Lilia `v` gap is probably a dictionary or phoneme-inventory issue in MM2.8. Upstream has not addressed it.

### Gaps
- What changed in Meiji v170, since the release notes are empty.
- Whether a LIEE MM3.x or TIGER v2 is in development. I found no public mention; announcements happen on Discord and Bluesky, which I could not search.
- The UTAU fandom and vocalsynth fandom pages returned HTTP 402.

## 2. Which other DiffSinger voicebanks are well regarded for English (2025–2026)?

### Takeaway
English-native DiffSinger banks are still scarce. The DiffSinger Wiki "supports English" category lists only six: Canary, Hanami Hoshino, Laru Mine, Miyo, Peiton and TIGER. The most active English-capable producer in 2025–26 is LUNAI Project, which has 20 multi-language singers at v170, including the English-singing Katyusha, Keiro Revenant and Liam Thorne. Native-English banks that come up repeatedly are Peiton (ARPAbet, 7 modes), TIGER, Raine Rena, Hoshino Hanami and Nishiren Gard. No public, dated community ranking exists; sentiment lives on Discord.

### Cited Findings
- DiffSinger Wiki "Voicebanks supporting English": Canary, Hanami Hoshino, Laru Mine, Miyo, Peiton, TIGER. Only the American English subcategory has entries — [DiffSinger Wiki category](https://diffsinger.miraheze.org/wiki/Category:Voicebanks_supporting_English)
- Peiton DS, by NebulaMeadow:
  - ARPAbet input through DIFFS EN
  - 7 modes: Americano (base), Chilled, Donut, Dulcet, Espresso, Mocha (operatic), Spiced (power)
  - Commercial use needs a paid licence; AI voice-changer use is forbidden
  - [UTAU Repository: Peiton](https://utaurepository.miraheze.org/wiki/Peiton)
- Search snippets from a UtaForum thread "Looking for english diffsinger AI voicebanks" recommend Raine Rena, Hoshino Hanami ("more pretty than cute") and Nishiren Gard, and name Tiger, POTG and Keiro as other English options. The thread itself would not load (a database error page), so this is snippet-level evidence with no date — [UtaForum thread](https://utaforum.net/threads/looking-for-english-diffsinger-ai-voicebanks.25396/)
- haru0l's list puts Peiton DS and Tiger under English; Monone and Ashera Lyre under Russian with EN/JP — [haru0l DiffSinger page](https://sites.google.com/view/haru0l/diffsinger)
- LUNAI v170 (2025-12-25) includes Aka Subaru, Albus Korde, Allen Crow, Amaboshi Cipher, Denka Shaa, Gahata Meiji, Karasu Yuutsukoe (v160), Katyusha, Keiro Revenant, Kuro Bousuku, Liam Thorne, Llane Crow, M1NT0, Memoka Ater, Monone, Natsume Juli, Nero, Nessie, Ohne Kuno and SAiFA. Each bank is about 410 MB, with downloads between 414 and 3,389 — [GitHub API](https://github.com/lunaiproject/lunai_singers/releases/tag/170)
- A third-party choir-rehearsal app (sjespersen/singer) chose LUNAI's Katyusha (soprano/alto), Keiro Revenant (tenor) and Liam Thorne (baritone/bass) as its **English** voices. Its `setup.sh` downloads them rather than bundling them — [sjespersen/singer](https://github.com/sjespersen/singer)
- LUNAI also maintains a fork, "OpenUtau-lunai" (37 stars, releases 0.2.0.0 on 2026-07-01 through 0.2.1.0 on 2026-07-22, pushed 2026-09-26). It has a "Singer Hub" for browsing and installing DiffSinger voicebanks — [keirokeer/OpenUtau-lunai](https://github.com/keirokeer/OpenUtau-DiffSinger-Lunai)
- Local file: TIGER's shared acoustic model also carries English-capable speakers such as Canary (arc, spark, voltage), Leif (blossom, lush, petal, soil, uprooted) and Miyo. These are separate community banks whose data trained into the same model. They are probably not licensed for use as TIGER, so check before exposing them.
- An "NGYY_ENG_Dataset" English DiffSinger dataset was reportedly updated on 2026-03-19 (search snippet only, not verified) — [GitHub topic diffsinger](https://github.com/topics/diffsinger)

### Inferences
- For English, the candidates worth testing against TIGER, Lilia and Meiji are LUNAI's Katyusha, Keiro Revenant, Liam Thorne and Allen Crow (they share Meiji's packaging, and Resonance already handles it), plus Peiton, which is ARPAbet-native and so a natural fit for CMU-dict G2P.
- No source supports calling any one bank "the best English DiffSinger bank in 2026". Treat quality claims as needing a listening test.

### Gaps
- No dated Reddit, YouTube or Discord ranking turned up. r/UTAU searches returned only UTAU (concatenative) itch.io banks.
- Raine Rena, Hanami Hoshino, Nishiren Gard, Canary and Laru Mine: versions, dates and ToS not retrieved.

## 3. Variance controls and vocoders per bank

### Takeaway
Of the three, only Lilia ships a full variance model (it predicts energy, breathiness, voicing and tension), but its acoustic model consumes only tension. Meiji predicts and consumes only tension. TIGER has no variance model and its acoustic model takes none of the four curves. TIGER and Meiji use tigermeat's custom "tgm_hifigan" vocoder. Lilia uses a fine-tuned NSF-HiFiGAN and names pc_nsf_hifigan 2025.02 as its default.

### Cited Findings
- **TIGER v106** (local file):
  - Models: `dsdur` and `dspitch` (`use_expr: true`), no `dsvariance`
  - Acoustic embeddings: energy, breathiness, voicing and tension all false; `use_variable_depth: true`, `max_depth: 0.6`
  - Vocoder: bundled "tgm_hifigan" r03 (`dsconfig` names `nsf_hifigan_44.1k_hop512_128bin_2024.02`, overridden by the bundled `dsvocoder`)
  - The DiffSinger Wiki lists "Gender, Velocity" as the supported parameters, i.e. key-shift and speed embeds — [DiffSinger Wiki](https://diffsinger.miraheze.org/wiki/TIGER)
- **Lilia MM2.8** (local file):
  - `dsvariance` predicts energy, breathiness, voicing and tension
  - Acoustic: only `use_tension_embed: true`, plus key-shift and speed; `use_variable_depth: false`
  - Vocoder: `pc_nsf_hifigan_44.1k_hop512_128bin_2025.02`, with a bundled `dsvocoder` "mm_finetune" `nsf_hifigan.onnx`
  - README: TENC (tension), VELC (speed) and GENC (gender/throat length); variance trained with Reflow for 500k steps, acoustics with DDPM for 800k (PoC-era figures) — [README](https://github.com/julieraptor/DIFFSINGER-LIEE-Immortal-Idol)
- **Meiji v160** (local file):
  - `dsvariance` and `dspitch` predict tension only; acoustic `use_tension_embed: true`
  - `use_lang_id: true`, `use_variable_depth: true`, `max_depth: 0.6`
  - Vocoder: `tgm_hifigan_v110.onnx`, the same family as TIGER's
- The openvpi reference vocoder line runs NSF-HiFiGAN (2022.12 and 2024.02), then **PC-NSF-HiFiGAN 2025.02**, released as SingingVocoders v1.0.0 on 2025-03-22. That release says to use and distribute it "under the terms described in the original release page" — [openvpi/SingingVocoders releases](https://github.com/openvpi/SingingVocoders/releases)
- Since v2.5.1, DiffSinger writes `pc_nsf_hifigan_44.1k_hop512_128bin_2025.02` as the default vocoder in generated `dsconfig.yaml` — [DiffSinger v2.5.1](https://github.com/openvpi/DiffSinger/releases/tag/v2.5.1)
- DiffSinger v2.3.0 (2024-03-11) added the voicing and tension parameters — [DiffSinger releases](https://github.com/openvpi/DiffSinger/releases)

### Inferences
- In all three banks, energy and breathiness controls do nothing at the acoustic stage. Lilia predicts them, but its acoustic model ignores them. A UI exposing breathiness or energy curves would be inert for all three; only tension (Lilia, Meiji) and gender/speed (all) are live.
- I found no RefineGAN or other non-HiFiGAN vocoder in mainstream English DiffSinger banks.

### Gaps
- Variance and vocoder configs of the LUNAI English singers and of Peiton were not inspected.

## 4. Licensing / terms of use

### Takeaway
None of the three may be freely bundled with an app.
- **TIGER** (CC BY-NC-ND 4.0 + Commons Clause) forbids redistribution and commercial use without a paid licence.
- **LUNAI/Meiji** forbids "distributing models or their copies" and needs written permission for commercial use.
- **LIEE** is the most permissive for songs: free commercial use of the output, with credit "LIEE". But it states: "You cannot use LIEE in AI-generated music and other generative AI contexts", which is a real risk for an LLM-driven composition workflow.
- The openvpi NSF-HiFiGAN vocoders are themselves CC BY-NC-SA 4.0.

### Cited Findings
- **TIGER LICENSE.md**: "CC BY-NC-ND 4.0 & Commons Clause License … may not be used, in whole or in part, for commercial purposes without the express written permission from the copyright holder and/or purchase of a commercial usage license." The README adds: "does not permit redistribution or creations of derivatives". A commercial licence is sold on Ko-fi. NSFW content is allowed and credit is required — [LICENSE.md](https://github.com/spicytigermeat/tiger_diffsinger); [DiffSinger Wiki](https://diffsinger.miraheze.org/wiki/TIGER)
- **LIEE ToS** (first published 2025-08-12):
  - "free to use the Database for individual commercial, non-commercial, and corporate use"
  - Exported audio may be used commercially without permission; credit as "LIEE" is required
  - Forbidden: using its output with voice-changers (RVC, Diff-SVC), "automated voice service", sample packs, or building or supplementing another database
  - Forbidden: "You cannot use LIEE in AI-generated music and other generative AI contexts", and porting needs the creator's contact
  - Redistributing the voicebank files is not addressed explicitly
  - [LIEE Terms of Use](https://docs.google.com/document/d/1t13WKc7VA2Q3G_MppznbAwhl3vLIL2MNPyjVN3kWDDg/edit)
- **LUNAI ToS** (updated 2026-01-16), which covers Meiji, Katyusha, Liam Thorne and the rest:
  - Non-commercial use requires the credit "<Name> from LUNAI Project"
  - Commercial use needs written permission (email lunaiproject@gmail.com)
  - Prohibited: "Porting models to other engines … or modifying them", "Training third-party AI models using derived audio files", "Creating or distributing 'voice packs'", "Distributing models or their copies", reverse engineering, and chatbot or roleplay use
  - Meiji is on the 18+-allowed list
  - [LUNAI terms of use file in lunaiproject/lunai_singers](https://github.com/lunaiproject/lunai_singers); canonical at https://lunaiproject.github.io/termsofuse/
- A third-party project describes the LUNAI English banks as "licensed CC BY-NC-SA 4.0 with the LUNAI Project terms of use … non-commercial use only" and downloads them at setup rather than bundling — [sjespersen/singer](https://github.com/sjespersen/singer)
- A 2026-09-25 licence survey by another DAW project concluded that "no English voicebank under a clearly compatible license was identified". It reports that the reference vocoder is CC-BY-NC-SA 4.0 — [sourdaw issue #4608](https://github.com/jcosta33/sourdaw/issues/4608)
- **Peiton**: commercial use needs a purchased licence; AI voice-changer use is forbidden — [UTAU Repository](https://utaurepository.miraheze.org/wiki/Peiton)
- DiffSinger code is Apache-2.0 (since v2.0.0), MakeDiffSinger is BSD-3-Clause, and OpenUtau is MIT — [GitHub API repo metadata](https://github.com/openvpi/DiffSinger)

### Inferences
- If Resonance ever distributes binaries to others, TIGER and Meiji must be user-downloaded, not bundled. Lilia's redistribution status is undefined, so ask the creator.
- Resonance's MCP/agent-driven vocal generation plausibly counts as a "generative AI context" under the LIEE ToS. Lilia being the *default* voicebank deserves a deliberate look, ideally a question to julieraptor (liee.immortal.idol@gmail.com).
- Meiji's ban on "porting to other engines … or modifying" is ambiguous for a non-OpenUtau ONNX host like Resonance, which runs the unmodified ONNX files.

### Gaps
- The exact licence text of PC-NSF-HiFiGAN 2025.02 (openvpi.github.io/vocoders) was not fetched; CC BY-NC-SA 4.0 comes from a secondary source.
- The terms of tigermeat's tgm_hifigan vocoder, which Meiji also bundles, were not found.

## 5. State of DiffSinger and OpenUtau, 2025–2026

### Takeaway
openvpi/DiffSinger is actively developed. The last tag is v2.5.1 (2026-01-08). The master branch was getting commits through September 2026 (dual-timestep reflow, LYNXNet2 kernels, an energy calculation fix), and a "v3" branch exists. OpenUtau is very active, with near-daily alpha releases (0.1.572.4-alpha on 2026-10-07) and many DiffSinger fixes in September and October 2026.

### Cited Findings
- DiffSinger tags:
  - v2.0.0 2023-07-16
  - v2.1.0 2023-08-26
  - v2.2.0 2023-11-22: melody encoder, ornaments
  - v2.3.0 2024-03-11: voicing and tension
  - v2.4.0 2024-07-13: Rectified Flow is the default; exported ONNX uses the continuous-acceleration API and log-e mels
  - v2.5.0 2025-04-01: universal multi-dictionary/multilingual support, LYNXNet backbone, RoPE; breaking dataset config
  - v2.5.1 2026-01-08
  - [GitHub API releases](https://github.com/openvpi/DiffSinger/releases)
- v2.5.1 explains that multi-dictionary language embeddings "has never worked correctly since its release": language IDs were all zeros, so the ONNX models embedded never-trained vectors. It also switches the default vocoder to pc_nsf_hifigan 2025.02 — [v2.5.1 release](https://github.com/openvpi/DiffSinger/releases/tag/v2.5.1)
- Commits in August and September 2026 include:
  - "Support dual-timestep reflow (#323)" (2026-09-15)
  - "fix energy calculation (#332)" (2026-09-15)
  - "Triton fused SoftSignGLU kernel for LYNXNet2" (2026-08-13)
  - "preserve note cent offsets during transposition" (2026-09-23)
  - Repo last pushed 2026-09-26. Branches include v3, v3-train, v3-lora and lynxnet2; the v3 branch was last committed 2026-05-30
  - [GitHub API commits/branches](https://github.com/openvpi/DiffSinger)
- MakeDiffSinger: no release since 2023-05-28 (SlurCutter), but pushed 2026-03-11 — [GitHub API](https://github.com/openvpi/MakeDiffSinger)
- OpenUtau (openutau/OpenUtau, the same repo as stakira/OpenUtau): 0.1.571-beta 2026-10-03 and 0.1.572.4-alpha 2026-10-07 — [GitHub API](https://github.com/openutau/OpenUtau/releases). DiffSinger commits in September and October 2026:
  - "Serialize DirectML session creation…" (#2494, 2026-10-06)
  - in-app `dsconfig.yaml`/`character.yaml` editing (#2456)
  - "always retake pitch locally" (#2406)
  - "Fix Lang ID assignment on Diffsinger singers without IDs" (#2412)
  - "Add real-time (live) DiffSinger pitch generation" (2026-09-05)
  - merged-phrase rendering fixes (#2388)
  - "Fix/diff singer g2p" (#2421)
  - [GitHub API commits, path OpenUtau.Core/DiffSinger](https://github.com/openutau/OpenUtau)
- DiffScope, openvpi's own editor, is still listed as "under development" — [DiffSinger README](https://github.com/openvpi/DiffSinger)

### Inferences
- Banks trained with v2.5.0 multi-dictionary support before the v2.5.1 fix (roughly April 2025 to January 2026) have untrained language embeddings. Lilia MM2.8 (`use_lang_id: false`) is unaffected. Meiji v160 (`use_lang_id: true`, August 2025) may have been trained in that window; v170 (December 2025) also predates the fix. Unconfirmed: LUNAI may use its own training setup.
- I saw no breaking ONNX-interface change after v2.4.0's continuous-acceleration and log-e mel switch. Dual-timestep reflow and LYNXNet2 may change exported model inputs once tagged, so watch for a v2.6 or v3.

### Gaps
- Whether the DiffSinger v3 branch changes the ONNX export contract. The README on that branch is unchanged, and I did not diff the export code.
