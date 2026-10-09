//! End-to-end proof for Part 1 of #884 ([#1125](https://github.com/rotnov/pycc/issues/1125)):
//! a call that passes its arguments by keyword compiles and runs to exactly
//! the same output as its positional twin, and a keyword call CPython itself
//! would reject fails `pycc check` with a spanned `T0021`.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(source.as_bytes()).unwrap();
    path
}

/// Builds `source` and returns the executable's stdout.
fn build_and_run(dir: &ScratchDir, name: &str, source: &str) -> String {
    let src = write_fixture(dir, &format!("{name}.py"), source);
    let out = dir.join(name);
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success(), "`pycc build` failed for {name}");
    let output = Command::new(&out).output().unwrap();
    assert!(
        output.status.success(),
        "the built program failed for {name}"
    );
    String::from_utf8(output.stdout).unwrap()
}

const BODY: &str = "def describe(width: int, height: int) -> None:\n    print(width)\n    print(height)\n    print(width * height)\n\n";

#[test]
fn a_keyword_call_produces_the_same_program_output_as_its_positional_twin() {
    let dir = ScratchDir::new("e2e_issue_1125").expect("failed to create scratch dir");
    let positional = build_and_run(&dir, "positional", &format!("{BODY}describe(3, 4)\n"));
    let keyword = build_and_run(
        &dir,
        "keyword",
        &format!("{BODY}describe(height=4, width=3)\n"),
    );
    let mixed = build_and_run(&dir, "mixed", &format!("{BODY}describe(3, height=4)\n"));
    assert_eq!(positional, "3\n4\n12\n");
    assert_eq!(keyword, positional);
    assert_eq!(mixed, positional);
}

#[test]
fn an_unexpected_keyword_argument_fails_check_with_t0021() {
    let dir = ScratchDir::new("e2e_issue_1125_bad").expect("failed to create scratch dir");
    let src = write_fixture(&dir, "bad.py", &format!("{BODY}describe(3, depth=4)\n"));
    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let mut rendered = String::from_utf8_lossy(&output.stdout).into_owned();
    rendered.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(
        rendered.contains("T0021")
            && rendered.contains("`describe` got an unexpected keyword argument `depth`"),
        "unexpected diagnostic: {rendered}"
    );
}
