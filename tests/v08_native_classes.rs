//! v0.8 accepts 1 and 4: what keeps library classes compiled natively, and
//! what makes their object traffic cheaper.
//!
//! * An annotated attribute store in a function (`self.x: T = v`) compiles
//!   for any `T` and value (`pycc_ast::desugar::ann_attrs`): before, only
//!   `list[int]`/`dict[str, int]` with an empty display did, and any other
//!   such store in a method sent its class to CPython. The fixture is
//!   compared with CPython as a `--non-strict` native program and as an
//!   `--ext --non-strict` module, and no function, method or class of it
//!   may run in CPython.
//! * A `--release` `--ext` module counts CPython objects inline
//!   (`pycc_codegen::object_rc`): the probe drives every object-traffic
//!   shape a hot loop has and checks that no count moves and that every
//!   temporary is freed when its last reference goes.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn last_line(bytes: &[u8]) -> String {
    text(bytes).lines().last().unwrap_or_default().to_string()
}

fn fixture(name: &str, dir: &Path, as_name: &str) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.py"));
    let path = dir.join(as_name);
    std::fs::copy(source, &path).expect("copy the fixture");
    path
}

fn assert_matches(ours: &Output, oracle: &Output) {
    assert_eq!(text(&pycc_scratch::lf(&ours.stdout)), text(&pycc_scratch::lf(&oracle.stdout)), "{}", text(&pycc_scratch::lf(&ours.stderr)));
    assert_eq!(ours.status.code(), oracle.status.code(), "{}", text(&pycc_scratch::lf(&ours.stderr)));
    assert_eq!(last_line(&pycc_scratch::lf(&ours.stderr)), last_line(&pycc_scratch::lf(&oracle.stderr)));
}

/// Builds `source` with `flags`, asserting it builds and that no function,
/// method or class runs in CPython.
fn build(source: &Path, out: &Path, flags: &[&str]) -> String {
    let build = pycc()
        .args(["build", "--report-fallbacks"])
        .args(flags)
        .arg(source)
        .arg("-o")
        .arg(out)
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    for unit in ["fallback: function", "fallback: method", "fallback: class"] {
        assert!(!report.contains(unit), "nothing should run in CPython:\n{report}");
    }
    report
}

fn native(name: &str, flags: &[&str]) {
    let dir = ScratchDir::new(&format!("{name}_native")).expect("scratch");
    let source = fixture(name, &dir, "m.py");
    let mut all = vec!["--non-strict"];
    all.extend_from_slice(flags);
    build(&source, &dir.join("app"), &all);
    let ours = Command::new(dir.join("app")).output().expect("run the build");
    let oracle = python().arg(&source).output().expect("python should spawn");
    assert_matches(&ours, &oracle);
}

fn ext(name: &str, flags: &[&str]) {
    let compiled = ScratchDir::new(&format!("{name}_ext")).expect("scratch");
    let source = fixture(name, &compiled, "m.py");
    let mut all = vec!["--ext", "--non-strict"];
    all.extend_from_slice(flags);
    build(&source, &compiled.join("m"), &all);
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    let plain = ScratchDir::new(&format!("{name}_ext_src")).expect("scratch");
    fixture(name, &plain, "m.py");
    let run = |dir: &Path| {
        python()
            .args(["-c", "import m"])
            .current_dir(dir)
            .output()
            .expect("python should spawn")
    };
    assert_matches(&run(&compiled), &run(&plain));
}

#[test]
fn annotated_attribute_stores_native() {
    native("v08_annotated_attrs", &[]);
}

#[test]
fn annotated_attribute_stores_ext() {
    ext("v08_annotated_attrs", &[]);
}

#[test]
fn annotated_attribute_stores_ext_release() {
    ext("v08_annotated_attrs", &["--release"]);
}

