#!/bin/sh
# Smoke-tests an unpacked release archive the way a user would run it.
#
#   scripts/smoke.sh PKG_DIR
#
# Copies PKG_DIR into a fresh temp directory, then -- with a cleared
# environment (PATH and an empty HOME only, no CARGO_* variables) -- builds
# and runs a native program, an --ext module and a --non-strict program, and
# compares each with CPython (`${PYCC_PYTHON:-python3}`). Last, it installs
# the archive's PEP 517 backend (python/pycc_build-*.whl) into a fresh venv,
# builds a small project's wheel with `pip wheel` (no network, the archive's
# bin/ on PATH), installs it and runs its compiled modules against CPython. The release
# workflow runs it on the tarball before publishing; tests/relocatable_release.rs
# checks the same things from `cargo test`. On Windows it runs under Git Bash
# with the MSVC developer environment (the linker driver needs it).
set -eu

case $(uname -s) in
    MINGW* | MSYS* | CYGWIN*) windows=1 exe=.exe ;;
    *) windows= exe= ;;
esac

src=$(cd "${1:?usage: scripts/smoke.sh PKG_DIR}" && pwd)
python=${PYCC_PYTHON:-python3}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp -R "$src" "$work/pkg"
mkdir "$work/home" "$work/native" "$work/ext" "$work/ext_src" "$work/ns"
pycc="$work/pkg/bin/pycc$exe"
# A Windows process needs a few system variables to start, and the linker
# driver the MSVC developer environment ("Developer Command Prompt"); those
# are kept, nothing else.
keep="SYSTEMROOT WINDIR TEMP TMP PATHEXT COMSPEC PROGRAMDATA LIB LIBPATH INCLUDE
    VCToolsInstallDir VCINSTALLDIR VSINSTALLDIR VisualStudioVersion WindowsSdkDir
    WindowsSDKVersion WindowsSdkBinPath UniversalCRTSdkDir UCRTVersion"
clean() {
    if [ -n "$windows" ]; then
        n=$#
        for v in $keep; do
            eval "x=\${$v-}"
            if [ -n "$x" ]; then set -- "$@" "$v=$x"; fi
        done
        set -- "$@" USERPROFILE="$(cygpath -w "$work/home")"
        # The command's n words move behind the variables.
        i=0
        while [ "$i" -lt "$n" ]; do a=$1; shift; set -- "$@" "$a"; i=$((i + 1)); done
    fi
    env -i PATH="$PATH" HOME="$work/home" ${PYCC_PYTHON:+PYCC_PYTHON="$PYCC_PYTHON"} "$@"
}
# Line content: both write \r\n on Windows, which $(...) keeps but tr drops
# (tests/text_stdio.rs compares the bytes).
same() {
    a=$(printf '%s' "$1" | tr -d '\r')
    b=$(printf '%s' "$2" | tr -d '\r')
    [ "$a" = "$b" ] || { printf 'MISMATCH %s\n--- pycc\n%s\n--- cpython\n%s\n' "$3" "$1" "$2" >&2; exit 1; }
}

clean "$pycc" version

cat > "$work/native/fib.py" <<'EOF'
def fib(n: int) -> int:
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)


print(fib(27))
EOF
cd "$work/native"
clean "$pycc" build fib.py -o fib --release
same "$(./fib)" "$("$python" fib.py)" native

cat > "$work/ext_src/m.py" <<'EOF'
def add(a: int, b: int) -> int:
    return a + b


def greet(name: str) -> str:
    return 'hello ' + name
EOF
cp "$work/ext_src/m.py" "$work/ext/m.py"
cd "$work/ext"
clean "$pycc" build m.py -o m --ext --release
rm m.py
driver="import m; print(m.add(2, 40), m.greet('pycc'))"
got=$(clean "$python" -c "$driver")
cd "$work/ext_src"
same "$got" "$(clean "$python" -c "$driver")" ext

cat > "$work/ns/prog.py" <<'EOF'
def squares(n):
    for i in range(n):
        yield i * i


inc = lambda x: x + 1
print(sum(squares(10)), inc(41))
EOF
cd "$work/ns"
clean "$pycc" build prog.py -o prog --non-strict
same "$(clean ./prog)" "$("$python" prog.py)" non-strict

# The PEP 517 backend: `pip wheel` of a project selecting `pycc.build`.
mkdir -p "$work/proj/src/hello" "$work/wheels"
cat > "$work/proj/pyproject.toml" <<'EOF2'
[build-system]
requires = ["pycc-build"]
build-backend = "pycc.build"

[project]
name = "hello-pycc"
version = "1.0"
requires-python = ">=3.13"

[tool.pycc]
packages = ["hello"]
pure-python = ["hello.__init__"]
EOF2
printf '' > "$work/proj/src/hello/__init__.py"
cp "$work/ext_src/m.py" "$work/proj/src/hello/fast.py"
clean "$python" -m venv "$work/venv"
if [ -n "$windows" ]; then vpy="$work/venv/Scripts/python.exe"; else vpy="$work/venv/bin/python"; fi
clean "$vpy" -I -m pip install -q --no-index --no-deps "$work"/pkg/python/pycc_build-*.whl
cd "$work"
PATH="$work/pkg/bin:$PATH" clean \
    "$vpy" -I -m pip wheel -q --no-build-isolation --no-index --no-deps -w "$work/wheels" "$work/proj"
for whl in "$work"/wheels/*.whl; do
    case $whl in
        *-cp313-abi3-*.whl) ;;
        *) echo "smoke: unexpected wheel $whl" >&2; exit 1 ;;
    esac
    clean "$vpy" -I -m pip install -q --no-index --no-deps "$whl"
done
if [ -n "$windows" ]; then suffix=.pyd; else suffix=.abi3.so; fi
driver="import hello.fast as m; print(m.__file__.endswith('$suffix'), m.add(2, 40), m.greet('pycc'))"
got=$(cd "$work/home" && clean "$vpy" -I -c "$driver")
want=$(cd "$work/proj/src" && clean "$python" -c "$driver")
same "$got" "True${want#False}" pep517

echo "smoke: ok"
