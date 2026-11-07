//! A selected function whose comprehension holds a walrus (`[y for x in xs
//! if (y := f(x))]`, pydantic's `_fields.py` and `networks.py`): the walrus
//! binds its target in the function (PEP 572), so the name is the
//! function's local, not a module global read through the namespace. The
//! `--ext --non-strict --select all` build succeeds, the function stays
//! bytecode with its diagnostic, and the module prints what CPython prints.
//! Before, the build rewrote the target into a namespace subscript and
//! failed to parse its own output.

use pycc_scratch::ScratchDir;
use std::process::Command;

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

const SOURCE: &str = "\
def f(xs):
    return [y for x in xs if (y := x) is not None]


class C:
    def g(self, xs):
        return {x: v for x in xs for w in [x] if (v := w * 2)}


def plain(xs):
    return [x * 2 for x in xs]


def main():
    print(f([1, None, 2]), C().g([0, 1, 2]), plain([3]))
";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_walrus_in_a_comprehension_keeps_the_build_and_cpython_results() {
    let dir = ScratchDir::new("walrus_in_comprehension_ext").expect("scratch");
    std::fs::write(dir.join("m.py"), SOURCE).expect("write");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--non-strict", "--select", "all", "--report-fallbacks"])
        .arg(dir.join("m.py"))
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    assert!(!report.contains("__pycc_fallback_ns"), "{report}");
    for name in ["f", "C.g"] {
        assert!(report.contains(&format!("not compiled: m:{name}: C0001")), "{name}\n{report}");
    }
    assert!(report.contains("compiled: m:plain\n"), "{report}");
    let run = || {
        python()
            .args(["-c", "import m\nm.main()\n"])
            .current_dir(&*dir)
            .output()
            .expect("python3 should spawn")
    };
    let ours = run();
    assert!(ours.status.success(), "{}", text(&ours.stderr));
    assert_eq!(text(&ours.stdout), "[1, 2] {1: 2, 2: 4} [6]\n");
}
