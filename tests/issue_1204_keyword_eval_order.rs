//! End-to-end proof for [#1204](https://github.com/rotnov/pycc/issues/1204):
//! a keyword call is bound only when binding cannot change the order its
//! argument values are evaluated in (`docs/TYPE_SYSTEM.md`, "Keyword argument
//! evaluation order"). A call that would reorder an observable value keeps
//! the unchanged `C0001` keyword-argument rejection; every other keyword call
//! runs to CPython's own output.

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

/// `g` announces each value it produces, so the output records the order
/// argument values were evaluated in.
const BODY: &str = "def g(n: int) -> int:\n    print(n)\n    return n\n\ndef f(a: int, b: int, c: int = 30) -> None:\n    print(a + b + c)\n\n";

#[test]
fn an_out_of_order_keyword_call_with_side_effects_keeps_the_capability_rejection() {
    let dir = ScratchDir::new("e2e_issue_1204_rejected").expect("failed to create scratch dir");
    let src = write_fixture(&dir, "bad.py", &format!("{BODY}f(b=g(2), a=g(1))\n"));
    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let mut rendered = String::from_utf8_lossy(&output.stdout).into_owned();
    rendered.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(
        rendered.contains("error[C0001]: keyword call arguments are not supported yet")
            && rendered.contains("bad.py:8:1"),
        "unexpected diagnostic: {rendered}"
    );
    assert!(
        !rendered.contains("T0021"),
        "unexpected diagnostic: {rendered}"
    );
}

#[test]
fn an_out_of_order_keyword_call_of_literals_and_names_still_binds() {
    let dir = ScratchDir::new("e2e_issue_1204_pure").expect("failed to create scratch dir");
    // CPython prints 21 and then 33.
    let output = build_and_run(
        &dir,
        "pure",
        &format!("{BODY}x = 1\nf(c=18, b=2, a=x)\nf(b=2, a=x)\n"),
    );
    assert_eq!(output, "21\n33\n");
}

#[test]
fn an_in_order_keyword_call_with_side_effects_evaluates_in_source_order() {
    let dir = ScratchDir::new("e2e_issue_1204_in_order").expect("failed to create scratch dir");
    // CPython prints 1, 2, 3 and then 6.
    let output = build_and_run(
        &dir,
        "in_order",
        &format!("{BODY}f(a=g(1), b=g(2), c=g(3))\n"),
    );
    assert_eq!(output, "1\n2\n3\n6\n");
}

#[test]
fn a_mixed_positional_and_keyword_call_with_side_effects_binds_in_order() {
    let dir = ScratchDir::new("e2e_issue_1204_mixed").expect("failed to create scratch dir");
    // CPython prints 1, 2, 3 and then 6.
    let output = build_and_run(&dir, "mixed", &format!("{BODY}f(g(1), b=g(2), c=g(3))\n"));
    assert_eq!(output, "1\n2\n3\n6\n");
}

#[test]
fn a_default_fills_after_an_in_order_call_with_side_effects() {
    let dir = ScratchDir::new("e2e_issue_1204_default").expect("failed to create scratch dir");
    // CPython prints 1, 2 and then 33: `c` is filled from its default.
    let output = build_and_run(&dir, "default", &format!("{BODY}f(a=g(1), b=g(2))\n"));
    assert_eq!(output, "1\n2\n33\n");
}
