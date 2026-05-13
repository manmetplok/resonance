#!/usr/bin/env bash
# Fetch the English DiffSinger voicebank + vocoder into experiments/svs-poc/models/.
# Idempotent: skips downloads that already exist.
#
# Models:
#   - TIGER (English DiffSinger v106) — 529 MB
#       https://github.com/spicytigermeat/tiger_diffsinger  (CC BY-NC-SA 4.0)
#   - tgm_hifigan v110 (companion NSF-HiFiGAN vocoder) — 50 MB
#       https://github.com/mrtigermeat/tgm_hifigan          (CC BY-NC-SA 4.0)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODELS="$ROOT/models"
mkdir -p "$MODELS/singer" "$MODELS/vocoder"

TIGER_URL="https://github.com/spicytigermeat/tiger_diffsinger/releases/download/v106/TIGER_DS_v106_PACK.zip"
TIGER_ZIP="$MODELS/tiger.zip"

VOCODER_URL="https://github.com/mrtigermeat/tgm_hifigan/releases/download/v110/tgm_hifigan_dsvocoder.zip"
VOCODER_ZIP="$MODELS/vocoder.zip"

if [[ ! -f "$TIGER_ZIP" ]]; then
    echo ">> downloading TIGER (529 MB) ..."
    curl -L -o "$TIGER_ZIP" "$TIGER_URL"
fi
if [[ ! -f "$VOCODER_ZIP" ]]; then
    echo ">> downloading tgm_hifigan (50 MB) ..."
    curl -L -o "$VOCODER_ZIP" "$VOCODER_URL"
fi

if [[ ! -d "$MODELS/singer/TIGER_DS_v106_PACK" ]] && [[ ! -f "$MODELS/singer/dsconfig.yaml" ]]; then
    echo ">> extracting TIGER ..."
    unzip -q -o "$TIGER_ZIP" -d "$MODELS/singer"
fi
if [[ ! -d "$MODELS/vocoder/dsvocoder" ]]; then
    echo ">> extracting vocoder ..."
    unzip -q -o "$VOCODER_ZIP" -d "$MODELS/vocoder"
fi

# TIGER ships the inner voicebank as a nested zip inside the outer pack.
INNER="$MODELS/singer/TIGER_DS_v106_PACK/Voice Library/TIGER_DS_v106.zip"
if [[ -f "$INNER" ]] && [[ ! -f "$MODELS/singer/extracted/dsacoustic/acoustic.onnx" ]]; then
    echo ">> extracting inner TIGER voice library ..."
    unzip -q -o "$INNER" -d "$MODELS/singer/extracted"
fi

# Rewrite dsconfig.yaml to drop the OpenUTAU `tgm_acou_v106.` namespace prefix so
# our PoC config parser finds the bare file names that are actually on disk.
DSCONFIG="$MODELS/singer/extracted/dsacoustic/dsconfig.yaml"
if [[ -f "$DSCONFIG" ]] && ! [[ -f "$DSCONFIG.original" ]]; then
    cp "$DSCONFIG" "$DSCONFIG.original"
    sed -i 's|tgm_acou_v106\.phonemes\.txt|phonemes.txt|; s|tgm_acou_v106\.onnx|acoustic.onnx|; s|^- tgm_acou_v106\.|- |' "$DSCONFIG"
    echo ">> rewrote dsconfig.yaml (original saved as dsconfig.yaml.original)"
fi

# Place the sample .ds (and any matching prerendered WAV) under models/sample/.
mkdir -p "$MODELS/sample"
SAMPLE_DS="$MODELS/sample/hello_tiger.ds"
if [[ ! -f "$SAMPLE_DS" ]]; then
    cp "$ROOT/fixtures/hello_tiger.ds" "$SAMPLE_DS"
fi

echo ">> done. Contents:"
find "$MODELS" -maxdepth 4 -type f \( -name '*.yaml' -o -name '*.onnx' -o -name '*.txt' \) | sed "s|$MODELS/|  |"