/// Strict mode checks the stored value against the annotation as it checks
/// a local's (`T0025`, at the store's line), and a matching store compiles
/// with the attribute typed by it.
#[test]
fn annotated_attribute_stores_strict() {
    let dir = ScratchDir::new("v08_ann_attr_strict").expect("scratch");
    let good = dir.join("good.py");
    std::fs::write(
        &good,
        "class C:\n    def __init__(self, n: int) -> None:\n        self.n: int = n * 2\n        \
         self.label: str = 'c' + str(n)\n\n\
         def show(n: int) -> str:\n    c = C(n)\n    c.n: int = c.n + 1\n    return c.label + ':' + str(c.n)\n",
    )
    .unwrap();
    build(&good, &dir.join("good"), &["--ext"]);
    let run = python()
        .args(["-c", "import good; print(good.show(4), good.C(1).n)"])
        .current_dir(&*dir)
        .output()
        .expect("python should spawn");
    assert_eq!(text(&pycc_scratch::lf(&run.stdout)), "c4:9 2\n", "{}", text(&pycc_scratch::lf(&run.stderr)));
    let bad = dir.join("bad.py");
    std::fs::write(
        &bad,
        "class C:\n    def __init__(self, n: int) -> None:\n        self.n: int = n\n\n\
         def f(c: C) -> None:\n    c.n: int = 'x'\n",
    )
    .unwrap();
    let check = pycc().arg("check").arg(&bad).output().expect("pycc should spawn");
    let message = text(&pycc_scratch::lf(&check.stderr)) + &text(&pycc_scratch::lf(&check.stdout));
    assert!(!check.status.success(), "{message}");
    assert!(message.contains("T0025"), "{message}");
    assert!(message.contains("bad.py:6:"), "{message}");
}

/// Inline reference counting in a `--release` `--ext` module: across many
/// trips through every way compiled code takes, passes on, stores and drops
/// a CPython object, a mortal object's count ends where it started, and
/// each temporary CPython creates is freed as soon as compiled code drops
/// its last reference (a `weakref.finalize` callback counts the frees).
#[test]
fn inline_object_counts_balance_and_free() {
    let dir = ScratchDir::new("v08_inline_rc").expect("scratch");
    let source = dir.join("rc.py");
    std::fs::write(
        &source,
        "from typing import Any\n\n\
         class Holder:\n    def __init__(self, item: Any) -> None:\n        self.item = item\n        self.seen = 0\n\n    \
         def swap(self, item: Any) -> Any:\n        old = self.item\n        self.item = item\n        self.seen += 1\n        return old\n\n\
         def churn(make: Any, keep: Any, n: int) -> int:\n    h = Holder(keep)\n    total = 0\n    \
         for i in range(n):\n        obj = make(i)\n        pair = [obj, keep]\n        old = h.swap(obj)\n        \
         if old is not keep:\n            total += len(pair)\n        d = {'k': obj}\n        total += len(d)\n    \
         h.swap(keep)\n    return total + h.seen\n",
    )
    .unwrap();
    build(&source, &dir.join("rc"), &["--ext", "--non-strict", "--release"]);
    let probe = "import rc, sys, weakref\n\
                 class T:\n    pass\n\
                 freed = [0]\n\
                 def make(i):\n    t = T()\n    weakref.finalize(t, lambda: freed.__setitem__(0, freed[0] + 1))\n    return t\n\
                 keep = T()\n\
                 before = sys.getrefcount(keep)\n\
                 print(rc.churn(make, keep, 1000))\n\
                 print(sys.getrefcount(keep) - before, freed[0])\n\
                 print(rc.churn(make, keep, 50000))\n\
                 print(sys.getrefcount(keep) - before, freed[0])\n\
                 print(sys.getrefcount(None) == sys.getrefcount(None))\n";
    let run = python()
        .args(["-c", probe])
        .current_dir(&*dir)
        .output()
        .expect("python should spawn");
    let oracle_dir = ScratchDir::new("v08_inline_rc_src").expect("scratch");
    std::fs::copy(&source, oracle_dir.join("rc.py")).unwrap();
    let oracle = python()
        .args(["-c", probe])
        .current_dir(&*oracle_dir)
        .output()
        .expect("python should spawn");
    assert_eq!(text(&pycc_scratch::lf(&run.stdout)), text(&pycc_scratch::lf(&oracle.stdout)), "{}", text(&pycc_scratch::lf(&run.stderr)));
    assert_eq!(text(&pycc_scratch::lf(&run.stdout)), "3999\n0 1000\n199999\n0 51000\nTrue\n");
}

