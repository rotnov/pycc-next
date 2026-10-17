#!/bin/sh
# Assembles the relocatable pycc release layout.
#
#   scripts/package.sh DEST [PROFILE_DIR]
#
# DEST becomes:
#   DEST/bin/pycc                  (bin/pycc.exe on Windows)
#   DEST/lib/pycc/libpycc_rt.a     (the runtime every compiled program links;
#                                  lib/pycc/pycc_rt.lib on Windows)
#   DEST/python/pycc_build-X.Y.Z-py3-none-any.whl
#                                  (the PEP 517 backend `pycc.build`, built from
#                                  python/ by itself with ${PYCC_PYTHON:-python3})
#   DEST/README.md DEST/LICENSE DEST/CHANGELOG.md
#
# PROFILE_DIR is the Cargo profile directory holding `pycc` and
# `libpycc_rt.a` (default: ${CARGO_TARGET_DIR:-target}/release). `pycc`
# finds `lib/pycc/` relative to its own executable (src/install_layout.rs),
# so DEST can be moved anywhere. Used by .github/workflows/release.yml and
# tests/relocatable_release.rs, so both see the same layout. On Windows it
# runs under Git Bash.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
dest=${1:?usage: scripts/package.sh DEST [PROFILE_DIR]}
profile=${2:-${CARGO_TARGET_DIR:-$root/target}/release}

# Windows (Git Bash): the MSVC target's executable and static-library names.
case $(uname -s) in
    MINGW* | MSYS* | CYGWIN*) exe=pycc.exe rt=pycc_rt.lib windows=1 ;;
    *) exe=pycc rt=libpycc_rt.a windows= ;;
esac

for f in "$profile/$exe" "$profile/$rt"; do
    [ -f "$f" ] || { echo "package.sh: missing $f (run cargo build --release)" >&2; exit 1; }
done

rm -rf "$dest"
mkdir -p "$dest/bin" "$dest/lib/pycc"
cp "$profile/$exe" "$dest/bin/$exe"
cp "$profile/$rt" "$dest/lib/pycc/$rt"
cp "$root/README.md" "$root/LICENSE" "$root/CHANGELOG.md" "$dest/"
mkdir "$dest/python"
wheel_dir=$(cd "$dest/python" && pwd)
# Native paths for a Windows Python (Git Bash's /d/a/... means nothing to it).
if [ -n "$windows" ]; then wheel_dir=$(cygpath -w "$wheel_dir"); fi
(cd "$root/python" && "${PYCC_PYTHON:-python3}" -I -B -c \
    'import sys; sys.path.insert(0, "."); from pycc import build; build.build_wheel(sys.argv[1])' \
    "$wheel_dir")
# Debug info only: keeps symbols and every archive member intact. Apple's
# strip spells it -S and has no --strip-debug. An MSVC build keeps its debug
# information in a separate .pdb, which is not shipped.
if [ -z "$windows" ] && command -v strip >/dev/null 2>&1; then
    case $(uname -s) in
        Darwin) strip -S "$dest/bin/pycc" "$dest/lib/pycc/libpycc_rt.a" ;;
        *) strip --strip-debug "$dest/bin/pycc" "$dest/lib/pycc/libpycc_rt.a" ;;
    esac
fi
# A macOS archive must run without Homebrew: pycc may only link libraries
# that ship with macOS (scripts/macos_llvm.sh links zstd statically).
if [ "$(uname -s)" = Darwin ]; then
    foreign=$(otool -L "$dest/bin/pycc" | tail -n +2 | grep -v -E '^[[:space:]]+(/usr/lib/|/System/Library/)' || true)
    if [ -n "$foreign" ]; then
        printf 'package.sh: bin/pycc links libraries macOS does not ship:\n%s\n' "$foreign" >&2
        exit 1
    fi
fi
