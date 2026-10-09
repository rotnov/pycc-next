//! End-to-end proof for chained assignment ([#1213], Part 5 of [#1018]).
//!
//! `docs/TYPE_SYSTEM.md`'s "Chained assignment" section is the contract.
//! The byte-exact oracle fixture is `tests/fixtures/chain_assign.py`
//! (registered in `tests/conformance/classes.rs`); this file owns the
//! refusals, the two-module temporary pin and the hosted `--ext` arm.
//!
//! [#1213]: https://github.com/rotnov/pycc/issues/1213
//! [#1018]: https://github.com/rotnov/pycc/issues/1018

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn rendered(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text.replace("\r\n", "\n")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
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

/// The entry module of the two-module program. Its import line has the
/// same length as `DEP`'s first line, so both chains, and both nested
/// chains, start at the same byte offset and share one temporary name.
const ENTRY: &str = "\
from b import xb, yb
def g() -> int:
    return 1
xa = ya = g()
for i in range(1):
    pa = qa = g()
    print(pa + qa)
print(xa + ya + xb + yb)
";

const DEP: &str = "\
# same offsets as a.
def h() -> int:
    return 2
xb = yb = h()
for j in range(1):
    pb = qb = h()
    print(pb + qb)
";

/// Two modules whose chained assignments sit at the same byte offsets link
/// and run as CPython does: the synthesized temporary is not a definition
/// the cross-module collision check sees, neither for a top-level chain nor
/// for one nested in a module-level `for` body.
#[test]
fn two_modules_with_chains_at_the_same_offsets_link_and_match_cpython() {
    assert_eq!(ENTRY.find("xa = "), DEP.find("xb = "));
    assert_eq!(ENTRY.find("pa = "), DEP.find("pb = "));
    let dir = ScratchDir::new("e2e_1213_two_modules").expect("scratch");
    std::fs::write(dir.join("a.py"), ENTRY).expect("write the entry module");
    std::fs::write(dir.join("b.py"), DEP).expect("write the dependency");
    let build = pycc()
        .arg("build")
        .arg(dir.join("a.py"))
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let run = Command::new(dir.join("app"))
        .output()
        .expect("the program should spawn");
    assert!(run.status.success(), "{}", rendered(&run));
    let oracle = python()
        .arg("a.py")
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(oracle.status.success(), "{}", rendered(&oracle));
    assert_eq!(stdout(&run), stdout(&oracle));
    assert_eq!(stdout(&run), "4\n2\n6\n");
}

/// The refusals a chain can reach: an empty display (the empty-container
/// pass could only name the temporary), a tuple piece with a non-name
/// element (the tuple-unpacking refusal, Part 1 of #891, spanned on the
/// element), a walrus value (the placement
/// check still sees it on the temporary's store) and a class-body chain (the
/// class-attribute path, unchanged).
#[test]
fn every_chained_assignment_refusal_is_named_and_located() {
    for (source, header, location) in [
        (
            "def f() -> int:\n    a = b = []\n    return len(a)\n",
            "error[C0001]: chained assignment of an empty `[]`/`{}` literal is not supported \
             yet; inside a function, annotate one name and assign it (`a: list[int] = []`, then \
             `b = a`)",
            "2:5",
        ),
        (
            "a = b = {}\n",
            "error[C0001]: chained assignment of an empty `[]`/`{}` literal is not supported yet",
            "1:1",
        ),
        (
            "t = (1, 2)\na = (b, c.d) = t\n",
            "error[C0001]: only bare-name targets in a tuple-unpacking assignment are \
             supported so far, got an attribute",
            "2:9",
        ),
        (
            "def f() -> int:\n    a = b = (x := 5)\n    return a\n",
            "error[C0001]: a walrus assignment (`:=`) is only supported in an `if`/`while` \
             condition or as a bare expression statement",
            "2:5",
        ),
        (
            "class C:\n    A = B = 1\n",
            "error[C0001]: a class-level attribute assignment must have a single target",
            "2:",
        ),
    ] {
        let text = check_fails("e2e_1213_refusals", source);
        assert!(text.contains(header), "{source}: {text}");
        assert!(
            text.contains(&format!("subject.py:{location}")),
            "{source}: expected the diagnostic at {location}: {text}"
        );
    }
}

/// A chain whose pieces the single-target arm types differently from each
/// other is refused exactly as the separate assignments are.
#[test]
fn a_chain_is_typed_as_its_single_target_assignments() {
    let text = check_fails(
        "e2e_1213_typing",
        "def f() -> None:\n    x = 1.5\n    y = 2\n    x = y = 3\n",
    );
    assert!(text.contains("error[T0023]"), "{text}");
}

/// The hosted subject: buffer element stores through a chain, with a
/// literal value (copied per target) and a computed one (bound once).
const SUBJECT: &str = "\
def fill(b: memoryview) -> None:
    b[0] = b[1] = 1.0


def spread(b: memoryview) -> None:
    b[0] = b[1] = b[2] * 2.0
";

/// A chained store into a buffer element is the plain `DictSet` store the
/// writability walk looks for: the export acquires the host buffer
/// writable, writes every target, and refuses a read-only exporter with
/// CPython's `BufferError`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn hosted_chained_buffer_stores_acquire_the_buffer_writable() {
    let dir = ScratchDir::new("e2e_1213_hosted").expect("scratch");
    std::fs::write(dir.join("chain_probe.py"), SUBJECT).expect("write the subject");
    let build = pycc()
        .arg("build")
        .arg(dir.join("chain_probe.py"))
        .arg("-o")
        .arg(dir.join("chain_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let run = hosted(
        &dir,
        "import array, chain_probe\n\
         data = array.array('d', [0.0, 0.0, 4.0])\n\
         chain_probe.fill(memoryview(data))\n\
         assert list(data) == [1.0, 1.0, 4.0], list(data)\n\
         chain_probe.spread(memoryview(data))\n\
         assert list(data) == [8.0, 8.0, 4.0], list(data)\n\
         frozen = bytes(24)\n\
         for call in (chain_probe.fill, chain_probe.spread):\n\
         \x20   try:\n\
         \x20       call(memoryview(frozen).cast('d'))\n\
         \x20   except BufferError:\n\
         \x20       pass\n\
         \x20   else:\n\
         \x20       raise AssertionError('a read-only exporter was accepted')\n\
         assert frozen == bytes(24), frozen\n\
         print('ok')\n",
    );
    assert!(run.status.success(), "{}", rendered(&run));
    assert_eq!(stdout(&run), "ok\n");
}

fn hosted(dir: &Path, program: &str) -> Output {
    python()
        .arg("-c")
        .arg(program)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn")
}
