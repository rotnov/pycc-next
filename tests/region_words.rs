//! Native `int` positions in the `str` word paths of region copies
//! (docs/design/selected-regions.md, R3; `tests/fixtures/region_words.py`):
//! a copy's `s[a:b]`, `s.startswith(t, a)` and `c = s[pos]` read the
//! position's word, with the inline fast paths on (`pycc_ext_fast`
//! published) and off (`PYCC_NO_FAST_OPS=1`), and the module prints what
//! CPython prints for its source: big `int` positions, a `str` subclass,
//! non-ASCII text and indexes past the end take CPython's own operation.

use pycc_scratch::ScratchDir;
use std::process::{Command, Output};

const SELECTED: [&str; 8] = ["skip", "kind", "cuts", "starts", "beyond", "edges", "step", "run"];

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(o: &Output) -> String {
    // Line endings as on POSIX: Windows' Python writes `\r\n`.
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).replace('\r', "")
}

/// The LLVM IR of the region copy of `name` in `ir`: the definition of
/// `@pyfn___pycc_rnative_liftN_<name>__`.
fn copy<'a>(ir: &'a str, name: &str) -> &'a str {
    let suffix = format!("_{name}__(");
    let start = ir
        .match_indices("\ndefine ")
        .map(|(at, _)| at)
        .find(|&at| {
            let line = ir[at + 1..].lines().next().unwrap_or("");
            line.contains("@pyfn___pycc_rnative_lift") && line.contains(&suffix)
        })
        .unwrap_or_else(|| panic!("a copy of {name}"));
    let end = ir[start..].find("\n}\n").expect("the definition ends") + start;
    &ir[start..end]
}

#[test]
fn region_copies_read_native_positions_as_words_and_match_cpython() {
    let scratch = ScratchDir::new("region_words").unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/region_words.py");
    std::fs::copy(&source, scratch.join("m.py")).unwrap();
    let ir_path = scratch.join("m.ll");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pycc"));
    cmd.arg("build")
        .arg(scratch.join("m.py"))
        .arg("-o")
        .arg(scratch.join("m"))
        .args(["--ext", "--non-strict", "--report-fallbacks", "--select", "none"])
        .env("PYCC_DUMP_LLVM", &ir_path);
    for name in SELECTED {
        cmd.args(["--compile", &format!("m:{name}")]);
    }
    let build = cmd.output().expect("pycc runs");
    let report = text(&build);
    assert!(build.status.success(), "{report}");
    for name in ["skip", "kind", "cuts", "starts", "beyond", "edges", "step"] {
        assert!(
            report.contains(&format!("specialized: `region {name}` runs natively when called with pos: int")),
            "{name}: {report}"
        );
    }
    // The word paths are taken where the position is native: the copies
    // slice an exact `str` without a `slice` or a boxed bound, compare
    // `startswith`'s bytes inline, and load `kind`'s character as a word.
    // Line endings as on POSIX: on Windows the dump ends its lines with `\r\n`.
    let ir = std::fs::read_to_string(scratch.join("m.ll.pre.ll"))
        .expect("the IR before optimization")
        .replace('\r', "");
    let cuts = copy(&ir, "cuts");
    assert_eq!(cuts.matches("call ptr @pycc_ext_str_slice(").count(), 10, "{cuts}");
    let starts = copy(&ir, "starts");
    assert!(starts.contains("str_startswith_compare"), "{starts}");
    let kind = copy(&ir, "kind");
    assert!(kind.contains("tagged_char_word"), "{kind}");

    std::fs::remove_file(scratch.join("m.py")).unwrap();
    let oracle = python()
        .args(["-I", "-c", "import runpy, sys; runpy.run_path(sys.argv[1])['main']()"])
        .arg(&source)
        .output()
        .expect("python3 runs");
    assert!(oracle.status.success(), "{}", text(&oracle));
    let expected = text(&oracle);
    // The cases the word paths must hand to CPython are reached.
    assert!(expected.contains("(5, 'comment', ['#x', '#x', 'ab ab', ' ab', 'b#', ' ab#x', '#x', '', '<#x>', '']"), "{expected}");
    assert!(expected.contains("IndexError cannot fit 'int' into an index-sized integer"), "{expected}");
    assert!(expected.contains("(1, 'other \u{e9}'"), "{expected}");
    let probe = "import sys; sys.path.insert(0, sys.argv[1]); import m; m.main()";
    for off in [false, true] {
        let mut run = python();
        run.args(["-I", "-c", probe]).arg(&*scratch);
        if off {
            run.env("PYCC_NO_FAST_OPS", "1");
        } else {
            run.env_remove("PYCC_NO_FAST_OPS");
        }
        let ours = run.output().expect("python3 runs");
        assert!(ours.status.success(), "fast paths off: {off}\n{}", text(&ours));
        assert_eq!(text(&ours), expected, "fast paths off: {off}");
    }
}
