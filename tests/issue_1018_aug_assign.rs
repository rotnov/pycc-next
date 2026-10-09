//! End-to-end proof for augmented assignment ([#1209], Part 1 of [#1018]).
//!
//! `docs/TYPE_SYSTEM.md`'s "Augmented assignment" section is the contract:
//! an admitted `target op= value` is lowered as `target = target op value`,
//! and every shape or type that rewrite could not honour exactly is refused.
//! The byte-exact oracle fixtures are `tests/fixtures/aug_assign_scalars.py`
//! and `tests/fixtures/aug_assign_targets.py`; this file owns the refusals,
//! the soundness-condition pins (S1 to S4) and the hosted `--ext` arms.
//!
//! [#1209]: https://github.com/rotnov/pycc/issues/1209
//! [#1018]: https://github.com/rotnov/pycc/issues/1018

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn rendered(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text.replace("\r\n", "\n")
}

/// Runs `pycc check` on `source` and returns the rendered diagnostics,
/// asserting that the check failed.
fn check_fails(category: &str, source: &str) -> String {
    let dir = ScratchDir::new(category).expect("scratch");
    let path = dir.join("subject.py");
    std::fs::write(&path, source).expect("write the subject");
    let output = pycc()
        .arg("check")
        .arg(&path)
        .output()
        .expect("pycc should spawn");
    assert!(!output.status.success(), "{source} was accepted");
    rendered(&output)
}

/// Asserts `source` is refused with exactly `header` (code and message)
/// located at `location` (`line:column` of the augmented statement).
fn assert_refused(category: &str, source: &str, header: &str, location: &str) {
    let text = check_fails(category, source);
    assert!(text.contains(header), "{source}: {text}");
    assert!(
        text.contains(&format!("subject.py:{location}")),
        "{source}: expected the diagnostic at {location}: {text}"
    );
}

/// Every `C0001` the augmented-assignment gates raise, spelled for `op=`
/// and spanned on the augmented statement itself.
#[test]
fn every_augmented_assignment_capability_refusal_is_named_and_located() {
    for (source, header) in [
        (
            "x = 1\nx @= 1\n",
            "error[C0001]: augmented assignment operator `@=` is not supported yet",
        ),
        (
            "class C:\n    def __init__(self) -> None:\n        self.n = 0\n\n\
             def f(c: C) -> C:\n    return c\n\nx = C()\nf(x).n += 1\n",
            "error[C0001]: augmented assignment to an attribute of a computed expression is not \
             supported yet; bind the object to a name first",
        ),
        (
            "def counts() -> dict[str, int]:\n    return {\"a\": 1}\n\ncounts()[\"a\"] += 1\n",
            "error[C0001]: augmented assignment to a subscript of a computed expression is not \
             supported yet; bind the object to a name first",
        ),
        (
            "d: dict[str, int] = {\"a\": 1}\nd[0:1] += 1\n",
            "error[C0001]: augmented assignment to a slice is not supported yet",
        ),
    ] {
        let line = source.lines().count();
        assert_refused("e2e_1018_c0001", source, header, &format!("{line}:1"));
    }
}

/// S4: the index purity predicate is `is_unobservable`, so an index that is
/// neither a name nor a literal is refused rather than evaluated twice.
#[test]
fn s4_a_computed_index_is_refused_rather_than_evaluated_twice() {
    for source in [
        "d: dict[str, int] = {\"a\": 1}\ndef key() -> str:\n    return \"a\"\n\nd[key()] += 1\n",
        "def f(d: dict[str, int], k: str) -> None:\n    d[k + \"x\"] += 1\n",
    ] {
        let line = source.lines().count();
        assert_refused(
            "e2e_1018_s4",
            source,
            "error[C0001]: augmented assignment with a computed index is not supported yet; \
             bind the index to a name first",
            &format!("{line}:"),
        );
    }
}