/// A non-strict function or method returning `T | None` is published
/// through an entry returning the result as a CPython object (the boundary
/// carries no `T | None`), so the method's class stays a native type: Python
/// callers get `None` or the value, as from the sources.
#[test]
fn optional_results_cross_the_boundary() {
    let dir = ScratchDir::new("v08_optional_returns").expect("scratch");
    let source = dir.join("opt.py");
    std::fs::write(
        &source,
        "from typing import Optional, Union\n\n\n\
         class Cursor:\n    def __init__(self, text: str) -> None:\n        self.text = text\n        \
         self.pos = 0\n\n    \
         def find(self, ch: str) -> Optional[int]:\n        i = self.text.find(ch, self.pos)\n        \
         if i < 0:\n            return None\n        self.pos = i + 1\n        return i\n\n    \
         def word(self) -> str | None:\n        return self.text if self.text else None\n\n\n\
         def half(n: int) -> Union[int, None]:\n    if n % 2:\n        return None\n    return n // 2\n",
    )
    .unwrap();
    let report = build(&source, &dir.join("opt"), &["--ext", "--non-strict", "--release"]);
    assert!(!report.contains("runs in CPython"), "{report}");
    let probe = "import opt\n\
                 c = opt.Cursor('a-b-c')\n\
                 print(c.find('-'), c.find('-'), c.find('-'), c.pos)\n\
                 print(c.word(), opt.Cursor('').word(), opt.half(8), opt.half(7))\n\
                 print(type(opt.Cursor.find).__name__ in ('function', 'method_descriptor'))\n";
    let run = python()
        .args(["-c", probe])
        .current_dir(&*dir)
        .output()
        .expect("python should spawn");
    assert_eq!(
        text(&pycc_scratch::lf(&run.stdout)),
        "1 3 None 4\na-b-c None 4 None\nTrue\n",
        "{}",
        text(&pycc_scratch::lf(&run.stderr))
    );
}

#[test]
fn class_of_self_and_declarations_native() {
    native("v08_class_of_self", &[]);
}

#[test]
fn class_of_self_and_declarations_ext() {
    ext("v08_class_of_self", &["--release"]);
}

/// A package built one extension per module (`--foreign-relative-imports`):
/// a container attribute of a compiled class that another module of the
/// package uses is a CPython object both modules share (an `append` or a
/// store in one is seen by the other, as in CPython), while the class stays
/// a native type with compiled methods.
#[test]
fn container_attributes_another_module_uses_are_shared() {
    let dir = ScratchDir::new("v08_external_reads").expect("scratch");
    let pkg = dir.join("pkg");
    std::fs::create_dir(&pkg).unwrap();
    std::fs::write(pkg.join("__init__.py"), "").unwrap();
    std::fs::write(
        pkg.join("a.py"),
        "from typing import Dict, List, Tuple\n\n\n\
         class S:\n    def __init__(self) -> None:\n        \
         self.d: Dict[int, Tuple[str, int]] = {}\n        self.xs = [1, 2]\n        \
         self.own = [7]\n        self.n = 0\n\n    \
         def put(self, k: int) -> int:\n        self.d[k] = ('a', k)\n        \
         self.xs.append(k)\n        self.own.append(k)\n        self.n += 1\n        \
         return len(self.d) + len(self.own)\n",
    )
    .unwrap();
    std::fs::write(
        pkg.join("b.py"),
        "def use(s):\n    s.d[5] = ('b', 5)\n    s.xs.append(9)\n    return len(s.d), list(s.xs), s.n\n",
    )
    .unwrap();
    let probe = "from pkg import a, b\n\
                 s = a.S()\n\
                 print(s.put(1), b.use(s), s.put(2))\n\
                 print(sorted(s.d.items()), s.xs, s.n)\n\
                 print(type(a.S.put).__name__)\n";
    let oracle = python()
        .args(["-c", probe])
        .current_dir(&*dir)
        .output()
        .expect("python should spawn");
    for module in ["a", "b"] {
        let source = pkg.join(format!("{module}.py"));
        build(
            &source,
            &pkg.join(module),
            &["--ext", "--non-strict", "--release", "--foreign-relative-imports"],
        );
    }
    for module in ["a", "b"] {
        std::fs::remove_file(pkg.join(format!("{module}.py"))).unwrap();
    }
    let ours = python()
        .args(["-c", probe])
        .current_dir(&*dir)
        .output()
        .expect("python should spawn");
    let expected = text(&pycc_scratch::lf(&oracle.stdout)).replace("\nfunction\n", "\nmethod_descriptor\n");
    assert_eq!(text(&pycc_scratch::lf(&ours.stdout)), expected, "{}", text(&pycc_scratch::lf(&ours.stderr)));
}
