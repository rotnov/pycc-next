//! A module's `frozenset` constants stay native in `--ext --non-strict`
//! mode (tomli's `TOML_WS = frozenset(" \t")`).
//!
//! The non-strict container widening makes a name holding a native
//! container an object when the container meets a CPython object, so that
//! no copy breaks aliasing. A `frozenset` of scalars crosses as an
//! immutable copy, which aliasing cannot tell apart, so it is never widened:
//! the module's calls of its `Iterable[str]` function keep the native
//! `frozenset[str]` specialization (`pycc_ast::desugar::specialize`), and
//! `in` on the constant stays native. Widened, every such call took the
//! object path, and tomli's `loads()` went from 2.53M to 2.95M instructions
//! per call (`bench/package_check.py --instructions`).

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
fn frozenset_constants_take_the_native_specialization() {
    let dir = ScratchDir::new("frozenset_constants_native").expect("scratch");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/frozenset_constants_native.py");
    let (ext, src) = (dir.join("ext"), dir.join("src"));
    std::fs::create_dir_all(&ext).expect("dir");
    std::fs::create_dir_all(&src).expect("dir");
    std::fs::copy(&fixture, dir.join("m.py")).expect("source");
    std::fs::copy(&fixture, src.join("m.py")).expect("source");
    let ll = dir.join("m.ll");
    let out = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--non-strict", "--release", "--report-fallbacks"])
        .arg(dir.join("m.py"))
        .arg("-o")
        .arg(ext.join("m"))
        .env("PYCC_DUMP_LLVM", &ll)
        .output()
        .expect("pycc should spawn");
    let report = text(&out.stderr);
    assert!(out.status.success(), "{report}");
    assert!(
        report.contains("specialized: `skip_chars` runs natively when called with chars: frozenset[str]"),
        "{report}"
    );
    // `words` passes the constants as native frozensets: its three calls
    // are of the specialization, none of the object-taking original.
    let mut pre = ll.clone().into_os_string();
    pre.push(".pre.ll");
    let ir = std::fs::read_to_string(pre).expect("ir");
    let words = ir
        .split("\ndefine ")
        .find(|f| f.lines().next().is_some_and(|l| l.contains("@pyfn_words(")))
        .expect("`words`");
    assert_eq!(words.matches("call i64 @pyfn__pycc_native_skip_chars(").count(), 3, "{words}");
    assert!(!words.contains("call i64 @pyfn_skip_chars("), "{words}");
    assert!(!words.contains("@pycc_ext_obj_retain"), "{words}");
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
    assert!(oracle.status.success(), "{}", text(&oracle.stderr));
    assert!(ours.status.success(), "{}", text(&ours.stderr));
    assert_eq!(text(&ours.stdout), text(&oracle.stdout));
}
