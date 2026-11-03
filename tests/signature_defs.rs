//! v0.12: an `--ext` module's published signature wrapper (#1194,
//! `src/ext_build/signatures.rs`) whose declared defaults are constants is
//! tabled as the wrapper function's own code object, and importing the
//! module makes the wrapper directly, its closure bound as the factory
//! `_pycc_make` binds it, instead of running the factory's chunk and calling
//! the factory. Everything Python sees of the wrapper is as before, and so
//! the source's: name, qualified name, module, defaults, keyword defaults
//! (a dict of its own per function), the parameters `inspect` reads, the
//! arguments bound and the `TypeError` of a call that cannot bind, and the
//! traceback of an exception the compiled body raises. A wrapper with a
//! default computed at run time still runs its factory, and so does one
//! that guards a scalar parameter's type with a fallback to the source
//! (`--non-strict`'s twin, a second closure the factory makes). (What the
//! wrappers leave out, their annotations among it, is in
//! `docs/DEVIATIONS.md`.)
//!
//! The oracle is `python3` (or `PYCC_PYTHON`) importing the source itself.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&pycc_scratch::lf(bytes)).replace("\r\n", "\n")
}

const MODULE: &str = r#"LIMIT = 3


def add(a: int, b: int = 10) -> int:
    return a + b


def scale(x, *, factor=2.0, label="s"):
    return label + str(x * factor)


def total(*nums):
    return sum(nums)


def tags(name, sep="-", **extra):
    out = name
    for k in extra:
        out += sep + k + "=" + extra[k]
    return out


def mix(a, /, b, *rest, c, d=4, **kw):
    return f"{a} {b} {rest} {c} {d} {sorted(kw.items())}"


def computed(a, b=LIMIT):
    return a * b


def fails(n, d=0):
    return n // d


class Box:
    def __init__(self, w=1, h=2):
        self.w = w
        self.h = h

    def area(self, *, scale=1):
        return self.w * self.h * scale

    def grow(self, *amounts, by=0):
        return self.w + sum(amounts) + by
"#;

const DRIVER: &str = r#"
import inspect, sys, traceback
made = []
sys.setprofile(lambda frame, event, arg: made.append(1) if event == "call" and frame.f_code.co_name == "_pycc_make" else None)
import m
sys.setprofile(None)
print("factories run at import:", len(made), file=sys.stderr)
for name in ("add", "scale", "total", "tags", "mix", "computed", "fails"):
    f = getattr(m, name)
    print(name, f.__name__, f.__qualname__, f.__module__, f.__kwdefaults__, type(f).__name__,
          [(p.name, p.kind.name, p.default) for p in inspect.signature(f).parameters.values()]
          if name != "computed" else f.__defaults__ is not None)
for name in ("area", "grow"):
    f = getattr(m.Box, name)
    print(name, f.__name__, f.__qualname__)
print(m.scale.__kwdefaults__ is m.mix.__kwdefaults__, m.scale.__kwdefaults__ == {"factor": 2.0, "label": "s"})
m.scale.__kwdefaults__["label"] = "t"
calls = [
    lambda: m.add(1), lambda: m.add(1, 2), lambda: m.add(b=5, a=6),
    lambda: m.scale(3.0), lambda: m.scale(3.0, factor=0.5),
    lambda: m.total(), lambda: m.total(*range(5)),
    lambda: m.tags("n"), lambda: m.tags("n", "+", k="v", z="w"),
    lambda: m.mix(1, 2, 3, 4, c=5), lambda: m.mix(1, b=2, c=3, d=0, e=9),
    lambda: m.computed(2), lambda: m.Box().area(), lambda: m.Box(3, 4).area(scale=2),
    lambda: m.Box(w=2).grow(1, 2, by=3),
    lambda: m.add(), lambda: m.add(1, 2, 3), lambda: m.add(1, c=2), lambda: m.scale(1.0, 2.0),
    lambda: m.mix(1, 2), lambda: m.mix(a=1, b=2, c=3), lambda: m.Box().area(3),
]
for call in calls:
    try:
        print(call())
    except TypeError as e:
        print("TypeError:", e)
try:
    m.fails(1)
except ZeroDivisionError as e:
    print("ZeroDivisionError:", e)
    for frame in traceback.extract_tb(sys.exc_info()[2]):
        print(frame.name, frame.filename.endswith(("m.py", "<string>")))
"#;

fn import(dir: &Path) -> Output {
    python()
        .args(["-B", "-c", DRIVER])
        .current_dir(dir)
        .output()
        .expect("python should spawn")
}

/// Builds the module as an `--ext` module, imports it and the source, and
/// checks the same stdout; returns the compiled side's count of factories
/// run at import.
fn check(label: &str) -> String {
    let compiled = ScratchDir::new(label).expect("scratch");
    let path = compiled.join("m.py");
    std::fs::write(&path, MODULE).expect("write the module");
    let build = pycc()
        .args(["build", "--ext", "--non-strict"])
        .arg(&path)
        .arg("-o")
        .arg(compiled.join("m"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    std::fs::remove_file(&path).expect("import the artifact, not the source");
    let plain = ScratchDir::new(&format!("{label}_src")).expect("scratch");
    std::fs::write(plain.join("m.py"), MODULE).expect("write the module");
    let (ours, oracle) = (import(&compiled), import(&plain));
    assert_eq!(text(&ours.stdout), text(&oracle.stdout), "{}", text(&ours.stderr));
    assert_eq!(ours.status.code(), Some(0), "{}", text(&ours.stderr));
    assert_eq!(oracle.status.code(), Some(0), "{}", text(&oracle.stderr));
    text(&ours.stderr).lines().next().unwrap_or_default().to_string()
}

#[test]
fn constant_default_wrappers_are_made_without_their_factories() {
    // Every wrapper but `computed`'s, whose default reads `LIMIT` when its
    // factory runs, and `add`'s, whose `int` parameters fall back to the
    // source through a twin, is made directly.
    assert_eq!(check("signature_defs"), "factories run at import: 2");
}
