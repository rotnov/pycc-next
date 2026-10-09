//! #1346: a `@staticmethod` called through a non-name instance receiver --
//! `make().h(1)` -- evaluates the receiver first, as CPython does, even
//! though the static method never sees it. Before #1346 the receiver was
//! dropped: `FS().h(1)` never ran `FS.__init__`, `make().h(2)` never called
//! `make()`, and `boom().h(1)` returned a value instead of raising.
//!
//! These are native builds with no foreign import, so they need no
//! interpreter and run in the coverage job. Each expected output is
//! CPython 3.14.7's own for the same source, recorded here; the foreign
//! `staticmethod(<callable>)` counterparts are compared live against the
//! host interpreter in `tests/issue_1284_foreign_static_class_attr.rs`.

use pycc_scratch::ScratchDir;
use std::process::Command;

/// The class every case calls through: an `__init__` that prints, and one
/// static method per result representation (small int, bigint, `str`,
/// `None`).
const FS: &str = "class FS:\n\
    \x20   def __init__(self) -> None:\n        print(\"init\")\n\n\
    \x20   @staticmethod\n    def h(x: int) -> int:\n        return x + 1\n\n\
    \x20   @staticmethod\n    def big(x: int) -> int:\n        return x + 9223372036854775807\n\n\
    \x20   @staticmethod\n    def s(x: str) -> str:\n        return x + \"!\"\n\n\
    \x20   @staticmethod\n    def nothing(x: int) -> None:\n        print(\"nothing\", x)\n\n\n\
    def make() -> FS:\n    print(\"make\")\n    return FS()\n\n\n\
    def boom() -> FS:\n    raise ValueError(\"boom\")\n\n\n\
    def mk(n: int) -> FS:\n    print(\"mk\", n)\n    return FS()\n\n\n\
    def side() -> int:\n    print(\"side\")\n    return 5\n\n\n";

/// Builds `FS` followed by `rest` natively and returns the binary's stdout.
fn run(tag: &str, rest: &str) -> String {
    let dir = ScratchDir::new(tag).expect("scratch");
    let source = dir.join("m.py");
    std::fs::write(&source, format!("{FS}{rest}")).expect("write the fixture source");
    let binary = dir.join("app");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&binary).output().expect("the binary runs");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n")
}

/// The receiver runs before the arguments and the call, and a receiver
/// that raises skips both.
#[test]
fn the_receiver_is_evaluated_before_the_arguments_and_the_call() {
    let out = run(
        "1346_order",
        "print(FS().h(1))\nprint(make().h(2))\nprint(make().h(side()))\n\
         try:\n    print(boom().h(1))\nexcept ValueError:\n    print(\"caught\")\n",
    );
    assert_eq!(out, "init\n2\nmake\ninit\n3\nmake\ninit\nside\n6\ncaught\n");
}

/// An int result bound and aliased, a bigint result in a loop, a `str`
/// result bound, aliased and printed directly, and a `None` result in
/// statement position all keep their ownership through the sequence.
#[test]
fn every_result_representation_survives_the_sequence() {
    let out = run(
        "1346_results",
        "x = FS().h(1)\ny = x\nprint(x, y)\n\
         for i in range(2):\n    b = FS().big(i) + 1\n    print(b)\n\
         t = FS().s(\"a\")\nu = t\nprint(t, u)\nprint(FS().s(\"b\"))\n\
         FS().nothing(1)\n\
         def f() -> None:\n    total = 0\n    for i in range(3):\n\
         \x20       total = total + FS().big(i) + 1\n    print(total)\n\n\n\
         f()\n",
    );
    assert_eq!(
        out,
        "init\n2 2\ninit\n9223372036854775808\ninit\n9223372036854775809\n\
         init\na! a!\ninit\nb!\ninit\nnothing 1\ninit\ninit\ninit\n27670116110564327427\n"
    );
}

/// The receiver stays inside the short-circuit operand it belongs to.
#[test]
fn a_short_circuited_receiver_is_not_evaluated() {
    let out = run(
        "1346_short_circuit",
        "def probe(flag: bool) -> None:\n    if flag and FS().h(1) == 2:\n\
         \x20       print(\"yes\")\n    else:\n        print(\"no\")\n\n\n\
         probe(False)\nprobe(True)\n",
    );
    assert_eq!(out, "no\ninit\nyes\n");
}

/// A walrus inside the receiver binds its target at module scope and in a
/// function body (the two get their slots through different paths). Before
/// #1346 this crashed codegen: the dropped receiver took the binding with
/// it, so the later read of `k` found no slot.
#[test]
fn a_walrus_inside_the_receiver_binds_its_target() {
    let out = run(
        "1346_walrus",
        "print(mk((k := 4)).h(2))\nprint(k)\n\
         def f() -> None:\n    print(mk((j := 5)).h(2))\n    print(j)\n\n\n\
         f()\n",
    );
    assert_eq!(out, "mk 4\ninit\n3\n4\nmk 5\ninit\n3\n5\n");
}
