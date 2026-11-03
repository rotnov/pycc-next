#!/bin/sh
# Prepares an Ubuntu 24.04 (noble) GitHub runner to build pycc: installs
# LLVM 22 from apt.llvm.org. Used by ci.yml, bench.yml, fuzz.yml and
# release.yml so all of them install it the same way.
#
#   scripts/linux_llvm.sh
#
# A runner's DNS sometimes fails to resolve apt.llvm.org for a while.
# `apt-get update` then only warns and leaves the source's lists out, so the
# install step finds no llvm-22-dev and exits 100 before any test runs.
# Both steps are therefore retried together: up to 5 attempts, waiting
# 10, 20, 40 and 80 seconds between them.
set -eu

wget -qO- https://apt.llvm.org/llvm-snapshot.gpg.key | sudo tee /etc/apt/trusted.gpg.d/apt.llvm.org.asc >/dev/null
echo 'deb https://apt.llvm.org/noble/ llvm-toolchain-noble-22 main' | sudo tee /etc/apt/sources.list.d/llvm22.list

attempt=1
delay=10
until sudo apt-get update -q && sudo apt-get install -y -q llvm-22-dev libpolly-22-dev clang-22 libzstd-dev; do
    if [ "$attempt" -ge 5 ]; then
        echo "linux_llvm.sh: LLVM 22 could not be installed after $attempt attempts" >&2
        exit 1
    fi
    echo "linux_llvm.sh: attempt $attempt failed; retrying in ${delay}s" >&2
    sleep "$delay"
    attempt=$((attempt + 1))
    delay=$((delay * 2))
done
