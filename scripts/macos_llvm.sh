#!/bin/sh
# Prepares a macOS (Apple silicon) GitHub runner to build pycc: installs
# Homebrew's keg-only llvm@22 and points llvm-sys at it. Used by ci.yml and
# release.yml so both build the same way.
#
#   scripts/macos_llvm.sh     # appends LLVM_SYS_221_PREFIX to $GITHUB_ENV
#
# LLVM's own libraries are linked statically; of its system libraries
# (`llvm-config --system-libs`: -lm -lz -lzstd -lxml2) only zstd is not part
# of macOS. Homebrew's zstd keg ships both libzstd.dylib and libzstd.a; the
# unversioned libzstd.dylib symlinks are what `ld -lzstd` would pick, so they
# are removed and the linker takes libzstd.a instead. The versioned
# libzstd.1.dylib stays for Homebrew's own tools (llvm-config itself). The
# resulting pycc depends only on /usr/lib and /System libraries, which
# scripts/package.sh checks.
set -eu

sw_vers
uname -m
brew install llvm@22
prefix=$(brew --prefix llvm@22)
"$prefix/bin/llvm-config" --version
"$prefix/bin/llvm-config" --system-libs --link-static
zstd=$(brew --prefix zstd)
[ -f "$zstd/lib/libzstd.a" ] || { echo "macos_llvm.sh: no $zstd/lib/libzstd.a" >&2; exit 1; }
rm -f "$zstd/lib/libzstd.dylib" "$(brew --prefix)/lib/libzstd.dylib"
# The macOS version Homebrew built LLVM's static libraries for (their minos).
vtool -show-build "$prefix/lib/libLLVMSupport.a" 2>/dev/null | grep -m1 -E 'minos|version' || true
echo "LLVM_SYS_221_PREFIX=$prefix" >> "$GITHUB_ENV"
