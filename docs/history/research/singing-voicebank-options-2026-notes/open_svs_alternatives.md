# Open / open-weight singing voice synthesis beyond classic DiffSinger voicebanks (as of Oct 2026)

Research date: 2026-10-07. ~19 tool calls; sources are primary (GitHub READMEs, arXiv, HF model cards) unless noted. Several claims about licences come from search-result summaries of READMEs and should be re-checked in the repo's LICENSE file before any integration decision.

## Which open-weight SVS models from 2024-2026 are notable, and what are their licences?

### Takeaway
The most important new option is **SoulX-Singer** (Soul AI Lab, Feb 2026). It is a zero-shot, score-conditioned (MIDI + lyrics) flow-matching SVS, trained on about 20k hours of English singing, and its code **and weights** are Apache-2.0. It is the only 2026 model found that combines English, score input, permissive weights and active tooling. Most other strong research models are either Mandarin-first (VocalRender), released without usable weights (TCSinger 2), or carry non-commercial/no-derivatives weight licences (Vevo2, YuE2).

### Cited Findings
**SoulX-Singer (Soul-AILab)**
- Released 2026-02-06. Apache-2.0: "free to use the codes and model weights". Later additions: a MIDI Editor tool (2026-02-08), an HF Spaces demo (2026-02-09), an eval dataset (2026-02-12), and a separate SVC model (2026-03-16). — [GitHub Soul-AILab/SoulX-Singer](https://github.com/Soul-AILab/SoulX-Singer)
- Architecture: a DiT flow-matching decoder predicts mel, and a neural vocoder turns it into a waveform. A "Singing Content Encoder" aligns lyrics, score, note types and F0, and a length regulator expands embeddings by note duration. — [arXiv 2602.07803](https://arxiv.org/html/2602.07803)
- Training data: over 42,000 h, about 20,000 h each of Mandarin and English plus about 2,000 h of Cantonese. English is fed as phonemes wrapped in `<BOW>`/`<EOW>` word-boundary tokens. — [arXiv 2602.07803](https://arxiv.org/html/2602.07803)
- Two modes. **Melody-control** takes a continuous F0 contour, for example from reference audio. **Score-control** takes only MIDI and lyrics, and the model predicts the acoustic features itself. — [arXiv 2602.07803](https://arxiv.org/html/2602.07803)
- Reported English WER on GMO-SVS is 0.151 (melody) and 0.149 (score), against 0.197 for ground truth. It beats StyleSinger, TCSinger, YingMusic-Singer and Vevosing on FFE, WER, SIM and SingMOS. (Self-reported; a WER below ground truth suggests ASR-favourable clean output rather than proof of better pronunciation.) — [arXiv 2602.07803](https://arxiv.org/html/2602.07803)
- The README "strongly recommend[s] manually correcting the alignment" with its MIDI Editor, because automatic preprocessing can misalign. It does not document VRAM needs or ONNX export. It needs Python 3.10 with PyTorch. — [GitHub](https://github.com/Soul-AILab/SoulX-Singer)
- Community uptake: ComfyUI nodes, a Pinokio one-click install, and an open request to add it to vllm-omni (issue #3389). — [ComfyUI-SoulX-Singer](https://github.com/Saganaki22/ComfyUI-SoulX-Singer); [vllm-omni #3389](https://github.com/vllm-project/vllm-omni/issues/3389)

**VocalRender (NTU et al., arXiv 2607.27768, 2026-07-30)**
- A "score-native" autoregressive diffusion model. Its input is composer-style symbolic scores: lyrics, MIDI pitch, note values and BPM, using an interleaved lyric–note representation that handles melisma. It has no explicit duration predictor. — [arXiv 2607.27768](https://arxiv.org/html/2607.27768)
- On subjective naturalness it beats TCSinger, TechSinger, Vevo2 and SoulX-Singer (+0.13 CMOS over the best of them) and has the best MS-MOS (2.96). It was trained on a 2,300 h open-source dataset (CrawlSinger-OS). The evaluation is Mandarin (OpenCPOP WER 4.44). Code: github.com/pymaster17/VocalRender. — [arXiv 2607.27768](https://arxiv.org/html/2607.27768)

**Vevo2 (Amphion, paper Aug 2025, code and checkpoints 2026-03-25)**
- Unifies TTS, SVS, VC, SVC, singing editing, humming-to-singing and instrument-to-singing. Built from an AR model (based on Qwen2.5-0.5B), a flow-matching transformer and a Vocos-based vocoder (about 250M). Trained on 101k h of speech (Emilia) and 7k h of singing. — [HF RMSnow/Vevo2](https://huggingface.co/RMSnow/Vevo2); [arXiv 2508.16332](https://arxiv.org/html/2508.16332v2)
- Weights are **CC-BY-NC-ND-4.0**. The Amphion code is MIT. The model card does not explicitly document SVS from a note score; melody is controlled through prosody tokens and F0. — [HF RMSnow/Vevo2](https://huggingface.co/RMSnow/Vevo2); [Amphion](https://github.com/open-mmlab/amphion)

**TCSinger 2 (ACL 2025, code July 2025)**
- Multilingual zero-shot SVS (zh, en, fr, es, de, it, ja, ko, ru) with prompt-based style, technique and emotion control. Code is MIT. — [GitHub AaronZ345/TCSinger2](https://github.com/aaronz345/tcsinger2)
- The README points users to training their own model and does not clearly offer pretrained checkpoints. Input needs phonemes, phoneme durations, note pitches and durations, plus singer, technique and emotion text. Vocoder is HiFi-GAN. — [GitHub](https://github.com/AaronZ345/TCSinger2)
- Predecessor TCSinger (EMNLP 2024) has the same author and style-transfer focus. — [GitHub AaronZ345/TCSinger](https://github.com/AaronZ345/TCSinger)

**YingMusic-Singer / -Plus (GiantAILab / ASLP-lab, arXiv 2603.24589, Mar 2026)**
- A diffusion/flow-matching *editing* model. It takes an optional timbre reference, a **singing clip that supplies the melody**, and new lyrics. It needs no MIDI and no alignment. It uses a unified IPA tokenizer for Chinese and English and outputs 44.1 kHz with about 727M parameters. — [GitHub YingMusic-Singer-Plus](https://github.com/ASLP-lab/YingMusic-Singer-Plus); [arXiv 2603.24589](https://arxiv.org/html/2603.24589v1)
- Licence is CC BY 4.0, except the VAE, which is under the Stability AI Community License. — [GitHub](https://github.com/ASLP-lab/YingMusic-Singer-Plus)

**DiffSinger (openvpi), the incumbent**
- The latest release is v2.5.1 (2026-01-08). It is still the main community engine behind OpenUtau voicebanks, including English ones. — [openvpi/DiffSinger](https://github.com/openvpi/DiffSinger); [vocalsynth wiki](https://vocalsynth.fandom.com/wiki/DiffSinger)

**Survey resource:** a curated review of deep-learning SVS approaches. — [SyntheticSingers](https://github.com/DaViD-Pigeon/SyntheticSingers); the MMGenre multi-genre SVS benchmark — [arXiv 2607.06986](https://arxiv.org/pdf/2607.06986)

### Inferences
- SoulX-Singer is the realistic "next step up" from DiffSinger voicebanks. Its input contract is the same as what Resonance already produces (notes, lyrics and phonemes, with timing from the score), so the change is mostly the inference runtime, not the data model.
- Integration cost is the blocker. All the new models are PyTorch, DiT-sized research code with no published ONNX. Running them from a Rust app means either a Python sidecar process or our own ONNX export of the encoder, DiT, length regulator and vocoder, plus a flow-matching sampling loop in Rust. That is plausible: DiffSinger's ONNX path already has a diffusion loop.
- VocalRender is the 2026 quality leader on paper, but only for Mandarin. It is unlikely to help English now.

### Gaps
- VRAM and CPU feasibility of SoulX-Singer is undocumented. I found no CPU-only inference reports.
- I found no independent (non-author) blind comparison of SoulX-Singer English against DiffSinger English voicebanks or against Synthesizer V/ACE Studio.
- The VocalRender weights licence was not checked.
- NNSVS/ENUNU and VISinger2 successors were not covered. Searches surfaced no notable 2025-2026 activity for them, which is weak evidence that they have been overtaken.

## Score input vs text-prompt / full-song generators: usable for note-accurate vocal lines?

### Takeaway
Only the SVS models (SoulX-Singer, VocalRender, TCSinger 2, DiffSinger) take a note-level score. The full-song generators (ACE-Step 1.5, YuE/YuE2, DiffRhythm, SongBloom, HeartMuLa) are conditioned on lyrics, style tags and BPM/key. They cannot be held to a given MIDI melody, so they are not usable as a note-accurate vocal-line renderer in a DAW. At most they could be a sketch or idea source, whose vocal stem could be resynthesized.

### Cited Findings
- ACE-Step 1.5 (Feb 2026, MIT): conditioned on style tags, lyrics, duration, BPM, key and time signature. It supports repaint/edit, cover, stem separation and Vocal2BGM, needs under 4 GB VRAM, and takes about 2 s per song on an A100. It does not offer note-level melody input. — [GitHub ace-step/ACE-Step-1.5](https://github.com/ace-step/ACE-Step-1.5); [DEV guide](https://dev.to/czmilo/ace-step-15-the-complete-2026-guide-to-open-source-ai-music-generation-522e)
- YuE: code is Apache-2.0. The original YuE weights became Apache-2.0 (2025-01-30). The YuE2 weights are CC BY-NC 4.0, which restricts commercial use of the weights and their outputs. YuE2 advertises "symbolic planning … editable scores". — [GitHub YuE](https://github.com/multimodal-art-projection/YuE); [Promptus licence explainer (secondary)](https://www.promptus.ai/blog/yue2-commercial-licence-explained)
- DiffRhythm: code and DiT weights are Apache-2.0. It is a full-length song latent-diffusion model. — [GitHub ASLP-lab/DiffRhythm](https://github.com/ASLP-lab/DiffRhythm)
- HeartMuLa is a family of open music foundation models (Jan 2026). — [HF papers 2601.10547](https://huggingface.co/papers/2601.10547)

### Inferences
- YuE2's "editable scores" could make it the first song generator to accept symbolic melody back in. That is not verified, and its NC weights would block shipping it in an MIT/Apache app anyway.
- YingMusic-Singer offers a middle path: it accepts a *sung audio* melody guide rather than MIDI. Resonance could feed it a DiffSinger render as the melody guide and get new timbre and lyrics, but this chains two models.

### Gaps
- I did not verify whether YuE2's symbolic planning accepts user-supplied note input.
- The SongBloom licence was not retrieved.

## English pronunciation, English singers, and zero-shot cloning

### Takeaway
SoulX-Singer has by far the largest English singing training set of the open models (about 20k h). It is zero-shot, so any reference clip becomes the singer: there are no "voicebanks", just a prompt clip. TCSinger 2 and Vevo2 are also zero-shot. The DiffSinger ecosystem still depends on per-singer voicebanks, and English banks are few.

### Cited Findings
- SoulX-Singer offers zero-shot singing for unseen singers with no fine-tuning, and English is a first-class language. — [GitHub](https://github.com/Soul-AILab/SoulX-Singer); [arXiv 2602.07803](https://arxiv.org/html/2602.07803)
- Vevo2 is zero-shot across six languages. Its weights are NC-ND. — [HF RMSnow/Vevo2](https://huggingface.co/RMSnow/Vevo2)
- TCSinger 2 supports cross-lingual zero-shot style transfer, including English. — [GitHub](https://github.com/aaronz345/tcsinger2)
- DiffSinger supports English, Japanese, Spanish, French and others through OpenUtau voicebanks. — [vocalsynth wiki](https://vocalsynth.fandom.com/wiki/DiffSinger)

### Inferences
- Zero-shot cloning changes the product model: the app could ship one model plus a few *licensed* reference clips, instead of per-voicebank downloads. The licence of the reference voice then becomes the legal question, not the model.
- SoulX-Singer's English uses phonemes with word boundaries, which is compatible with Resonance's CMU G2P output. Its exact phone set (ARPAbet or IPA) needs checking.

### Gaps
- Which English phoneme inventory SoulX-Singer uses was not determined.
- I found no listening-test data comparing English diction across these models and DiffSinger.

## Expressiveness controls and external pitch

### Takeaway
SoulX-Singer accepts an **external F0 curve** in melody-control mode, so Resonance could keep its own pitch editing (vibrato and pitch curves) and pass F0 in. Explicit energy, breathiness, tension and vibrato knobs, which DiffSinger's variance models expose, are **not** documented for any of the new zero-shot models. Their expressiveness comes from the reference clip and from text style prompts (TCSinger 2: technique and emotion text).

### Cited Findings
- SoulX-Singer conditions on note pitch, continuous F0 (through binary gating), note types and lyrics. The README does not detail vibrato, energy or breath controls. — [arXiv 2602.07803](https://arxiv.org/html/2602.07803); [GitHub](https://github.com/Soul-AILab/SoulX-Singer)
- TCSinger 2 controls technique, emotion and style through text or natural-language descriptions plus audio prompts. The README does not name vibrato or breathiness. — [GitHub TCSinger2](https://github.com/AaronZ345/TCSinger2)
- Vevo2 controls melody through prosody tokens and F0, and supports humming-to-singing. — [HF RMSnow/Vevo2](https://huggingface.co/RMSnow/Vevo2)
- Synthesizer V Studio 2 (Mar 2025) exposes pitch control points and curves, vocal-mode parameters, and consonant timing through JS/Lua scripting. This is the benchmark for parametric control. — [Synthesizer V Studio 2 wiki](https://synthv.fandom.com/wiki/Synthesizer_V_Studio_2); [Dreamtonics scripting manual](https://resource.dreamtonics.com/scripting/index.html)

### Inferences
- A hybrid is plausible. Keep DiffSinger's variance/pitch predictor, or Resonance's own pitch curve, to generate F0, then render timbre with SoulX-Singer in melody-control mode. That keeps tension and breath control only if DiffSinger renders those first and SVC/melody-mode carries them over, which is unverified.

### Gaps
- None of the new models documents tension, breathiness or energy curve inputs. Confirming this would need a code read.

## Voice-conversion layer on a synthesized guide vocal

### Takeaway
SVC on top of a guide vocal is mature. The ecosystems differ mainly in licence:
- **Applio/RVC** is MIT and very mature, but each target voice needs training.
- **Seed-VC** is zero-shot and good for singing, but GPL-3.0 and archived since Nov 2025.
- **SoulX-Singer's SVC model** (Mar 2026) falls under the Apache-2.0 project.
- **Vevo2** is NC-ND.

### Cited Findings
- Seed-VC does zero-shot VC and SVC from 1–30 s of reference audio. Its singing model is a 44.1 kHz F0-conditioned DiT with a BigVGAN vocoder. It is GPL-3.0, and the repo was archived (read-only) on 2025-11-21. — [GitHub Plachtaa/seed-vc](https://github.com/Plachtaa/seed-vc); [maestro-seedvc licence note](https://github.com/blizaine/maestro-seedvc)
- Applio (RVC toolkit) is MIT for code and weights. It uses RMVPE F0 extraction and added opt-in high-register octave-error correction above about 1040 Hz. RVC requires training a model for each target voice. — [GitHub IAHispano/Applio](https://github.com/iahispano/Applio); [Applio releases](https://github.com/IAHispano/Applio/releases)
- SoulX-Singer released an SVC model on 2026-03-16. — [GitHub](https://github.com/Soul-AILab/SoulX-Singer)
- Newer research on zero-shot SVC: R2-SVC (robust and expressive real-world SVC, arXiv 2510.20677) and OneVoice (unified zero-shot VC, arXiv 2601.18094). — [R2-SVC](https://arxiv.org/pdf/2510.20677); [OneVoice](https://arxiv.org/pdf/2601.18094)

### Inferences
- Seed-VC's GPL licence rules out linking it into an MIT/Apache app. Running it as a separate user-installed process is possible, but it is abandoned upstream.
- An RVC model is a small, ONNX-exportable network (the community has done this widely), so it is the cheapest "make the DiffSinger voice sound like X" layer for a Rust host. Its quality ceiling is bounded by the guide's phrasing and diction.

### Gaps
- No licence or quality data was gathered on the SoulX-Singer SVC weights specifically, beyond the repo-wide Apache-2.0.
- R2-SVC and OneVoice weight availability was not checked.

## Commercial closed benchmarks and local SDK/API access

### Takeaway
Synthesizer V Studio 2 (Mar 2025) and ACE Studio are the quality and control benchmarks. Neither offers an embeddable engine SDK. ACE Studio has an **experimental local MCP server** (HTTP on localhost:21572 or stdio) that lets external agents drive the app. SynthV 2 has in-app JS/Lua scripting. No sources were found for a Vocaloid 6 or Emvoice developer SDK.

### Cited Findings
- Synthesizer V Studio 2 by Dreamtonics was released in March 2025. Its scripting covers retakes, pitch curves, vocal modes, the mixer and consonant timing. — [synthv wiki](https://synthv.fandom.com/wiki/Synthesizer_V_Studio_2); [Dreamtonics scripting](https://resource.dreamtonics.com/scripting/index.html)
- The ACE Studio MCP Server is "currently in experimental phase". It supports Streamable HTTP at `http://localhost:21572/mcp` or STDIO, and is documented for Claude Code and Codex. — [ACE Studio MCP docs](https://docs.acestudio.ai/ai-tools/ace-studio-mcp-server); [External Agent Access](https://docs.acestudio.ai/ai-agent/external-agent-access)
- ACE Studio (Beijing Timedomain) has been a paid subscription since 2023. — [vocalsynth wiki](https://vocalsynth.fandom.com/wiki/Synthesizer_V_Studio); [Sound On Sound review](https://www.soundonsound.com/reviews/ace-studio)

### Inferences
- The ACE Studio MCP route lets an agent produce benchmark-quality vocals with Resonance (both are MCP-driven). But ACE Studio runs on Windows/macOS, which conflicts with Resonance being Linux-only, and it is closed and paid. It is a benchmark, not an integration target.

### Gaps
- Vocaloid 6 and Emvoice SDK/API status was not found.
- I found no rigorous public MOS comparison of open models against SynthV 2 or ACE Studio for English.
