//! The `tuple` half of the `ext` boundary against a real CPython (#1050,
//! Part 3 of #1037).
//!
//! Every test here is `#[ignore]`d, exactly as `tests/issue_1049_ext_str.rs`
//! is: each builds an artifact and asks an installed CPython 3.13+ to import
//! it, which is a property of the machine rather than of the change under
//! test. CI runs them on every Tier-1 `native-build-test` leg through that
//! job's `cargo test --workspace -- --include-ignored`.
//!
//! They are deliberately *not* where line coverage comes from -- the coverage
//! job runs `llvm-cov` without `--include-ignored`, so an ignored test earns
//! none, and the new unpack helpers live in C, which `llvm-cov` does not
//! instrument at all. `src/ext_build_tests/generated_c.rs` covers the emitted
//! text and `crates/pycc_codegen/src/ext_thunk_tests.rs` covers the emitted
//! thunk signatures. This file covers the one thing neither can: that the two
//! independently-derived spellings of the same ABI actually agree at run
//! time.
//!
//! That agreement is the whole reason the thunk exists. pycc's convention for
//! passing and returning an aggregate is not the platform C struct ABI --
//! measured on aarch64-apple-darwin, a pycc function returning
//! `tuple[int, int, int, int, int]` hands the five words back in `x0`-`x4`
//! where clang's C ABI passes a hidden `sret` pointer -- and the `--ext` link
//! defers undefined symbols (`-undefined dynamic_lookup` on Mach-O,
//! `-Bsymbolic` on ELF), so every disagreement in this area links cleanly and
//! shows up only as a fault at the first call. The five-element fixtures
//! below are the ones that were observed to fault before the thunk existed.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Builds `body` as an extension module named `module` inside `dir`.
fn build_ext(dir: &Path, module: &str, body: &str) {
    let src = dir.join("m.py");
    std::fs::write(&src, body).expect("write the fixture source");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
}

/// Runs `script` with the built module importable from `dir`.
fn run_python(dir: &Path, script: &str) {
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        stderr_of(&run)
    );
}

/// The conforming calls, at every arity that distinguishes a convention.
///
/// Arity 1 is where a flattening bug hides: `tuple[int]` and `int` occupy the
/// same single slot at the thunk, so only the `PyTuple_Check` on the way in
/// and the `PyTuple_New(1)` on the way out keep the Python-level types apart.
/// Arity 5 is where the aggregate ABI stops fitting registers on every target
/// this runs on, and `-> tuple[int; 5]` is the exact signature that faulted
/// through the pre-#1050 `void *`-cast call. `float` and `bool` are present
/// in both directions because the three element widths differ (`i64`, `f64`,
/// `i8`) and a single wrong one is a silent misread rather than a crash.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_built_ext_module_round_trips_tuples_at_every_arity_and_element_type() {
    let dir = ScratchDir::new("ext_tuple_ok").expect("scratch");
    build_ext(
        &dir,
        "pycc_tuple_mod",
        "def one(t: tuple[int]) -> tuple[int]:\n    return (t[0] + 1,)\n\n\
         def pair(t: tuple[int, int]) -> tuple[int, int]:\n    return (t[1], t[0])\n\n\
         def five(a: int) -> tuple[int, int, int, int, int]:\n    \
         return (a, a + 1, a + 2, a + 3, a + 4)\n\n\
         def sum5(t: tuple[int, int, int, int, int]) -> int:\n    \
         return t[0] + t[1] + t[2] + t[3] + t[4]\n\n\
         def floats(t: tuple[float, float]) -> tuple[float, float]:\n    \
         return (t[1], t[0])\n\n\
         def flags(t: tuple[bool, bool]) -> tuple[bool, bool]:\n    \
         return (t[1], t[0])\n\n\
         def mixed(t: tuple[int, float, bool]) -> tuple[bool, float, int]:\n    \
         return (t[2], t[1], t[0])\n",
    );
    run_python(
        &dir,
        "import pycc_tuple_mod as m\n\
         assert m.one((41,)) == (42,)\n\
         assert type(m.one((1,))) is tuple\n\
         assert m.pair((1, 2)) == (2, 1)\n\
         assert m.five(10) == (10, 11, 12, 13, 14)\n\
         assert m.sum5((1, 2, 3, 4, 5)) == 15\n\
         assert m.floats((1.5, -2.25)) == (-2.25, 1.5)\n\
         assert m.flags((True, False)) == (False, True)\n\
         assert m.flags((True, False))[0] is False\n\
         assert m.mixed((7, 0.5, True)) == (True, 0.5, 7)\n\
         assert m.mixed((7, 0.5, True))[0] is True\n\
         print('ok')\n",
    );
}

