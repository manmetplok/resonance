#!/bin/bash
# Build all CLAP plugins and copy them to target/bundled/*.clap
set -e

mkdir -p target/bundled

for plugin in resonance-drums resonance-amp resonance-eq resonance-ir resonance-reverb resonance-wavetable; do
    cargo build --release -p "$plugin"
    so_name="lib${plugin//-/_}.so"
    cp "target/release/$so_name" "target/bundled/${plugin}.clap"
    echo "  Bundled ${plugin}.clap"
done

echo "Done. Plugins in target/bundled/"