/// S1: `BinOp` admits only immutable operand types, so a mutable or
/// user-defined operand is refused, never rebound to a fresh object in place
/// of CPython's in-place `__iadd__`.
#[test]
fn s1_a_mutable_or_user_defined_operand_is_refused() {
    for (source, header) in [
        (
            "xs: list[int] = [1]\nxs += [1]\n",
            "error[T0021]: operator Add is not defined for `list[int]` and `list[int]`",
        ),
        (
            "t = (1,)\nt += (2,)\n",
            "error[T0021]: operator Add is not defined for `tuple[int]` and `tuple[int]`",
        ),
        (
            "class V:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
             def __iadd__(self, o: int) -> int:\n        return 0\n\na = V(1)\na += 2\n",
            "error[T0021]: operator Add is not defined for `V` and `int`",
        ),
    ] {
        let text = check_fails("e2e_1018_s1", source);
        assert!(text.contains(header), "{source}: {text}");
    }
}

/// S2: `global` is refused, so no call inside the value can rebind the
/// container or index between the load and the store.
#[test]
fn s2_a_global_declaration_is_still_refused() {
    let text = check_fails(
        "e2e_1018_s2",
        "x = 1\ndef g() -> None:\n    global x\n    x += 1\n",
    );
    assert!(
        text.contains("error[C0001]: statement kind not supported yet: a `global` declaration"),
        "{text}"
    );
}

/// S3: a walrus in the value, which could rebind the index, the container or
/// the name between the load and the store, is refused by the placement
/// check the rewritten statement runs through.
#[test]
fn s3_a_walrus_in_the_value_is_refused() {
    for source in [
        "d: dict[str, int] = {\"a\": 1}\nk = \"a\"\nd[k] += (k := \"b\")\n",
        "d: dict[str, int] = {\"a\": 1}\nk = \"a\"\nd[k] += len(d := {\"a\": 1})\n",
        "x = 1\nx += (x := 2)\n",
    ] {
        let line = source.lines().count();
        assert_refused(
            "e2e_1018_s3",
            source,
            "error[C0001]: a walrus assignment (`:=`) is only supported in an `if`/`while` \
             condition or as a bare expression statement (#774)",
            &format!("{line}:1"),
        );
    }
}

/// Target and scope pins: each is the refusal the plain assignment
/// `target = target op value` already gets, which is also where CPython
/// raises.
#[test]
fn every_inherited_refusal_is_the_plain_assignments() {
    for (source, header) in [
        // `__init__` reads `self.n` before any `self.n = ...`: CPython raises
        // `AttributeError`; an augmented assignment is not a declaration.
        (
            "class C:\n    def __init__(self) -> None:\n        self.m = 0\n        self.n += 1\n",
            "error[T0044]: class `C` has no attribute named `n`",
        ),
        // A class-level attribute has no storage to write to (#911).
        (
            "class C:\n    count = 0\n\n    def bump(self) -> None:\n        self.count += 1\n",
            "error[T0044]: cannot assign to `count`: it is a class-level attribute of class `C`",
        ),
        (
            "class C:\n    count = 0\n\nC.count += 1\n",
            "error[T0021]: name `C` is not defined",
        ),
        // A class-body augmented assignment stays behind the class-body gate.
        (
            "class C:\n    n = 0\n    n += 1\n",
            "error[C0001]: a class body statement must be a method definition",
        ),
        // A local that is only bound by the augmented assignment itself:
        // CPython raises `UnboundLocalError`.
        (
            "x = 1\ndef h() -> None:\n    x += 1\n",
            "error[T0021]: local name `x` is not bound before this use",
        ),
        // A getter-only property.
        (
            "class C:\n    def __init__(self) -> None:\n        self._n = 1\n\n    @property\n    \
             def n(self) -> int:\n        return self._n\n\nc = C()\nc.n += 1\n",
            "error[T0044]: property `n` of class `C` is read-only (has no setter)",
        ),
        // A `bool` name cannot be rebound to the `int` the sum produces.
        (
            "b = True\nb += 1\n",
            "error[T0023]: cannot assign `int` to `b`, previously inferred as `bool`",
        ),
        // Narrowing is killed by the rewritten assignment, exactly as by
        // `x = x + 1`, so the narrowed type does not survive it.
        (
            "def f(x: int | None) -> int:\n    if x is not None:\n        x += 1\n        \
             return x\n    return 0\n",
            "error[T0022]: return type mismatch: expected `int`, found `int | None`",
        ),
    ] {
        let text = check_fails("e2e_1018_inherited", source);
        assert!(text.contains(header), "{source}: {text}");
    }
}