/// A `bool` element is a `bool` and not a flattened `1`. D-141 encodes
/// `False`/`True` as dedicated marker words, and the shim's ingress reads
/// `PyBool_Check` before `PyLong_Check` specifically so an `int` slot cannot
/// swallow the identity -- a `tuple` element goes through the same helper,
/// so the same order has to hold one level down.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_bool_element_keeps_its_identity_and_an_int_element_refuses_a_bool_slot() {
    let dir = ScratchDir::new("ext_tuple_bool").expect("scratch");
    build_ext(
        &dir,
        "pycc_tuple_bool",
        "def ints(t: tuple[int, int]) -> int:\n    return t[0] + t[1]\n\n\
         def flags(t: tuple[bool, bool]) -> tuple[bool, bool]:\n    return (t[0], t[1])\n",
    );
    run_python(
        &dir,
        // D-244 rule 7 admits `bool` at an `int` parameter -- it is the one
        // implicit widening the boundary keeps -- so `ints` accepts them and
        // the arithmetic sees 0/1.
        "import pycc_tuple_bool as m\n\
         assert m.ints((True, True)) == 2\n\
         out = m.flags((True, False))\n\
         assert out[0] is True and out[1] is False, repr(out)\n\
         try:\n    m.flags((1, 0))\n\
         except TypeError as e:\n    assert 'element 1' in str(e), str(e)\n\
         else:\n    raise AssertionError('an int at a bool element must be refused')\n\
         print('ok')\n",
    );
}

/// The closed half of D-244 rule 7 at the tuple ingress: shape first, then
/// arity, then each element's own type. `list` is the case a C-API habit
/// would most plausibly let through -- `PySequence_Fast` would accept it --
/// and D-086 forbids the implicit conversion in either direction.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_tuple_parameter_refuses_a_non_tuple_and_every_wrong_arity() {
    let dir = ScratchDir::new("ext_tuple_typeerror").expect("scratch");
    build_ext(
        &dir,
        "pycc_tuple_refuse",
        "def f(t: tuple[int, int]) -> int:\n    return t[0]\n\n\
         def g(x: int) -> int:\n    return x\n",
    );
    run_python(
        &dir,
        // Every line of the script carries its own indentation inside the
        // literal: Rust's `\`-continuation strips the leading whitespace of
        // the next source line, so borrowed indentation would not survive.
        "import pycc_tuple_refuse as m\n\
         cases = [\n\
         (lambda: m.f([1, 2]), 'list at a tuple parameter'),\n\
         (lambda: m.f('ab'), 'str at a tuple parameter'),\n\
         (lambda: m.f(1), 'int at a tuple parameter'),\n\
         (lambda: m.f(None), 'None at a tuple parameter'),\n\
         (lambda: m.f(iter((1, 2))), 'iterator at a tuple parameter'),\n\
         (lambda: m.f((1,)), 'a shorter tuple'),\n\
         (lambda: m.f((1, 2, 3)), 'a longer tuple'),\n\
         (lambda: m.f(()), 'the empty tuple'),\n\
         (lambda: m.g((1, 2)), 'a tuple at an int parameter'),\n\
         ]\n\
         for call, what in cases:\n    try:\n        call()\n    \
         except TypeError as e:\n        assert '() argument 1' in str(e), str(e)\n    \
         else:\n        raise AssertionError(what + ' must raise TypeError')\n\
         try:\n    m.f()\n\
         except TypeError as e:\n    assert 'takes exactly 1 argument' in str(e), str(e)\n\
         else:\n    raise AssertionError('arity must be checked')\n\
         assert m.f((4, 5)) == 4\n\
         print('ok')\n",
    );
}

