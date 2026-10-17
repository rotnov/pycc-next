#!/usr/bin/env bash
# Prepares a Windows (x86_64, MSVC) GitHub runner to build pycc: downloads
# LLVM's own prebuilt release (`clang+llvm-<ver>-x86_64-pc-windows-msvc`,
# from github.com/llvm/llvm-project, pinned by version and sha256), unpacks it
# and points llvm-sys at it. Used by ci.yml and release.yml so both build the
# same way. Run under Git Bash after the MSVC developer environment is set up
# (ilammy/msvc-dev-cmd), so `LIB` already names the MSVC and SDK libraries.
#
#   scripts/windows_llvm.sh     # appends LLVM_SYS_221_PREFIX and LIB to $GITHUB_ENV
#
# The archive is LLVM's official build: llvm-config.exe, clang.exe (pycc's
# linker driver on Windows, src/build_pipeline.rs), lld-link.exe and the
# static `LLVM*.lib` libraries llvm-sys links. No Windows package manager
# carries LLVM 22 with its static libraries and llvm-config.
#
# That build enables libxml2, used only by LLVMWindowsManifest (the
# `llvm-mt`/`lld-link /manifest` merger), so `llvm-config --system-libs`
# names a static libxml2 library the archive does not ship. pycc never calls
# into LLVMWindowsManifest, so no member of that library is ever pulled
# into the link and no libxml2 symbol is ever needed; the linker only has to
# be able to open a library of that name. An empty archive of that name is
# written instead of building libxml2: should a future pycc reference it, the
# link fails with the unresolved libxml2 symbols, never silently.
set -euo pipefail

version=22.1.8
sha256=d96c2cc1736f4eb7fa43cb9bbdf56d93551a9ae0a9aadb9c99c3c3b2b712a234
name=clang+llvm-$version-x86_64-pc-windows-msvc

work=$(cygpath -u "${RUNNER_TEMP:?}")/llvm
mkdir -p "$work"
cd "$work"
curl -sSfL --retry 3 -o llvm.tar.xz \
    "https://github.com/llvm/llvm-project/releases/download/llvmorg-$version/$name.tar.xz"
echo "$sha256  llvm.tar.xz" | sha256sum -c -
tar -xf llvm.tar.xz
rm llvm.tar.xz
prefix=$work/$name
"$prefix/bin/llvm-config.exe" --version
system_libs=$("$prefix/bin/llvm-config.exe" --system-libs --link-static)
echo "llvm-config --system-libs: $system_libs"

stubs=$work/stub-libs
mkdir -p "$stubs"
: > "$stubs/empty.c"
"$prefix/bin/clang.exe" -target x86_64-pc-windows-msvc -c "$(cygpath -w "$stubs/empty.c")" \
    -o "$(cygpath -w "$stubs/empty.obj")"
for lib in $system_libs; do
    case $lib in
        *xml*)
            # `-lxml2s`, `xml2s.lib` or a path on LLVM's build machine; how
            # llvm-sys spells it to the linker differs, so both the file name
            # and its `lib`-less form are provided.
            lib=${lib##*/}
            lib=${lib##*\\}
            lib=${lib#-l}
            case $lib in *.lib) ;; *) lib=$lib.lib ;; esac
            for stub in "$lib" "${lib#lib}"; do
                "$prefix/bin/llvm-lib.exe" "/out:$(cygpath -w "$stubs/$stub")" "$(cygpath -w "$stubs/empty.obj")"
                echo "empty stand-in for $stub (see the header of this script)"
            done
            ;;
    esac
done

echo "LLVM_SYS_221_PREFIX=$(cygpath -w "$prefix")" >> "$GITHUB_ENV"
echo "LIB=$(cygpath -w "$stubs");${LIB:-}" >> "$GITHUB_ENV"
