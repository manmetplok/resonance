#!/bin/bash
# Build all CLAP plugins and copy them to target/bundled/*.clap
#
# The plugin list is DERIVED from the plugins/*/ directories rather than written out
# here (ba todo #1250). It used to be a literal list, which meant a new
# plugin crate had to be registered in three places that nothing checked
# against each other — the workspace Cargo.toml, this script, and a
# per-plugin bundler.toml.
#
# What fixes that class of bug is the DERIVATION below, not the drift
# checks: ba todo #1073's plugin *was* a workspace member and *did* ship
# a bundler.toml, so the literal list here was the only thing wrong. The
# checks guard the cases derivation alone cannot see. Do not swap the
# derivation back for a literal list and keep the checks.
#
# The bundler.toml files are gone: nothing ever read them (nih-plug-style
# stubs, and this project does not use nih-plug), and only 6 of 11
# plugins had one, so they advertised a registration step that did not
# exist.
set -euo pipefail

cd "$(dirname "$0")/.."

# Workspace members under plugins/, from cargo itself rather than a
# hand-rolled TOML parse. A sed/grep version mis-read four realistic
# inputs: a comment inside the array, a `members` key under a different
# table, a `members = ["plugins/*"]` glob, and an indented key. cargo
# resolves globs and comments for us and is the same source of truth the
# build uses.
members_under_plugins() {
    cargo metadata --format-version 1 --no-deps --offline 2>/dev/null |
        python3 -c '
import json, sys, os
md = json.load(sys.stdin)
root = md["workspace_root"]
for p in md["packages"]:
    rel = os.path.relpath(os.path.dirname(p["manifest_path"]), root)
    if rel.startswith("plugins" + os.sep):
        print(rel.split(os.sep, 1)[1])
'
}

# Every directory under plugins/ that is a crate.
dirs_under_plugins() {
    for dir in plugins/*/; do
        [ -f "${dir}Cargo.toml" ] || continue
        basename "$dir"
    done
}

mapfile -t plugins < <(dirs_under_plugins | sort)
mapfile -t members < <(members_under_plugins | sort)

if [ ${#plugins[@]} -eq 0 ]; then
    echo "error: no plugin crates found under plugins/" >&2
    exit 1
fi

# Cross-check BOTH directions. dirs->members catches a crate added
# without registering it; members->dirs catches a registered plugin this
# glob cannot see (e.g. plugins/experimental/foo/), which would
# otherwise be silently left out of the bundle — the #1073 failure mode,
# one directory level deeper.
if [ ${#members[@]} -eq 0 ]; then
    echo "error: could not read workspace members from cargo metadata." >&2
    echo "       Refusing to bundle rather than skipping the drift check." >&2
    exit 1
fi

# No `|| true` here: comm exits non-zero on unsorted input, and swallowing
# that would leave the variables holding truncated output while both
# emptiness tests pass — silently bundling an unvalidated list.
missing_from_members=$(comm -23 <(printf '%s\n' "${plugins[@]}") <(printf '%s\n' "${members[@]}"))
missing_from_dirs=$(comm -13 <(printf '%s\n' "${plugins[@]}") <(printf '%s\n' "${members[@]}"))
if [ -n "$missing_from_members" ]; then
    echo "error: plugin crates not in the workspace members array:" >&2
    printf '  plugins/%s\n' "$missing_from_members" >&2
    exit 1
fi
if [ -n "$missing_from_dirs" ]; then
    echo "error: workspace members under plugins/ that this script cannot see" >&2
    echo "       (nested deeper than plugins/<name>/ ?):" >&2
    printf '  plugins/%s\n' "$missing_from_dirs" >&2
    exit 1
fi

# Every plugin must actually produce a loadable .clap, i.e. be a cdylib.
# A shared rlib-only helper crate parked under plugins/ would otherwise
# pass both checks, build fine, and fail at the copy — after earlier
# plugins had already been written into the bundle.
for plugin in "${plugins[@]}"; do
    if ! grep -qE '^[[:space:]]*crate-type[[:space:]]*=.*cdylib' "plugins/${plugin}/Cargo.toml"; then
        echo "error: plugins/${plugin} is not a cdylib, so it cannot be a CLAP plugin." >&2
        echo "       Move it out of plugins/ if it is a shared library." >&2
        exit 1
    fi
done

# Build everything BEFORE publishing anything, in ONE cargo invocation:
# a failure then leaves the previous bundle intact instead of a
# half-updated mix of old and new.
build_args=()
for plugin in "${plugins[@]}"; do
    build_args+=(-p "$plugin")
done
cargo build --release "${build_args[@]}"

# Stage into a scratch directory and swap at the very end. Deleting
# target/bundled up front and copying into it means any failed cp leaves
# the user with an EMPTY bundle -- worse than the stale one they had.
# Swapping also drops a renamed or deleted plugin's old .clap, which the
# scanner would otherwise load alongside the new one (it dedups by
# nothing, so the catalog listed the plugin twice).
staging=target/bundled.new
rm -rf "$staging"
mkdir -p "$staging"

for plugin in "${plugins[@]}"; do
    so_name="lib${plugin//-/_}.so"
    cp "target/release/$so_name" "$staging/${plugin}.clap"
    echo "  Bundled ${plugin}.clap"
done

rm -rf target/bundled
mv "$staging" target/bundled

echo "Done. ${#plugins[@]} plugins in target/bundled/"