/// The exact refusal text, because it is the only place the two index spaces
/// meet in one sentence.
///
/// Both indices are one-based: the argument index already was, following
/// CPython's own `f() argument 1` convention, and a zero-based element
/// beside it in the same sentence reads as a typo rather than as a different
/// numbering. So `element 2` names `t[1]`. The wrong-arity message is the
/// other half -- it counts Python *arguments*, never the flattened C slots
/// the thunk actually takes, so a one-argument function whose argument is a
/// five-element tuple still says "takes exactly 1 argument".
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_refused_element_names_its_argument_and_its_one_based_position() {
    let dir = ScratchDir::new("ext_tuple_message").expect("scratch");
    build_ext(
        &dir,
        "pycc_tuple_message",
        "def place(a: int, t: tuple[int, float, bool]) -> int:\n    return a\n\n\
         def wide(t: tuple[int, int, int, int, int]) -> int:\n    return t[0]\n",
    );
    run_python(
        &dir,
        "import pycc_tuple_message as m\n\
         def message(call):\n    \
         try:\n        call()\n    \
         except TypeError as e:\n        return str(e)\n    \
         raise AssertionError('expected a TypeError')\n\
         first = message(lambda: m.place(1, ('x', 0.5, True)))\n\
         assert first == (\"place() argument 2, element 1: 'str' object \"\n\
         \"cannot be interpreted as an integer\"), first\n\
         third = message(lambda: m.place(1, (1, 0.5, 'x')))\n\
         assert third == (\"place() argument 2, element 3: 'str' object \"\n\
         \"cannot be interpreted as a bool\"), third\n\
         second = message(lambda: m.place(1, (1, 'x', True)))\n\
         assert 'argument 2, element 2' in second, second\n\
         scalar = message(lambda: m.place('x', (1, 0.5, True)))\n\
         assert 'element' not in scalar, scalar\n\
         assert scalar.startswith('place() argument 1:'), scalar\n\
         arity = message(lambda: m.wide((1, 2, 3, 4, 5), 6))\n\
         assert 'takes exactly 1 argument' in arity, arity\n\
         length = message(lambda: m.wide((1, 2)))\n\
         assert length == 'wide() argument 1: expected a tuple of length 5, got 2', length\n\
         print('ok')\n",
    );
}

/// The two `int` range failures a tuple adds, and the ownership discipline
/// each one owes.
///
/// Ingress: `pycc_rt`'s inline-integer range is [-2^62, 2^62-1], narrower
/// than `i64`, so a conforming CPython `int` can still be out of range
/// (#1040). Egress: each returned element arrives retained (D-180 rule 6)
/// and it is `pycc_ext_pack_int` that discharges the ownership, so the
/// wrapper packs *every* element before acting on any failure -- bailing at
/// the first would leak one `BigIntObj` per call, on exactly the path that
/// already raises. The loop is that leak's only observable form from Python:
/// a double free or a use-after-free in the branch faults here.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_out_of_range_int_element_raises_overflow_in_either_direction() {
    let dir = ScratchDir::new("ext_tuple_overflow").expect("scratch");
    build_ext(
        &dir,
        "pycc_tuple_overflow",
        "def take(t: tuple[int, int]) -> int:\n    return t[0]\n\n\
         def grow(t: tuple[int, int]) -> tuple[int, int]:\n    \
         return (t[0] * 1000000000, t[1] * 1000000000)\n",
    );
    run_python(
        &dir,
        "import pycc_tuple_overflow as m\n\
         inline_max = 2 ** 62 - 1\n\
         assert m.take((inline_max, 0)) == inline_max\n\
         try:\n    m.take((inline_max + 1, 0))\n\
         except OverflowError as e:\n    assert 'element 1' in str(e), str(e)\n\
         else:\n    raise AssertionError('an out-of-range element must raise')\n\
         try:\n    m.take((0, 2 ** 64))\n\
         except OverflowError as e:\n    assert 'element 2' in str(e), str(e)\n\
         else:\n    raise AssertionError('an out-of-range element must raise')\n\
         assert m.grow((2, 3)) == (2000000000, 3000000000)\n\
         for _ in range(2000):\n    \
         try:\n        m.grow((3000000000000, 3000000000000))\n    \
         except OverflowError:\n        pass\n    \
         else:\n        raise AssertionError('both elements overflow the egress')\n\
         assert m.grow((4, 5)) == (4000000000, 5000000000)\n\
         print('ok')\n",
    );
}

/// A `tuple` subclass is a tuple: `PyTuple_Check` and not
/// `PyTuple_CheckExact`, the same reading that lets a `str` subclass through
/// at #1049's boundary -- rule 7 is closed against duck typing, not against
/// subtyping. The identity does not survive: the elements are copied out by
/// value, so what comes back is an exact `tuple` even when what went in was
/// not, and even when the export is the identity function.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_tuple_subclass_is_accepted_and_the_returned_object_is_an_exact_tuple() {
    let dir = ScratchDir::new("ext_tuple_subclass").expect("scratch");
    build_ext(
        &dir,
        "pycc_tuple_subclass",
        "def echo(t: tuple[int, int]) -> tuple[int, int]:\n    return (t[0], t[1])\n",
    );
    run_python(
        &dir,
        "import pycc_tuple_subclass as m\n\
         class Pair(tuple):\n    pass\n\
         source = Pair((3, 4))\n\
         out = m.echo(source)\n\
         assert out == (3, 4), repr(out)\n\
         assert type(out) is tuple, type(out)\n\
         assert out is not source\n\
         plain = (5, 6)\n\
         assert m.echo(plain) is not plain, 'the boundary copies rather than forwarding'\n\
         import collections\n\
         Point = collections.namedtuple('Point', 'x y')\n\
         assert m.echo(Point(7, 8)) == (7, 8)\n\
         assert type(m.echo(Point(7, 8))) is tuple\n\
         print('ok')\n",
    );
}

