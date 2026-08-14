#!/bin/bash
# Build all CLAP plugins and copy them to target/bundled/*.clap
#
# The plugin list is DERIVED from plugins/*/ rather than written out
# here (ba todo #1250). It used to be a literal list, which meant a new
# plugin crate had to be registered in three places that nothing checked
# against each other — the workspace Cargo.toml, this script, and a
# per-plugin bundler.toml. ba todo #1073 shipped a plugin that was
# missing from this script through two rounds of review because of it.
#
# The bundler.toml files are gone: nothing ever read them (they were
# nih-plug-style stubs, and this project does not use nih-plug), and
# only 6 of 11 plugins had one, so they advertised a registration step
# that did not exist.
set -euo pipefail

cd "$(dirname "$0")/.."

mkdir -p target/bundled

# Every directory under plugins/ that is a crate is a CLAP plugin.
plugins=()
for dir in plugins/*/; do
    [ -f "${dir}Cargo.toml" ] || continue
    plugins+=("$(basename "$dir")")
done

if [ ${#plugins[@]} -eq 0 ]; then
    echo "error: no plugin crates found under plugins/" >&2
    exit 1
fi

# Drift check: a plugin directory that is not a workspace member will
# not build, and bundling would silently skip it. Fail loudly instead —
# this is the check that would have caught #1073.
missing=()
for plugin in "${plugins[@]}"; do
    grep -q "\"plugins/${plugin}\"" Cargo.toml || missing+=("$plugin")
done
if [ ${#missing[@]} -gt 0 ]; then
    echo "error: plugin crates missing from the workspace members in Cargo.toml:" >&2
    printf '  plugins/%s\n' "${missing[@]}" >&2
    exit 1
fi

for plugin in "${plugins[@]}"; do
    cargo build --release -p "$plugin"
    so_name="lib${plugin//-/_}.so"
    cp "target/release/$so_name" "target/bundled/${plugin}.clap"
    echo "  Bundled ${plugin}.clap"
done

echo "Done. ${#plugins[@]} plugins in target/bundled/"
