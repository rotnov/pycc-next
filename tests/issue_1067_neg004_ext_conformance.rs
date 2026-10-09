//! NEG-004 (Part E of #1038, #1067): the CPython-driven conformance
//! harness for D-244's host call boundary.
//!
//! D-244 rule 7 says a non-conforming call raises `TypeError` at the
//! wrapper before the compiled body runs, "with no argument mutated and no
//! partial effect left behind". This file is where that sentence is
//! checked against a real interpreter: one `--ext` artifact, then every
//! refusal shape the boundary can produce plus the three properties a
//! single refusal cannot show on its own -- that nothing was left behind,
//! that the export is still correct afterwards, and that a call with two
//! non-conforming arguments names the first one.
//!
//! There is no oracle here, and that is the point. Rule 4's pinned CPython
//! 3.14.7 oracle scopes to *conforming* calls -- what a compiled function
//! computes. A refusal is pycc's own contract at the boundary, so the
//! interpreter in this file is a **loader**, not an oracle: `PYCC_PYTHON`
//! (default `python3`) only has to be a CPython 3.13+ that can import the
//! artifact. Both supported versions must agree, which is why the harness
//! is expected to pass unchanged under `PYCC_PYTHON=python3.14`.
//!
//! `PYCC_PYTHON` is read twice for two different purposes: `src/ext_build.rs`
//! probes it at build time for the development headers, and every `ext`
//! test reads it at run time as the loader. Building against one version
//! and loading on another is `PYCC_PYTHON_INCLUDE`'s job, not this one's.
//!
//! Assertion convention: text pycc authors is asserted **exactly**; text
//! CPython authors is asserted by exception *type* plus a substring both
//! supported versions share. Exactly one shape below is CPython-authored
//! -- the lone-surrogate `UnicodeEncodeError` from
//! `PyUnicode_AsUTF8AndSize` -- and it carries a comment saying so. (The
//! keyword-argument shape was the second until #1461 made the generated
//! binder author it.)
//!
//! `#[ignore]`d for the reason every `ext` test is: it asks an installed
//! CPython 3.13+ to import a built artifact, which is a property of the
//! machine. CI runs it twice, on both supported loaders: on every Tier-1
//! `native-build-test` leg against the 3.13 stable-ABI floor, and in
//! `build-test-coverage`'s own `cargo test --workspace -- --include-ignored`
//! step, which runs after that job installs CPython 3.14.7. It is measured
//! in neither, because only that job's earlier `llvm-cov` step is, and that
//! one runs without `--include-ignored`. The completeness guard that *is*
//! inside the coverage denominator lives in
//! `src/ext_build_tests/refusal_completeness.rs`.
//!
//! One `#[test]`, deliberately: each one costs a full `pycc build` on every
//! Tier-1 leg, so the shapes share a single artifact and a single script.

use pycc_scratch::ScratchDir;
use std::process::Command;

/// The probe module. Every export exists to be called *wrongly*; the
/// conforming calls are the re-entrancy half of the same statement.
///
/// `_log` is seeded with one element because an empty list literal has no
/// inferable element type here (`T0003`), so the counter's baseline is
/// `size() == 1` rather than 0. It is mutated in place rather than
/// rebound: a module-level rebinding needs a `global` statement, which is
/// not compiled today, and mutation is what the no-partial-effect property
/// needs anyway.
const FIXTURE: &str = r#"
_log: list[int] = [0]


def take_int(x: int) -> int:
    return x + 1


def two(a: int, b: int) -> int:
    return a + b


def take_tuple(t: tuple[int, int]) -> int:
    return t[0] + t[1]


def take_str(s: str) -> str:
    return s


def take_float(x: float) -> float:
    return x + 1.0


def take_bool(b: bool) -> bool:
    return b


def take_tuple_fb(t: tuple[float, bool]) -> float:
    return t[0]


def join(a: str, b: str) -> str:
    return a + b


def bump(x: int) -> int:
    _log.append(x)
    return len(_log)


def bump2(x: int, y: int) -> int:
    _log.append(x)
    return y


def size() -> int:
    return len(_log)


def take_view(v: memoryview) -> int:
    return 7


def view_then_int(v: memoryview, n: int) -> int:
    return n
"#;