/// A rebound public name calls what Python says it calls. The thunk reaches
/// the compiled function through the `fnptr_<name>` slot the `def` fills at
/// module level -- exactly one LLVM frame further in than the scalar path's
/// own cast through that global -- so the rebinding property #1036 tested
/// for scalars has to survive the extra indirection. The redefinition also
/// shares one slot and one signature, so a second thunk would be a duplicate
/// symbol and the artifact would not compile at all.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_rebound_public_name_dispatches_through_the_slot_the_module_body_filled() {
    let dir = ScratchDir::new("ext_tuple_rebound").expect("scratch");
    build_ext(
        &dir,
        "pycc_tuple_rebound",
        "def f(t: tuple[int, int]) -> tuple[int, int]:\n    return (t[0], t[1])\n\n\
         def f(t: tuple[int, int]) -> tuple[int, int]:\n    return (t[1], t[0])\n",
    );
    run_python(
        &dir,
        "import pycc_tuple_rebound as m\n\
         assert m.f((1, 2)) == (2, 1), 'the last definition owns the name'\n\
         print('ok')\n",
    );
}

/// A raised exception must reach Python from a `tuple`-returning export too.
/// Its out-pointer locals are left exactly as uninitialized as the call
/// found them on that path, so the wrapper's pending-flag check stands
/// *positionally* before any read of them: a pack that ran first would read
/// indeterminate storage, which is undefined behaviour rather than a wrong
/// value. A `-> None` export carrying a tuple parameter is the other arm --
/// nothing is assigned from the call at all.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_tuple_returning_export_raises_rather_than_packing_uninitialized_storage() {
    let dir = ScratchDir::new("ext_tuple_raise").expect("scratch");
    build_ext(
        &dir,
        "pycc_tuple_raise",
        "def pick(n: int) -> tuple[int, float]:\n    \
         if n == 0:\n        raise ValueError(\"zero\")\n    return (n, 1.5)\n\n\
         def sink(t: tuple[int, int]) -> None:\n    \
         if t[0] == 0:\n        raise ValueError(\"zero\")\n",
    );
    run_python(
        &dir,
        "import pycc_tuple_raise as m\n\
         assert m.pick(1) == (1, 1.5)\n\
         for _ in range(2000):\n    \
         try:\n        m.pick(0)\n    \
         except ValueError as e:\n        assert str(e) == 'zero', str(e)\n    \
         else:\n        raise AssertionError('the exception did not cross the boundary')\n\
         assert m.pick(2) == (2, 1.5)\n\
         assert m.sink((1, 2)) is None\n\
         try:\n    m.sink((0, 2))\n\
         except ValueError:\n    pass\n\
         else:\n    raise AssertionError('a None-returning export must raise too')\n\
         print('ok')\n",
    );
}

/// #1050 regression: a *stored* tuple's elements are borrowed, so the
/// `tuple` egress must take its own reference before the packer discharges
/// one. `pycc_ext_pack_int` releases a heap-bigint word on its
/// `OverflowError` path; without a matching retain that release decrements a
/// count the module global still holds, and the *second* call faults inside
/// the host interpreter (observed before the fix as a non-unwinding panic in
/// `pycc_rt_bigint_release`, an outright use-after-free in a release build).
///
/// So one call proves nothing here -- the loop is the test. The `bool` and
/// `float` elements are the other half of the contract: only the `int` slots
/// take a reference, because only the `int` packer discharges one.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_stored_tuple_of_bigints_survives_repeated_refused_returns() {
    let dir = ScratchDir::new("ext_tuple_borrowed_bigint").expect("scratch");
    build_ext(
        &dir,
        "pycc_tuple_borrowed",
        "saved: tuple[int, bool, float] = ((2 ** 62 - 1) + 1, True, 1.5)\n\n\
         def get() -> tuple[int, bool, float]:\n    return saved\n\n\
         def small() -> tuple[int, int]:\n    return (1, 2)\n",
    );
    run_python(
        &dir,
        "import pycc_tuple_borrowed as m\n\
         for _ in range(2000):\n    \
         try:\n        m.get()\n    \
         except OverflowError:\n        pass\n    \
         else:\n        raise AssertionError('an out-of-range int must be refused')\n\
         assert m.small() == (1, 2)\n\
         print('ok')\n",
    );
}
