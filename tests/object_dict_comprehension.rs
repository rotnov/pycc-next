//! Non-strict mode (v0.6 accept 5): a dict comprehension whose key or value
//! is a CPython object (an empty list display nothing types, a `sys.argv`
//! item), or that iterates a CPython object, builds the CPython `dict`
//! natively instead of running the statement in CPython. Compared with the
//! live CPython oracle; `--report-fallbacks` proves no statement falls back.

#[path = "stdlib_support/mod.rs"]
mod support;

#[test]
fn object_dict_comprehensions_match_cpython_without_fallback() {
    let source = support::fixture("object_dict_comprehension.py");
    let expected = support::outcome(
        &std::process::Command::new(support::python())
            .arg(&source)
            .output()
            .expect("the oracle runs"),
    );
    let dir = pycc_scratch::ScratchDir::new("object_dict_comprehension").expect("scratch");
    let out = dir.join("prog");
    let build = support::pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&out)
        .args(["--release", "--non-strict", "--report-fallbacks"])
        .output()
        .expect("pycc spawns");
    assert!(build.status.success(), "{}", support::rendered(&build));
    let stderr = pycc_scratch::lf(&*String::from_utf8_lossy(&build.stderr));
    assert!(!stderr.contains("fallback:"), "{stderr}");
    let run = std::process::Command::new(&out).output().expect("the program spawns");
    assert_eq!(support::outcome(&run), expected);
}