/// The host-side script. It runs under the loader, not under an oracle, so
/// every assertion here is about pycc's own boundary contract.
const SCRIPT: &str = r#"
import pycc_1067_mod as m


class SubInt(int):
    pass


class SubStr(str):
    pass


class SubTuple(tuple):
    pass


class SubFloat(float):
    pass


def refuse(fn, args, kwargs, exc, text, exact, good, want):
    """Refuse `fn(*args, **kwargs)`, then prove `fn(*good)` still works."""
    try:
        fn(*args, **kwargs)
    except exc as e:
        got = str(e)
        if exact:
            assert got == text, got
        else:
            assert text in got, got
    else:
        raise AssertionError('the call was not refused')
    # Re-entrancy. A rule-7 refusal returns before the compiled body runs,
    # so the very next conforming call to the same export must return its
    # correct value -- not merely not crash.
    assert fn(*good) == want, fn


# Shapes 1-3: a wrong type at an `int` parameter. pycc-authored text.
refuse(m.take_int, ('x',), {}, TypeError,
       "take_int() argument 1: 'str' object cannot be interpreted as an integer",
       True, (1,), 2)
refuse(m.take_int, (1.5,), {}, TypeError,
       "take_int() argument 1: 'float' object cannot be interpreted as an integer",
       True, (1,), 2)
refuse(m.take_int, (None,), {}, TypeError,
       "take_int() argument 1: 'NoneType' object cannot be interpreted as an integer",
       True, (1,), 2)

# Shapes 4-5 and 17: arity. Checked before any argument is read, because
# reading `args[1]` of a one-argument call is out of bounds, not a
# `TypeError`. pycc-authored text, including the singular/plural split.
refuse(m.two, (1,), {}, TypeError,
       'two() takes exactly 2 arguments (1 given)', True, (1, 2), 3)
refuse(m.two, (1, 2, 3), {}, TypeError,
       'two() takes exactly 2 arguments (3 given)', True, (1, 2), 3)
refuse(m.take_int, (), {}, TypeError,
       'take_int() takes exactly 1 argument (0 given)', True, (1,), 2)

# Shape 6: keyword arguments. Since #1461 a keyword naming a parameter is
# bound as CPython binds it (`m.two(a=1, b=2)` returns 3), so the refused
# shape is a keyword naming no parameter. pycc-authored text, worded as
# CPython's own.
refuse(m.two, (1,), {'c': 2}, TypeError,
       "two() got an unexpected keyword argument 'c'", True, (1, 2), 3)
assert m.two(b=2, a=1) == 3

# Shapes 7-9: `tuple`. Length, container type, and element type are three
# distinct refusals, and the element one is 1-based in its own message.
# pycc-authored text.
refuse(m.take_tuple, ((1, 2, 3),), {}, TypeError,
       'take_tuple() argument 1: expected a tuple of length 2, got 3',
       True, ((1, 2),), 3)
refuse(m.take_tuple, ([1, 2],), {}, TypeError,
       "take_tuple() argument 1: 'list' object cannot be interpreted as a tuple",
       True, ((1, 2),), 3)
refuse(m.take_tuple, ((1, 'x'),), {}, TypeError,
       "take_tuple() argument 1, element 2: 'str' object cannot be interpreted as an integer",
       True, ((1, 2),), 3)

# Shapes 10-11: a wrong type at a `str` parameter. pycc-authored text.
refuse(m.take_str, (1,), {}, TypeError,
       "take_str() argument 1: 'int' object cannot be interpreted as a str",
       True, ('ok',), 'ok')
refuse(m.take_str, (None,), {}, TypeError,
       "take_str() argument 1: 'NoneType' object cannot be interpreted as a str",
       True, ('ok',), 'ok')

# Shape 12: a lone surrogate. CPython-authored -- the message comes out of
# `PyUnicode_AsUTF8AndSize`, and it is a `UnicodeEncodeError`, not a
# `TypeError`, so the shared substring is what is asserted.
refuse(m.take_str, ('\ud800',), {}, UnicodeEncodeError,
       "'utf-8' codec can't encode character", False, ('ok',), 'ok')

# Shapes 13-16: conformance is by protocol, not by exact type. `bool` is an
# `int`, and a subclass of an admitted type is admitted -- but what comes
# back is the base type, never the subclass.
assert m.take_int(True) == 2
assert m.take_int(SubInt(4)) == 5
back = m.take_str(SubStr('ab'))
assert back == 'ab', back
assert type(back) is str, type(back)
assert m.take_tuple(SubTuple((3, 4))) == 7

