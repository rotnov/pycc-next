//! A module's `frozenset` constants read by compiled functions of an
//! `--ext --non-strict` module (tomli's `TOML_WS = frozenset(" \t")`,
//! passed to an `Iterable[str]` parameter and tested with `in`): every
//! function compiles, and the module prints what CPython prints for its
//! source, `in` on a list or a `str` passed for the same parameter
//! included.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn python() -> std::ffi::OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into())
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle and Python.h (PYCC_PYTHON, default python3)"]
fn frozenset_constants_match_cpython() {
    let dir = ScratchDir::new("frozenset_constants_native").expect("scratch");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/frozenset_constants_native.py");
    let (ext, src) = (dir.join("ext"), dir.join("src"));
    std::fs::create_dir_all(&ext).expect("dir");
    std::fs::create_dir_all(&src).expect("dir");
    std::fs::copy(&fixture, dir.join("m.py")).expect("source");
    std::fs::copy(&fixture, src.join("m.py")).expect("source");
    let out = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--non-strict", "--select", "all", "--release", "--report-fallbacks"])
        .arg(dir.join("m.py"))
        .arg("-o")
        .arg(ext.join("m"))
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&out.stderr));
    assert!(out.status.success(), "{report}");
    for name in ["skip_chars", "words", "first", "codes"] {
        assert!(report.contains(&format!("compiled: m:{name}\n")), "{name}\n{report}");
    }
    let script = "import m\n\
                  print(m.words('  ab cd\\tef '), m.first('   x', 0), m.first('x', 5), m.codes('az'))\n\
                  print(m.skip_chars('aab', 0, ['a']), m.skip_chars('aab', 0, 'ab'), sorted(m.WS), type(m.WORD).__name__)\n";
    let run = |d: &Path| {
        Command::new(python())
            .args(["-c", script])
            .current_dir(d)
            .output()
            .expect("python")
    };
    let (ours, oracle) = (run(&ext), run(&src));
    assert!(oracle.status.success(), "{}", text(&pycc_scratch::lf(&oracle.stderr)));
    assert!(ours.status.success(), "{}", text(&pycc_scratch::lf(&ours.stderr)));
    assert_eq!(text(&pycc_scratch::lf(&ours.stdout)), text(&pycc_scratch::lf(&oracle.stdout)));
}
