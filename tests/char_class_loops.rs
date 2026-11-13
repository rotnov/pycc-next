//! Character-class loops (docs/design/selected-regions.md, R4;
//! `tests/fixtures/char_class_loops.py`): `while s[pos] in chars` and
//! `not in`, tested without a character object (`crates/pycc_codegen/src/
//! char_class.rs`), and prepared once before the loop when `s` and `chars`
//! are parameters the loop only reads, in region copies (a native position)
//! and in their generic lifts (an `object` position). With the inline fast
//! paths on (`pycc_ext_fast` published) and off (`PYCC_NO_FAST_OPS=1`) the
//! module prints what CPython prints for its source: non-ASCII text, a `str`
//! subclass, positions past the end, below zero, too big for an index or a
//! `bool`, a `set`, a `frozenset` subclass, sets that are too large or hold a
//! non-`str` member, a set rebound or grown in the loop, and a mask evicted
//! while the loop runs take CPython's own operations.

use pycc_scratch::ScratchDir;
use std::process::{Command, Output};

const SELECTED: [&str; 8] = ["skip", "until", "churn", "rebound", "grown", "local", "scan", "run"];

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(o: &Output) -> String {
    // Line endings as on POSIX: Windows' Python writes `\r\n`.
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).replace('\r', "")
}

/// The LLVM IR of the function of `ir` whose symbol starts with `prefix`
/// and ends with `_<name>__`.
fn function<'a>(ir: &'a str, prefix: &str, name: &str) -> &'a str {
    let suffix = format!("_{name}__(");
    let start = ir
        .match_indices("\ndefine ")
        .map(|(at, _)| at)
        .find(|&at| {
            let line = ir[at + 1..].lines().next().unwrap_or("");
            line.contains(&format!("@{prefix}")) && line.contains(&suffix)
        })
        .unwrap_or_else(|| panic!("{prefix}..{name}"));
    let end = ir[start..].find("\n}\n").expect("the definition ends") + start;
    &ir[start..end]
}

#[test]
fn character_class_loops_are_prepared_once_and_match_cpython() {
    let scratch = ScratchDir::new("char_class_loops").unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/char_class_loops.py");
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
    for name in ["skip", "until", "churn", "rebound", "grown"] {
        assert!(
            report.contains(&format!("specialized: `region {name}` runs natively when called with pos: int")),
            "{name}: {report}"
        );
    }
    // A loop over parameters it only reads is prepared before it, in the
    // copy and in the lift; a loop that rebinds its set tests each
    // character on its own.
    let ir = std::fs::read_to_string(scratch.join("m.ll.pre.ll"))
        .expect("the IR before optimization")
        .replace('\r', "");
    for prefix in ["pyfn___pycc_rnative_lift", "pyfn___pycc_lift"] {
        for name in ["skip", "until", "churn", "grown"] {
            let body = function(&ir, prefix, name);
            assert!(body.contains("char_class_loop_slow"), "{prefix} {name}: {body}");
            assert!(body.contains("char_class_prepared_slow"), "{prefix} {name}: {body}");
        }
        let rebound = function(&ir, prefix, "rebound");
        assert!(!rebound.contains("char_class_loop"), "{prefix}: {rebound}");
        assert!(rebound.contains("char_class_item_slow"), "{prefix}: {rebound}");
    }

    std::fs::remove_file(scratch.join("m.py")).unwrap();
    let oracle = python()
        .args(["-I", "-c", "import runpy, sys; runpy.run_path(sys.argv[1])['main']()"])
        .arg(&source)
        .output()
        .expect("python3 runs");
    assert!(oracle.status.success(), "{}", text(&oracle));
    let expected = text(&oracle);
    // The cases the fast paths must hand to CPython are reached.
    assert!(expected.contains("IndexError cannot fit 'int' into an index-sized integer"), "{expected}");
    assert!(expected.contains("IndexError string index out of range"), "{expected}");
    assert!(expected.contains("ab\tc  é# -3 [-2, -3, -3, 8, "), "{expected}");
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