# No partial effect: a refused call leaves the module exactly as it found
# it. `bump` is the one export with an observable side effect, and `size`
# reads it without touching it.
before = m.size()
try:
    m.bump('x')
except TypeError:
    pass
else:
    raise AssertionError('bump was not refused')
assert m.size() == before, m.size()
assert m.bump(9) == before + 1
assert m.size() == before + 1, m.size()

# ...and the same holds when the non-conforming argument is not the first
# one. `bump2` conforms at argument 1 and is refused at argument 2, so a
# call that reached the compiled body would append to `_log` before
# raising: the counter is what says it never got there.
before = m.size()
refuse(m.bump2, (1, 'x'), {}, TypeError,
       "bump2() argument 2: 'str' object cannot be interpreted as an integer",
       True, (1, 7), 7)
assert m.size() == before + 1, m.size()

# Shapes 18-19: a wrong type at a `float` parameter. `PyFloat_Check` with
# no fallback converter, so an `int` is refused rather than widened.
# pycc-authored text.
refuse(m.take_float, (1,), {}, TypeError,
       "take_float() argument 1: 'int' object cannot be interpreted as a float",
       True, (1.5,), 2.5)
refuse(m.take_float, (None,), {}, TypeError,
       "take_float() argument 1: 'NoneType' object cannot be interpreted as a float",
       True, (1.5,), 2.5)

# Shape 20: a wrong type at a `bool` parameter. `PyBool_Check`, never
# `PyObject_IsTrue`: rule 7's boundary is closed, so `1` is not `True`.
# pycc-authored text.
refuse(m.take_bool, (1,), {}, TypeError,
       "take_bool() argument 1: 'int' object cannot be interpreted as a bool",
       True, (True,), True)

# Shapes 21-22: the `_at` element helpers for the other two D-116 element
# types, each naming its own 1-based element index. pycc-authored text.
refuse(m.take_tuple_fb, ((1, True),), {}, TypeError,
       "take_tuple_fb() argument 1, element 1: 'int' object cannot be "
       "interpreted as a float",
       True, ((1.5, True),), 1.5)
refuse(m.take_tuple_fb, ((1.5, 1),), {}, TypeError,
       "take_tuple_fb() argument 1, element 2: 'int' object cannot be "
       "interpreted as a bool",
       True, ((1.5, True),), 1.5)

# Shapes 23-24: `float` conformance is by protocol too, at a scalar and at
# an element position -- `PyFloat_Check` is subtype-aware -- and what comes
# back is the base type. `bool` has no counterpart: it cannot be subclassed.
back = m.take_float(SubFloat(1.5))
assert back == 2.5, back
assert type(back) is float, type(back)
assert m.take_tuple_fb((SubFloat(1.5), True)) == 1.5

# Shapes 25-28, plus 25b and 25c: Part 1 of #1027's buffer carrier, walked
# in the order `pycc_ext_unpack_memoryview` applies its arms. Shapes 25, 26,
# 27 and 28 are one refusal per arm; #1129's widening of arm 1 adds two more
# that are not one-per-arm -- 25b is a second operand answered by the format
# arm, and 25c is an acceptance rather than a refusal, the positive half of
# that widening. pycc-authored text throughout except shape 26, whose message
# comes out of CPython's own `PyObject_GetBuffer` and is propagated verbatim.
#
# `bytearray(8)` is the conforming witness's backing store rather than a
# `bytes`: rule 7 admits a read-only buffer just as it admits a writable
# one for a parameter this module only reads (the request is
# `PyBUF_C_CONTIGUOUS | PyBUF_FORMAT`; Part 1 of #1142 adds
# `PyBUF_WRITABLE` only for a parameter whose own body stores into it, and
# no export here does), and a `bytearray` is what the release probe below
# needs.
good_view = memoryview(bytearray(8)).cast('d')

