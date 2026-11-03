#!/bin/sh
# Prepares an Ubuntu 24.04 (noble) GitHub runner to build pycc: installs
# LLVM 22 into /usr/lib/llvm-22, the LLVM_SYS_221_PREFIX every Linux job
# uses. Used by ci.yml, bench.yml, fuzz.yml and release.yml so all of them
# install it the same way.
#
#   scripts/linux_llvm.sh
#   PYCC_LLVM_SOURCE=release scripts/linux_llvm.sh   # skip apt.llvm.org
#
# The first choice is apt.llvm.org's llvm-22-dev, libpolly-22-dev and
# clang-22. A runner's DNS sometimes fails to resolve apt.llvm.org for many
# minutes; `apt-get update` then only warns and leaves the source's lists
# out, so the install finds no llvm-22-dev and exits 100. The apt install is
# tried 3 times (waiting 10 and 20 seconds between attempts); if it still
# fails, LLVM's own prebuilt release is used instead: the
# `LLVM-<ver>-Linux-<X64|ARM64>.tar.xz` archive from the
# github.com/llvm/llvm-project release, pinned by version and sha256, served
# by GitHub rather than apt.llvm.org. It is the same LLVM 22 with the same
# static libraries, Polly included, so llvm-sys links it the same way. Only
# what the build reads is unpacked into /usr/lib/llvm-22 (llvm-config, clang
# -- pycc's linker driver for `--target` builds, src/build_pipeline.rs --
# its resource directory, the headers and the static libraries; about 0.8 GB
# of the archive's 12 GB). Its `llvm-config --system-libs` names zlib, zstd
# (as libzstd.a) and libxml2, which come from Ubuntu's own archive.
set -eu

version=22.1.8
prefix=/usr/lib/llvm-22

from_apt() {
    wget -qO- https://apt.llvm.org/llvm-snapshot.gpg.key | sudo tee /etc/apt/trusted.gpg.d/apt.llvm.org.asc >/dev/null
    echo 'deb https://apt.llvm.org/noble/ llvm-toolchain-noble-22 main' | sudo tee /etc/apt/sources.list.d/llvm22.list >/dev/null
    attempt=1
    delay=10
    until sudo apt-get update -q && sudo apt-get install -y -q llvm-22-dev libpolly-22-dev clang-22 libzstd-dev; do
        if [ "$attempt" -ge 3 ]; then
            echo "linux_llvm.sh: apt.llvm.org install failed after $attempt attempts" >&2
            return 1
        fi
        echo "linux_llvm.sh: attempt $attempt failed; retrying in ${delay}s" >&2
        sleep "$delay"
        attempt=$((attempt + 1))
        delay=$((delay * 2))
    done
}

from_release() {
    case $(uname -m) in
        x86_64) arch=X64; sha256=df0e1ecf16caf3489a272a5eea4eec9b0d82878f6477fa309504f918a0006384 ;;
        aarch64) arch=ARM64; sha256=805efad2bb91cb4967fa569e0881d10c0f69c04461cf671cccbae19f547acc34 ;;
        *) echo "linux_llvm.sh: no LLVM release archive for $(uname -m)" >&2; exit 1 ;;
    esac
    # apt.llvm.org's source, if added, would make every later `apt-get
    # update` fail the same way; Ubuntu's archive is enough from here on.
    sudo rm -f /etc/apt/sources.list.d/llvm22.list
    sudo apt-get update -q
    sudo apt-get install -y -q libzstd-dev libxml2-dev zlib1g-dev
    name=LLVM-$version-Linux-$arch
    archive=${RUNNER_TEMP:-/tmp}/$name.tar.xz
    curl -sSfL --retry 5 --retry-all-errors -o "$archive" \
        "https://github.com/llvm/llvm-project/releases/download/llvmorg-$version/$name.tar.xz"
    echo "$sha256  $archive" | sha256sum -c -
    sudo mkdir -p "$prefix"
    sudo tar -xJf "$archive" -C "$prefix" --strip-components=1 --wildcards \
        "$name/bin/llvm-config" "$name/bin/clang" "$name/bin/clang-$(echo "$version" | cut -d. -f1)" \
        "$name/include/llvm/*" "$name/include/llvm-c/*" \
        "$name/lib/libLLVM*.a" "$name/lib/libPolly*.a" "$name/lib/clang/*"
    rm -f "$archive"
}

if [ "${PYCC_LLVM_SOURCE:-apt}" = release ]; then
    echo "linux_llvm.sh: PYCC_LLVM_SOURCE=release; using LLVM's release archive" >&2
    from_release
elif ! from_apt; then
    echo "linux_llvm.sh: falling back to LLVM's release archive from github.com" >&2
    from_release
fi

"$prefix/bin/llvm-config" --version
system_libs=$("$prefix/bin/llvm-config" --system-libs --link-static)
echo "llvm-config --system-libs: $system_libs"
# A system library named by absolute path (libzstd.a) must exist here, or
# the link fails much later with a less direct error.
for lib in $system_libs; do
    case $lib in
        /*) [ -f "$lib" ] || { echo "linux_llvm.sh: $lib (from llvm-config --system-libs) is missing" >&2; exit 1; } ;;
    esac
done
"$prefix/bin/clang" --version | head -n 1
