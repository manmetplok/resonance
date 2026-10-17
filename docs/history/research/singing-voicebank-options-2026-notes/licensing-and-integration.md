# Licensing, runtime and integration constraints for local SVS in an MIT/Apache Rust DAW (as of 2026-10-07)

Local context (from the repo, not the web): `resonance-svs` pins `ort = "=2.0.0-rc.12"` with `download-binaries`, `copy-dylibs`, `api-24`, `tls-rustls`, plus optional `cuda` / `rocm` features — [resonance-svs/Cargo.toml](../../../../resonance-svs/Cargo.toml). The app knows three banks (TIGER, Lilia, Meiji), runs acoustic + vocoder on CPU with `speedup` 20 / `depth` 1000, and prefers a bank-bundled vocoder over a shared `vocoder/dsvocoder/` — [docs/features/compose/vocal-rendering.md](../../../../docs/features/compose/vocal-rendering.md).

## Licensing: voicebank terms, alternative model licenses, ship vs. user-installs

### Takeaway
Nothing in the DiffSinger ecosystem that sounds good in English is permissively licensed end to end: the code (openvpi DiffSinger) is Apache-2.0, but the standard openvpi NSF-HiFiGAN / PC-NSF-HiFiGAN vocoders are CC BY-NC-SA 4.0 and community voicebanks carry bespoke character ToUs (no derivatives, permission for commercial use or porting, bans on AI-voice-changer input). An MIT/Apache app can ship the engine but should treat every bank and the vocoder as a user-installed asset; among the alternatives only song generators (ACE-Step, YuE, DiffRhythm) and RVC are permissively licensed, and those are either huge or not text-to-singing.

