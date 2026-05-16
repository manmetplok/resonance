#!/usr/bin/env bash
# Fetch DiffSinger voicebanks + companion vocoders into
# experiments/svs-poc/models/. Idempotent: skips downloads that already
# exist. SHA256 is verified for any file we download fresh.
#
# Models:
#   - TIGER (English DiffSinger v106) — 529 MB
#       https://github.com/spicytigermeat/tiger_diffsinger    (CC BY-NC-SA 4.0)
#   - tgm_hifigan v110 (companion NSF-HiFiGAN vocoder) — 50 MB
#       https://github.com/mrtigermeat/tgm_hifigan            (CC BY-NC-SA 4.0)
#   - LIEE Lilia (multi-language DiffSinger MM2.8) — 580 MB
#       https://github.com/julieraptor/DIFFSINGER-LIEE-Immortal-Idol
#   - Gahata Meiji v160 (multi-language DiffSinger) — 314 MB
#       https://github.com/lunaiproject/lunai_singers

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

# --- Additional voicebanks ----------------------------------------------------
#
# Each entry: download → sha256 verify → extract into models/voicebanks/<name>/.
# Both packs ship a single zip with a freeform top-level directory, so we
# extract into a temp dir and `mv` the inner contents up so consumers
# don't need to know the upstream pack name.

VOICEBANKS="$MODELS/voicebanks"
mkdir -p "$VOICEBANKS"

# Downloads + verifies + extracts a voicebank zip. Args:
#   $1 = name (used for the destination dir + zip filename)
#   $2 = upstream URL
#   $3 = expected SHA256 of the zip
#   $4 = optional inner zip name (relative to the outer extract) — when
#        non-empty, the outer zip is treated as a wrapper and the inner
#        zip is extracted into the destination dir instead.
fetch_voicebank() {
    local name="$1" url="$2" want_sha="$3" inner="${4:-}"
    local zip="$VOICEBANKS/${name}.zip"
    local dest="$VOICEBANKS/${name}"
    if [[ ! -f "$zip" ]]; then
        echo ">> downloading $name ..."
        curl -L --progress-bar -o "$zip" "$url"
    fi
    local got_sha
    got_sha=$(sha256sum "$zip" | cut -d' ' -f1)
    if [[ "$got_sha" != "$want_sha" ]]; then
        echo "!! $name SHA256 mismatch:"
        echo "   expected: $want_sha"
        echo "   got:      $got_sha"
        echo "   re-download: rm $zip && rerun"
        exit 1
    fi
    if [[ ! -d "$dest" ]]; then
        echo ">> extracting $name ..."
        local tmp
        tmp=$(mktemp -d)
        unzip -q -o "$zip" -d "$tmp"
        if [[ -n "$inner" ]]; then
            local inner_path="$tmp/$inner"
            if [[ ! -f "$inner_path" ]]; then
                echo "!! $name inner zip missing: $inner_path"
                rm -rf "$tmp"
                exit 1
            fi
            mkdir -p "$dest"
            unzip -q -o "$inner_path" -d "$dest"
        else
            mkdir -p "$dest"
            cp -r "$tmp"/* "$dest/"
        fi
        # Many community packs wrap everything in a single freeform top-
        # level directory. Lift its contents up one level so callers can
        # rely on a stable path (e.g. `voicebanks/lilia/dsconfig.yaml`).
        local nested
        nested=$(find "$dest" -mindepth 1 -maxdepth 1 -type d)
        if [[ $(echo "$nested" | wc -l) == "1" ]] && \
           [[ ! -f "$dest/dsconfig.yaml" ]] && \
           [[ -f "$nested/dsconfig.yaml" ]]; then
            shopt -s dotglob
            mv "$nested"/* "$dest/"
            shopt -u dotglob
            rmdir "$nested"
        fi
        rm -rf "$tmp"
    fi
}

# LIEE Lilia (MM 2.8 / JubiLIEE 2025 1.1). Outer zip wraps an inner zip
# that holds the actual voicebank tree.
fetch_voicebank "lilia" \
    "https://github.com/julieraptor/DIFFSINGER-LIEE-Immortal-Idol/releases/download/MM2.8/Diffsinger.LIEE.Immortal.Idol.MM.2.8.JubiLIEE.2025.1.1.zip" \
    "d9fd90e61186f6397c148e3817832bd547d1f13b2ba7ba3d812efe020328fa12" \
    "Diffsinger LIEE Immortal Idol MM 2.8 (JubiLIEE 2025)/Diffsinger LIEE Immortal Idol (JubiLIEE 2025).zip"

# Gahata Meiji v160. Single-zip layout — the pack root extracts straight
# into the destination dir.
fetch_voicebank "meiji" \
    "https://github.com/lunaiproject/lunai_singers/releases/download/Gahata_Meiji_v160/Gahata_Meiji_v160.zip" \
    "2428c93da550e9e9f67e97664279bf748c797d9229e5194c31a93b47543d7afe"

echo ">> done. Contents:"
find "$MODELS" -maxdepth 5 -type f \( -name '*.yaml' -o -name '*.onnx' -o -name '*.json' -o -name '*.txt' \) | sed "s|$MODELS/|  |"