# 25: exports no buffer at all. #1129 widened this arm from
# `PyMemoryView_Check` to `PyObject_CheckBuffer`, so what it refuses is now
# the objects that export nothing -- a `list`, an `int`, `None` -- and no
# longer the buffer exporters that are merely not `memoryview`s. Those are
# answered by the arms below on the properties of the buffer they export,
# which is shape 25b.
refuse(m.take_view, ([1.0, 2.0],), {}, TypeError,
       "take_view() argument 1: 'list' object does not export a buffer",
       True, (good_view,), 7)

# 25b: a `bytes` is a buffer exporter, so it passes the widened arm 1 and
# is refused by the *format* arm instead -- the same answer shape 28 gets
# for a `memoryview` over the same bytes. The pair is the statement that
# arm 1 is now about the protocol and arm 4 about the element type.
refuse(m.take_view, (b'abcdefgh',), {}, TypeError,
       "take_view() argument 1: a buffer of format 'B' is not supported -- "
       "only format 'd' (a contiguous float64 buffer) is",
       True, (good_view,), 7)

# 25c: and an `array.array('d')` -- a conforming buffer that is not a
# `memoryview` -- is now *accepted*, which is the positive half of the same
# widening and the one arm no refusal shape can state.
import array
assert m.take_view(array.array('d', [1.5, 2.5])) == 7

# 26: a `memoryview` that is not C-contiguous. CPython-authored -- the
# message is `PyObject_GetBuffer`'s own, propagated without rewriting.
# Thirty-two bytes rather than sixteen: `[::2]` over a two-element buffer
# yields one element, which *is* contiguous, so the smaller store would
# assert nothing.
strided = memoryview(bytearray(32)).cast('d')[::2]
assert not strided.c_contiguous
refuse(m.take_view, (strided,), {}, (TypeError, BufferError),
       'contiguous', False, (good_view,), 7)

# 27: the right format and the wrong rank.
refuse(m.take_view, (memoryview(bytearray(16)).cast('d', (2, 1)),), {}, TypeError,
       'take_view() argument 1: a buffer with ndim 2 is not supported yet -- '
       'only a one-dimensional buffer is',
       True, (good_view,), 7)

# 28: the right rank and the wrong format.
refuse(m.take_view, (memoryview(bytearray(8)),), {}, TypeError,
       "take_view() argument 1: a buffer of format 'B' is not supported -- "
       "only format 'd' (a contiguous float64 buffer) is",
       True, (good_view,), 7)

# 28b: a multi-character format. A `ctypes` array of `c_double` exports
# `'<d'` -- the right itemsize and a format that is still not `'d'` -- so
# it exercises the one arm where the message must name more than a single
# character (D-244 statement (e): the `TypeError` names what it saw).
import ctypes
wide = memoryview((ctypes.c_double * 2)())
assert wide.format == '<d' and wide.itemsize == 8
refuse(m.take_view, (wide,), {}, TypeError,
       "take_view() argument 1: a buffer of format '<d' is not supported -- "
       "only format 'd' (a contiguous float64 buffer) is",
       True, (good_view,), 7)

# The release property, which no refusal shape can state on its own: the
# wrapper owns the `Py_buffer` for exactly the duration of the call, on the
# success path as well as on every bail. `bytearray.append` raises
# `BufferError` while any export is outstanding, so a successful append
# after each call is the proof that none was leaked.
store = bytearray(8)
probe = memoryview(store).cast('d')
assert m.take_view(probe) == 7
probe.release()
store.append(0)

# ...and the same after a refusal at argument 2, which must release the
# buffer argument 1 already acquired.
store2 = bytearray(8)
probe2 = memoryview(store2).cast('d')
refuse(m.view_then_int, (probe2, 'x'), {}, TypeError,
       "view_then_int() argument 2: 'str' object cannot be interpreted as an integer",
       True, (probe2, 3), 3)
probe2.release()
store2.append(0)

# Refusal ordering: with two simultaneously non-conforming arguments, the
# message names argument 1. Arguments are unpacked left to right and the
# first failure returns, so the second one is never reached.
refuse(m.join, (1, 2), {}, TypeError,
       "join() argument 1: 'int' object cannot be interpreted as a str",
       True, ('a', 'b'), 'ab')
"#;

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_built_ext_module_refuses_every_non_conforming_call_shape_at_the_boundary() {
    let dir = ScratchDir::new("1067_ext").expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, FIXTURE).expect("write the fixture");
    let build = Command::new(std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc")))
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("pycc_1067_mod"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(SCRIPT)
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}
