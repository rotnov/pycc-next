#!/bin/sh
# Smoke-tests an unpacked release archive the way a user would run it.
#
#   scripts/smoke.sh PKG_DIR
#
# Copies PKG_DIR into a fresh temp directory, then -- with a cleared
# environment (PATH and an empty HOME only, no CARGO_* variables) -- builds
# and runs a native program, an --ext module and a --non-strict program, and
# compares each with CPython (`${PYCC_PYTHON:-python3}`). The release
# workflow runs it on the tarball before publishing; tests/relocatable_release.rs
# checks the same things from `cargo test`.
set -eu

src=$(cd "${1:?usage: scripts/smoke.sh PKG_DIR}" && pwd)
python=${PYCC_PYTHON:-python3}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp -R "$src" "$work/pkg"
mkdir "$work/home" "$work/native" "$work/ext" "$work/ext_src" "$work/ns"
pycc="$work/pkg/bin/pycc"
clean() { env -i PATH="$PATH" HOME="$work/home" ${PYCC_PYTHON:+PYCC_PYTHON="$PYCC_PYTHON"} "$@"; }
same() { [ "$1" = "$2" ] || { printf 'MISMATCH %s\n--- pycc\n%s\n--- cpython\n%s\n' "$3" "$1" "$2" >&2; exit 1; }; }

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

echo "smoke: ok"