### Cited Findings
**DiffSinger code and vocoders**
- openvpi/DiffSinger: "This forked DiffSinger repository is licensed under the Apache 2.0 License." The README also carries a usage restriction: "Any organization or individual is prohibited from using any functionalities ... to generate someone's voice without his/her consent." Deployment targets listed: OpenUTAU, DiffScope. The README says nothing about a license for weights. — [openvpi/DiffSinger](https://github.com/openvpi/DiffSinger)
- openvpi vocoders: NSF-HiFiGAN v1 (Dec 2022, 44.1 kHz), finetuned NSF-HiFiGAN (Feb 2024, range to C6), PC-NSF-HiFiGAN (Feb 2025, pitch-shift capable, range to D#7). "Pretrained models are released under the Attribution-NonCommercial-ShareAlike 4.0 International license." They are distributed for DiffSinger inference, SingingVocoders fine-tuning, and OpenUTAU (as ONNX). The page lists no exemption for OpenUtau or DiffSinger use. — [openvpi/vocoders releases](https://github.com/openvpi/vocoders/releases)
- Another open-source DAW project (sourdaw, browser DAW) reached the same conclusion in its own admissibility review: the reference vocoder is CC-BY-NC-SA 4.0, and "no English voicebank under a clearly compatible license was identified". Its open questions were whether its distribution counts as non-commercial and whether share-alike reaches app code or generated audio. — [sourdaw issue #4608](https://github.com/jcosta33/sourdaw/issues/4608)

**Community voicebank ToUs (examples)**
- LIEE "Lilia" (julieraptor; DiffSinger v2.6 "MM", August 2025; recorded JA/EN/KO/Latin/Tagalog/Cebuano/ES/ZH; optimal C3–G5). — [GitHub repo](https://github.com/julieraptor/DIFFSINGER-LIEE-Immortal-Idol)
  - ToU quotes: "You can use exported audio files using the Database in commercial or non-commercial works, without the Creator's permission." / LIEE "must be mentioned in the description … credited as 'LIEE'." / "You cannot use the raw data or exported audio files of the Database as an input for AI-based voice-changers and instruments (such as Diff-SVC or RVC)." / "You cannot use raw data or exported audio file outputs to create a new database, or to supplement another database. **Please contact the Creator if you would like to port LIEE to another singing synthesizer program.**" — [LIEE ToU (Google Doc)](https://docs.google.com/document/d/1t13WKc7VA2Q3G_MppznbAwhl3vLIL2MNPyjVN3kWDDg/edit?usp=sharing)
  - A search summary of the same ToU says "commercial use of the voicebank requires permission" and "use with AI voice changers requires permission". Read alongside the doc, this means commercial use of the *bank* (as opposed to rendered audio) needs permission. That is my reading of a secondary summary. — [LIEE ToU](https://docs.google.com/document/d/1t13WKc7VA2Q3G_MppznbAwhl3vLIL2MNPyjVN3kWDDg/edit?usp=sharing)
  - Further rule: do not release a model pre-trained on LIEE that is meant to recreate a celebrity, a paid vocal-synth voice or a copyrighted character. Breaching the ToU obliges the user to cease use and remove files. — [julieraptor repo](https://github.com/julieraptor/DIFFSINGER-LIEE-Immortal-Idol)
- TIGER (spicytigermeat; v106, 2024-04-23, reflow backend; EN/JA/ZH/ES/FR/IT/PT-BR/TH/RU/KO): commercial use of the voicebank or the character "requires purchase of commercial license" (Ko-fi). "Derivatives of any sort are forbidden." A ~30-min public dataset is offered for parallel learning of DiffSinger voice libraries only. The wiki says nothing on redistribution; full terms are at tigermeat.xyz. — [DiffSinger Wiki: TIGER](https://diffsinger.miraheze.org/wiki/TIGER)
- UFR Team "Millefeuille" multi-provider DiffSinger banks: resulting models "must be free upon distribution"; "use of renders or model assets for creating new products, corpuses, or AI training is prohibited"; commercial use of renders in ads or products is prohibited (doujin music sales excepted); use with "generative AI art, ChatGPT and similar software, cryptocurrencies, and NFTs" is banned. — [search summary of UFR/HoodyP ToU](https://hoodypprojectameko.carrd.co/) (aggregated by the search tool; the clause wording was not checked against the primary ToU page)

**Alternative models**
- ENUNU (NNSVS → UTAU bridge): MIT. — [oatsu-gh/ENUNU](https://github.com/oatsu-gh/ENUNU)
- ESPnet VISinger2 multi-singer pretrained (zh/ja): code is MIT, and the model card says the "models themselves follow their respective official licenses". A search snippet gives CC-BY-4.0 for the weights; the README does not name the training datasets. — [espnet/mixdata_svs_visinger2 model card](https://huggingface.co/espnet/mixdata_svs_visinger2_spkemb_lang_pretrained_avg/blob/main/README.md)
- DiffRhythm: "released under the Apache License 2.0" (code and weights). It asks users to verify originality and disclose AI involvement. — [ASLP-lab/DiffRhythm](https://github.com/ASLP-lab/DiffRhythm)
- YuE: Apache 2.0 (commercial use permitted). ACE-Step 1.5: MIT. — [it-jim 2026 roundup](https://www.it-jim.com/blog/best-open-source-ai-music-generator/), [boppy roundup](https://boppy.me/blog/best-open-source-ai-music-models). **Conflict:** the ACE-Step 1.5 arXiv HTML header shows "CC BY 4.0", which is probably the paper's license rather than the code's — [arXiv 2602.00744](https://arxiv.org/html/2602.00744v1). Check against the repo LICENSE.
- Voice conversion: RVC is MIT; so-vits-svc 4.1 is AGPL-3.0; Seed-VC is GPL-3.0. — [search summary](https://github.com/svc-develop-team/so-vits-svc), [Plachtaa/seed-vc](https://github.com/Plachtaa/seed-vc), [RVC Wikipedia](https://en.wikipedia.org/wiki/Retrieval-based_Voice_Conversion)
- libwaifu treats "Seed-VC and ASTRAL are GPL-3.0, code and weights" and puts its port behind an off-by-default `gpl` build flag (FFmpeg `--enable-gpl` pattern): "A default build contains no GPL code and is MIT again." — [libwaifu PR #74](https://github.com/ling0322/libwaifu/pull/74)

### Inferences
- **Ship the engine, not the voice.** The DiffSinger code path (Apache-2.0) is compatible with MIT OR Apache-2.0. The vocoder (CC BY-NC-SA) and every surveyed English bank are not. Bundling them in release artifacts would put NC/SA or bespoke non-OSI terms into the distribution. A "user installs bank + vocoder" model (as OpenUtau does) keeps the app's license clean and leaves ToU acceptance with the user.
- **Lilia's ToU has a porting clause** ("contact the Creator if you would like to port LIEE to another singing synthesizer program"). Resonance loads Lilia directly rather than via OpenUtau, so it arguably falls under that clause. Asking julieraptor for written OK is cheap and removes doubt. Lilia's render terms (commercial use OK, credit "LIEE") suit end users. Its ban on using renders as RVC/Diff-SVC input matters if a voice-conversion stage is ever chained after the bank.
- **ToU obligations flow through features.** If Resonance adds an RVC/Seed-VC "re-voice" stage, then for Lilia (and probably UFR banks), feeding bank output into it would breach the bank ToU. The app should surface per-bank flags (credit string, commercial, VC-allowed) from bank metadata or a curated table.
- **The vocoder is the bottleneck.** Even a permissively licensed bank would still need a permissive vocoder. Options: train or fine-tune an own vocoder (HiFi-GAN/Vocos/BigVGAN code is permissive, but the weights depend on the training data), or ask openvpi about terms. Whether CC BY-NC-SA "SA" reaches rendered audio or the host app is unresolved (sourdaw Q-001); it is not a basis for bundling.
- **GPL/AGPL VC** (Seed-VC, so-vits-svc) could only ever be an optional external tool or a separately licensed plugin. The libwaifu feature-flag model is a precedent.
- Song generators are permissively licensed but solve a different problem (whole songs from text and lyrics, not note-accurate singing on a user's melody). See the runtime section.

### Gaps
- No English DiffSinger bank was found under an OSI or CC-BY license. I did not do a full survey of every English bank (e.g. Meiji's terms were not fetched; check its readme).
- TIGER's full ToU (tigermeat.xyz) and any explicit redistribution or "host software" clause were not retrieved.
- Primary text of the UFR/Millefeuille ToU not verified.
- The ACE-Step 1.5 code/weights license conflict (MIT vs CC BY 4.0 header) is unresolved here. YuE's Apache-2.0 status rests on secondary roundups, not the repo LICENSE.
- No licensing info found on recent diffusion/flow-matching SVS research models (e.g. post-2024 academic SVS). Most ship no weights or research-only weights, but this was not checked.

## Runtime: CPU/GPU speed, memory, near-interactive re-render feasibility

### Takeaway
Published DiffSinger RTF numbers are GPU-only and old (2021 paper: RTF 0.191 on a V100). Practical CPU speed depends on the diffusion step count (`speedup`/`depth`), and OpenUtau defaults to `speedup` 50 for that reason. Phrase-level CPU re-rendering is how OpenUtau users work, but I found no published CPU RTF. Song generators need multi-GB GPUs (ACE-Step 1.5 ≥4 GB, DiffRhythm ≥8 GB, YuE 7B ≥16 GB) and are not CPU-interactive.

### Cited Findings
- DiffSinger paper: shallow diffusion took RTF from 0.348 to 0.191 on a single V100 (45.1% speed-up), plus 0.14 MOS and 0.5 CMOS gains. — [DiffSinger (AAAI 2022)](https://arxiv.org/pdf/2105.02446)
- OpenUtau: "Diffsinger Rendering speedup is 50 times by default. A smaller speedup may improve ... audio, but slow down the rendering." CPU is the default runner. Windows offers DirectML ("set 'Machine Learning Runner' to 'DirectML' … choose your discrete graphics card"), and macOS offers CoreML (DiffSinger needs macOS 11+). — [OpenUtau wiki: DiffSinger support](https://github.com/openutau/OpenUtau/wiki/DiffSinger-support)
- Resonance currently renders on CPU at `speedup` 20, `depth` 1000, i.e. 2.5x the denoising steps of OpenUtau's default. — [vocal-rendering.md](../../../../docs/features/compose/vocal-rendering.md)
- ACE-Step 1.5: about 2B-parameter DiT plus a Qwen-based LM (1.7B). It "runs locally with less than 4GB of VRAM". A full song takes under 2 s on an A100 and under 10 s on an RTX 3090; the distilled model makes a 240 s track in about 1 s on an A100. The paper does not mention CPU or ONNX. — [arXiv 2602.00744](https://arxiv.org/html/2602.00744v1)
- ACE-Step 1.5 needs ≥4 GB (DiT-only) or ≥6 GB (LLM+DiT). XL checkpoints need ≥12 GB with offloading; 20 GB+ is recommended. — [ACE-Step 1.5 INSTALL / roundup](https://github.com/ace-step/ACE-Step-1.5/blob/main/docs/en/INSTALL.md)
- YuE 7B: 16 GB VRAM minimum at fp16. — [Spheron 2026 guide](https://www.spheron.network/blog/deploy-open-source-ai-music-generation-gpu-cloud-2026/)
- DiffRhythm-base needs at least 8 GB VRAM (`--chunked` at that size). Base makes 1m35s songs, full makes 4m45s; no RTF stated. — [ASLP-lab/DiffRhythm](https://github.com/ASLP-lab/DiffRhythm)

### Inferences
- With reflow/rectified-flow banks (TIGER v106 is "reflow" — [wiki](https://diffsinger.miraheze.org/wiki/TIGER)), the acoustic step count is the main CPU cost. Rendering at OpenUtau-like step counts (~20 steps, `speedup` 50 at depth 1000) would cut acoustic time about 2.5x versus the current `speedup` 20. That trade is worth measuring in-tree with the existing `render_resonance_vocal` example.
- For "near-interactive", copy OpenUtau's structure: render per phrase, cache per phrase (`DiffSingerCache.cs`), and re-render only phrases whose inputs changed. Treat the vocoder as a separate, cheap pass (HiFi-GAN-class vocoders are a single forward pass).
- Song generators are GPU-batch tools, not note-level instruments. In a DAW they suit an optional "generate a reference/demo" job, not the vocal track's edit loop.

### Gaps
- No published CPU RTF or wall-clock numbers for DiffSinger ONNX (acoustic or vocoder) on desktop CPUs were found. Measuring locally is the only reliable source.
- Bank size on disk / RAM per stage (acoustic, variance, pitch, duration, vocoder) was not found in sources. It can be measured from the installed banks.
- No CPU/RTF data found for VISinger2, NNSVS, RVC or Seed-VC in this pass.

## Integration: ONNX export, Rust runtimes, execution providers, English G2P, vocoder handling

### Takeaway
`ort` 2.0 is still a release candidate (rc.13, 2026-07-28; Resonance pins rc.12) but is described as production-ready. It supports CUDA/TensorRT/MIGraphX/DirectML/CoreML/OpenVINO/WebGPU/XNNPACK and has pure-Rust backends (ort-tract, ort-candle) as alternative backends. ONNX Runtime **removed the ROCm EP in 1.23**, so Resonance's `rocm` feature needs to move to MIGraphX. OpenUtau's English DiffSinger path is lexicon-first: a per-bank `dsdict` (ARPAbet-like) plus built-in ARPAbet G2P for out-of-dictionary words.

### Cited Findings
**ort and runtimes**
- Latest ort is 2.0.0-rc.13 (2026-07-28), "production-ready and recommended for new and existing projects". Earlier RCs: rc.12 (2026-03-05), rc.11 (2026-01-07), rc.10 (2025-06-01). — [docs.rs ort](https://docs.rs/crate/ort/latest), [lib.rs](https://lib.rs/crates/ort)
- ort is "primarily a wrapper for Microsoft's ONNX Runtime library, but also offers support for other pure-Rust runtimes". — [ort.pyke.io](https://ort.pyke.io/)
- EP docs (rc.13): CUDA (≥13.2, cuDNN ≥9.23), TensorRT, ROCm, MIGraphX, DirectML, CoreML, OpenVINO, WebGPU (experimental, "may produce incorrect results/crashes"; on Linux via Vulkan/OpenGL), XNNPACK. Alternative backends: ort-candle, ort-tract, ort-web. If an EP fails to register, ort silently falls back to CPU unless `.error_on_failure()` is set. — [ort execution providers](https://ort.pyke.io/perf/execution-providers)
- ONNX Runtime: "ROCm Execution Provider has been removed since the 1.23 release"; users should use MIGraphX or Vitis AI. ROCm 7.0 is the last supported distribution. — [ORT ROCm EP docs](https://onnxruntime.ai/docs/execution-providers/ROCm-ExecutionProvider.html), [PR #25181 Delete ROCM EP](https://github.com/microsoft/onnxruntime/pull/25181), [ORT v1.23.0 release](https://github.com/Microsoft/onnxruntime/releases/tag/v1.23.0)

**DiffSinger in OpenUtau (reference host)**
- OpenUtau's `OpenUtau.Core/DiffSinger/` contains `DiffSingerRenderer`, `DiffSingerVocoder`, `DiffSingerVariance`, `DiffSingerPitch`, `DiffSingerCache`, `DiffSingerConfig`, `DiffSingerSpeakerEmbedManager`, `DiffSingerRealCurveScheduler`, `RealTimePitchGenerationService`, `DiffSingerRetake`, among others. That is a full multi-stage pipeline with caching, speaker embeddings, retakes and real-time pitch generation. — [OpenUtau DiffSinger dir](https://github.com/openutau/OpenUtau/tree/master/OpenUtau.Core/DiffSinger)
- English phonemizers: `DiffSingerEnglishPhonemizer`, `DiffSingerARPAPlusEnglishPhonemizer`, `DiffSingerG2pPhonemizer`, plus about 24 others including `DiffSingerRhythmizerPhonemizer`. — [Phonemizers dir](https://github.com/openutau/OpenUtau/tree/master/OpenUtau.Core/DiffSinger/Phonemizers)
- G2P: `ArpabetG2p.cs`, `ArpabetPlusG2p.cs` (plus other languages), with a `Data/` subdirectory for the lexicon and model resources. — [OpenUtau G2p dir](https://github.com/openutau/OpenUtau/tree/master/OpenUtau.Core/G2p)
- Vocoder handling: the user downloads nsf_hifigan and drag-drops it into OpenUtau (a separate package, not bundled). — [OpenUtau wiki](https://github.com/openutau/OpenUtau/wiki/DiffSinger-support)
- Per-bank dictionaries: banks ship `dsdict` YAMLs (e.g. `dsdict-enx.yaml`, also in `dsdur`/`dspitch`). TIGER's "DIFFS EN X" phonemizer uses [ax]/[dx] for American English. — [tiger_diffsinger usage guide](https://github.com/spicytigermeat/tiger_diffsinger/blob/main/wiki_usage_guide.md). A community English dict uses "a slightly modified ARPAbet with extensions useful for singing in UK/AU/US/CAD English". — [intunist/diffsinger-english-support](https://github.com/intunist/diffsinger-english-support)
- Known weakness of dict-only: "if a word isn't in the dsdict, it just doesn't know what to do", hence G2P plugins. — [UTAU SnR blog, 2025-02](https://utausnr.blogspot.com/2025/02/trying-to-make-diffsinger-g2p.html). Community ARPAsing phonemizers add vowel and consonant fallbacks. — [Cadlaxa Syllable-Based-ARPAsing-Phonemizer](https://github.com/Cadlaxa/Syllable-Based-ARPAsing-Phonemizer)
- Users swap `dsdict-en` files to mix languages (e.g. Korean+English). — [Team Coda on X](https://x.com/CodaSVS/status/1944805893440463273)

**Other integrations**
- libwaifu (C++ voice library) ports Seed-VC v2 (HuBERT, ASTRAL, CFM DiT, AR) natively and gates the GPL code behind a flag. — [libwaifu PR #70](https://github.com/ling0322/libwaifu/pull/70), [PR #74](https://github.com/ling0322/libwaifu/pull/74)

### Inferences
- **ROCm action item:** Resonance's `rocm = ["ort/rocm"]` targets an EP that ONNX Runtime no longer ships (≥1.23). Switch to MIGraphX, or rely on the CPU fallback. Because ort falls back to CPU silently, a "GPU" build could quietly run on CPU; use `.error_on_failure()` or log the active EP.
- **Bumping ort** from rc.12 to rc.13 is low-risk per upstream messaging, but rc.13 docs cite CUDA ≥13.2, which matters for the `cuda` feature.
- **tract/candle as pure-Rust fallbacks:** reachable through ort-tract/ort-candle, but whether they cover DiffSinger's graphs (diffusion loops, variable-length ops, speaker embedding gathers) is unverified. ONNX Runtime CPU remains the safe default.
- **English G2P:** OpenUtau's design (lexicon first, ARPAbet G2P fallback, per-bank `dsdict` mapping onto the bank's symbol set) is the proven pattern. Resonance already substitutes symbols per bank (e.g. Lilia `v` → `f`; vocal-rendering.md), which matches the per-bank `dsdict` role. Lexicon packaging: CMUdict (BSD-style) could be bundled; bank-shipped `dsdict` files fall under the bank ToU and should be read from the user's install, not vendored.
- **Vocoder:** keep it a separately installed, swappable asset. Prefer a bank-bundled vocoder when present (Resonance already does), and record which vocoder rendered a take, since PC-NSF vs NSF output differs.
- **Song generators via ONNX:** no ONNX export path was found for ACE-Step/YuE/DiffRhythm. Integrating one would mean Python sidecars or large ports, a different class of work from DiffSinger.

### Gaps
- Could not load the OpenUtau DiffSinger wiki in full or DeepWiki's English-phonemizer page (HTTP 429), so the exact file layout (`dsconfig.yaml`, `character.yaml`, `dsvariance/`, `dsdur/`, `dspitch/`) and the ONNX Runtime package OpenUtau uses (Microsoft.ML.OnnxRuntime.DirectML etc.) are not cited here.
- Whether OpenUtau's built-in English G2P is an ONNX seq2seq model or a CMUdict lookup was not confirmed from source.
- No evidence found on burn/candle/tract running DiffSinger or VISinger ONNX graphs end to end.
- ONNX export availability for NNSVS, VISinger2 (ESPnet) and RVC was not confirmed in this pass.

## How other open-source hosts integrate SVS, and lessons

### Takeaway
OpenUtau is the main open-source DiffSinger host. It keeps the engine Apache/MIT and has users install banks and the vocoder themselves, gives CPU by default with DirectML/CoreML options and a user-facing speed/quality knob, and splits English into phonemizer classes plus per-bank dictionaries. I found no VST/CLAP DiffSinger singing plugin. The only DAW-side precedent found (sourdaw) blocked English SVS on licensing, not engineering.

### Cited Findings
- OpenUtau's model: user-imported vocoder, CPU default, DirectML/CoreML options, `speedup` default 50 as the speed/quality trade-off. — [OpenUtau wiki](https://github.com/openutau/OpenUtau/wiki/DiffSinger-support)
- OpenUtau's renderer is split into pitch, variance, duration (phonemizer-side) and acoustic+vocoder stages, with a render cache, retakes, speaker-embed manager and real-time pitch service. — [OpenUtau DiffSinger dir](https://github.com/openutau/OpenUtau/tree/master/OpenUtau.Core/DiffSinger)
- openvpi lists OpenUTAU and DiffScope (under development) as deployment frontends. — [openvpi/DiffSinger](https://github.com/openvpi/DiffSinger)
- ENUNU bridged NNSVS models into UTAU as a plugin (MIT). — [oatsu-gh/ENUNU](https://github.com/oatsu-gh/ENUNU)
- sourdaw (browser DAW) blocked English DiffSinger singing for want of a license-compatible English bank and vocoder. Its options: legal confirmation on CC-BY-NC-SA, finding permissive assets, or building a first-party voicebank. — [sourdaw #4608](https://github.com/jcosta33/sourdaw/issues/4608)

### Inferences
- Lessons for Resonance: (1) keep banks and the vocoder out of the repo and its release artifacts, and give a guided "install a bank" flow with ToU display and a credit string. (2) Expose a speedup/quality preference, as OpenUtau does. (3) Cache per phrase and re-render diffs. (4) Use per-bank dictionary and symbol mapping with a G2P fallback. (5) Treat GPU EPs as opt-in and verify they are actually active. (6) A first-party permissively licensed voice (own recordings plus own vocoder training) is the only route to bundling a voice in an MIT/Apache app.

### Gaps
- No VST3/CLAP singing-synth plugin wrapping DiffSinger, NNSVS or VISinger was found in this pass. Absence was not exhaustively confirmed.
- DiffScope's status in 2026 was not checked.
