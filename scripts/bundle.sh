#!/bin/bash
# Build all CLAP plugins and copy them to target/bundled/*.clap
#
# The plugin list is DERIVED from the workspace rather than written out
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

# Workspace members living under plugins/, read from the members array
# specifically. Grepping the whole file would also match a commented-out
# line or a path dependency in [workspace.dependencies], which silently
# disarms the cross-check below.
members_under_plugins() {
    # Print from the `members =` line up to and including the line that
    # closes the array. NOT a `/start/,/end/` range: sed looks for the
    # end pattern only from the line AFTER the start, so on this repo's
    # single-line members array the range ran on to the next `]` — the
    # `[workspace.dependencies]` header — sweeping in any commented-out
    # or unrelated `"plugins/..."` text between, which is exactly the
    # disarming this check exists to prevent.
    sed -n '/^members[[:space:]]*=/{:a;p;/\]/q;n;ba}' Cargo.toml |
        grep -o '"plugins/[^"]*"' |
        tr -d '"' |
        sed 's|^plugins/||'
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
missing_from_members=$(comm -23 <(printf '%s\n' "${plugins[@]}") <(printf '%s\n' "${members[@]}") || true)
missing_from_dirs=$(comm -13 <(printf '%s\n' "${plugins[@]}") <(printf '%s\n' "${members[@]}") || true)
if [ -n "$missing_from_members" ]; then
    echo "error: plugin crates not in the workspace members array:" >&2
    printf '  plugins/%s\n' $missing_from_members >&2
    exit 1
fi
if [ -n "$missing_from_dirs" ]; then
    echo "error: workspace members under plugins/ that this script cannot see" >&2
    echo "       (nested deeper than plugins/<name>/ ?):" >&2
    printf '  plugins/%s\n' $missing_from_dirs >&2
    exit 1
fi

# Every plugin must actually produce a loadable .clap, i.e. be a cdylib.
# A shared rlib-only helper crate parked under plugins/ would otherwise
# pass both checks, build fine, and fail at the copy — after earlier
# plugins had already been written into the bundle.
for plugin in "${plugins[@]}"; do
    if ! grep -q 'cdylib' "plugins/${plugin}/Cargo.toml"; then
        echo "error: plugins/${plugin} is not a cdylib, so it cannot be a CLAP plugin." >&2
        echo "       Move it out of plugins/ if it is a shared library." >&2
        exit 1
    fi
done

# Build everything BEFORE publishing anything, so a failure leaves the
# previous bundle intact instead of a half-updated mix of old and new.
for plugin in "${plugins[@]}"; do
    cargo build --release -p "$plugin"
done

# Replace the directory wholesale: a renamed or deleted plugin used to
# leave its old .clap behind, and the scanner loads every *.clap it finds
# with no dedup by plugin id, so the catalog listed the stale build
# alongside the new one.
rm -rf target/bundled
mkdir -p target/bundled

for plugin in "${plugins[@]}"; do
    so_name="lib${plugin//-/_}.so"
    cp "target/release/$so_name" "target/bundled/${plugin}.clap"
    echo "  Bundled ${plugin}.clap"
done

echo "Done. ${#plugins[@]} plugins in target/bundled/"