/// The hosted subject: accumulators in a function, in a method and through
/// a buffer element, plus a store whose value expression announces itself.
const SUBJECT: &str = "\
def scale(b: memoryview, k: float) -> None:
    i = 0
    while i < len(b):
        b[i] *= k
        i += 1


def total(b: memoryview) -> float:
    s = 0.0
    i = 0
    while i < len(b):
        s += b[i]
        i += 1
    return s


def noisy() -> float:
    print(\"value\")
    return 1.0


def bump_at(b: memoryview, i: int) -> None:
    b[i] += noisy()


class Acc:
    def __init__(self) -> None:
        self.total = 0.0

    def add(self, b: memoryview) -> None:
        i = 0
        while i < len(b):
            self.total += b[i]
            i += 1

    def value(self) -> float:
        return self.total
";

fn build_ext(dir: &Path) -> Output {
    std::fs::write(dir.join("aug_probe.py"), SUBJECT).expect("write the subject");
    pycc()
        .arg("build")
        .arg(dir.join("aug_probe.py"))
        .arg("-o")
        .arg(dir.join("aug_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
}

fn run_hosted(dir: &Path, program: &str) -> Output {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(program)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn")
}

/// The interop accumulators, called from a real interpreter: `b[i] *= k`
/// writes the host's memory, `s += b[i]` and `self.total += b[i]` sum it, an
/// out-of-range `b[i] += f()` raises `IndexError` before `f` runs (the load
/// comes first, as in CPython), and a read-only exporter handed to a storing
/// export is refused with CPython's `BufferError` because the rewritten
/// statement is the store the writability walk looks for.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn hosted_augmented_accumulators_match_cpython() {
    let dir = ScratchDir::new("e2e_1018_hosted").expect("scratch");
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", rendered(&build));
    let run = run_hosted(
        &dir,
        "import array, aug_probe\n\
         data = array.array('d', [1.5, -2.25, 3.0])\n\
         aug_probe.scale(memoryview(data), 2.0)\n\
         assert list(data) == [3.0, -4.5, 6.0], list(data)\n\
         assert aug_probe.total(memoryview(data)) == 4.5\n\
         acc = aug_probe.Acc()\n\
         acc.add(memoryview(data))\n\
         acc.add(memoryview(data))\n\
         assert acc.value() == 9.0, acc.value()\n\
         aug_probe.bump_at(memoryview(data), 0)\n\
         assert list(data) == [4.0, -4.5, 6.0], list(data)\n\
         try:\n\
         \x20   aug_probe.bump_at(memoryview(data), 3)\n\
         except IndexError as error:\n\
         \x20   assert 'out of bounds' in str(error), str(error)\n\
         else:\n\
         \x20   raise AssertionError('an out-of-range augmented store was accepted')\n\
         assert list(data) == [4.0, -4.5, 6.0], list(data)\n\
         frozen = bytes(24)\n\
         for call in (\n\
         \x20   lambda: aug_probe.scale(memoryview(frozen).cast('d'), 2.0),\n\
         \x20   lambda: aug_probe.bump_at(memoryview(frozen).cast('d'), 0),\n\
         ):\n\
         \x20   try:\n\
         \x20       call()\n\
         \x20   except BufferError:\n\
         \x20       pass\n\
         \x20   else:\n\
         \x20       raise AssertionError('a read-only exporter was accepted')\n\
         assert frozen == bytes(24), frozen\n\
         assert aug_probe.total(memoryview(frozen).cast('d')) == 0.0\n\
         print('ok')\n",
    );
    assert!(run.status.success(), "{}", rendered(&run));
    // `value` is printed once, by the in-range call only.
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n"),
        "value\nok\n"
    );
}
