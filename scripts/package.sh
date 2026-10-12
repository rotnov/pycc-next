#!/bin/sh
# Assembles the relocatable pycc release layout.
#
#   scripts/package.sh DEST [PROFILE_DIR]
#
# DEST becomes:
#   DEST/bin/pycc
#   DEST/lib/pycc/libpycc_rt.a     (the runtime every compiled program links)
#   DEST/README.md DEST/LICENSE DEST/CHANGELOG.md
#
# PROFILE_DIR is the Cargo profile directory holding `pycc` and
# `libpycc_rt.a` (default: ${CARGO_TARGET_DIR:-target}/release). `pycc`
# finds `lib/pycc/` relative to its own executable (src/install_layout.rs),
# so DEST can be moved anywhere. Used by .github/workflows/release.yml and
# tests/relocatable_release.rs, so both see the same layout.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
dest=${1:?usage: scripts/package.sh DEST [PROFILE_DIR]}
profile=${2:-${CARGO_TARGET_DIR:-$root/target}/release}

for f in "$profile/pycc" "$profile/libpycc_rt.a"; do
    [ -f "$f" ] || { echo "package.sh: missing $f (run cargo build --release)" >&2; exit 1; }
done

rm -rf "$dest"
mkdir -p "$dest/bin" "$dest/lib/pycc"
cp "$profile/pycc" "$dest/bin/pycc"
cp "$profile/libpycc_rt.a" "$dest/lib/pycc/libpycc_rt.a"
cp "$root/README.md" "$root/LICENSE" "$root/CHANGELOG.md" "$dest/"
# Debug info only: keeps symbols and every archive member intact. Apple's
# strip spells it -S and has no --strip-debug.
if command -v strip >/dev/null 2>&1; then
    case $(uname -s) in
        Darwin) strip -S "$dest/bin/pycc" "$dest/lib/pycc/libpycc_rt.a" ;;
        *) strip --strip-debug "$dest/bin/pycc" "$dest/lib/pycc/libpycc_rt.a" ;;
    esac
fi
