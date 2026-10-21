//! Regression tests for runtime memory-safety and hash-flooding defects in
//! the generic containers (`pycc_rt::{set, dict, str_hash}`):
//!
//! - an in-place set operator (`a &= b`) must keep `a`'s identity -- its
//!   reference count, kind and marker -- so an alias of `a` stays valid;
//! - `==` between containers of class instances is refused at compile time
//!   (the runtime compares by value and an instance set is a different
//!   object);
//! - freeing a long chain of dicts runs in constant stack;
//! - `str` hashes are CPython's keyed SipHash-1-3, so under one
//!   `PYTHONHASHSEED` a set of `str` iterates exactly as CPython's does.
//!
//! The oracle is CPython itself, run on the same file.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn build(dir: &ScratchDir, source: &Path) -> PathBuf {
    let out = dir.join("prog");
    let build = pycc()
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(&out)
        .arg("--release")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        text(&pycc_scratch::lf(&build.stdout)),
        text(&pycc_scratch::lf(&build.stderr))
    );
    out
}

fn assert_same(ours: &Output, theirs: &Output) {
    assert_eq!(text(&pycc_scratch::lf(&ours.stdout)), text(&pycc_scratch::lf(&theirs.stdout)), "stdout differs");
    assert_eq!(ours.status.code(), theirs.status.code(), "{}", text(&pycc_scratch::lf(&ours.stderr)));
}

/// Builds fixture `name`, runs it and CPython with `seed` as
/// `PYTHONHASHSEED`, and compares.
fn matches_cpython_with_seed(name: &str, seed: &str) {
    let dir = ScratchDir::new("runtime_hardening").expect("scratch");
    let prog = build(&dir, &fixture(name));
    let ours = Command::new(&prog)
        .env("PYTHONHASHSEED", seed)
        .output()
        .expect("the program should run");
    let theirs = python()
        .arg(fixture(name))
        .env("PYTHONHASHSEED", seed)
        .output()
        .expect("python3 should spawn");
    assert_same(&ours, &theirs);
}

#[test]
fn in_place_set_operators_keep_aliases_valid() {
    matches_cpython_with_seed("set_inplace_alias.py", "0");
}

#[test]
fn a_long_chain_of_dicts_is_freed_in_constant_stack() {
    matches_cpython_with_seed("deep_container_chain.py", "0");
}

#[test]
fn str_sets_iterate_in_cpython_order_under_the_same_hash_seed() {
    for seed in ["0", "1", "4242", "4294967295"] {
        matches_cpython_with_seed("str_set_order.py", seed);
    }
}

/// Without `PYTHONHASHSEED` the key is random per process, as in CPython:
/// two runs of the same program disagree on a 12-element `str` set's order
/// at least once in a few tries.
#[test]
fn str_hashes_are_randomized_per_process_by_default() {
    let dir = ScratchDir::new("runtime_hardening").expect("scratch");
    let prog = build(&dir, &fixture("str_set_order.py"));
    let run = || {
        let out = Command::new(&prog)
            .env_remove("PYTHONHASHSEED")
            .output()
            .expect("the program should run");
        assert!(out.status.success());
        text(&pycc_scratch::lf(&out.stdout))
    };
    let first = run();
    assert!(
        (0..8).any(|_| run() != first),
        "eight unseeded runs all iterated a str set in one order"
    );
}

#[test]
fn comparing_containers_of_instances_is_a_compile_error() {
    let dir = ScratchDir::new("runtime_hardening").expect("scratch");
    for (name, body) in [
        ("sets", "a: set[P] = {p}\n    b: set[P] = {p}\n    print(a == b)"),
        ("lists", "a: list[P] = [p]\n    b: list[P] = [p]\n    print(a != b)"),
        (
            "dict_values",
            "a: dict[str, P] = {\"k\": p}\n    b: dict[str, P] = {\"k\": p}\n    print(a == b)",
        ),
    ] {
        let source = dir.join(format!("{name}.py"));
        std::fs::write(
            &source,
            format!(
                "class P:\n    def __init__(self, x: int) -> None:\n        self.x = x\n\n\n\
                 def main() -> None:\n    p = P(1)\n    {body}\n\n\nmain()\n"
            ),
        )
        .expect("write the source");
        let out = pycc()
            .arg("build")
            .arg(&source)
            .arg("-o")
            .arg(dir.join(name))
            .output()
            .expect("pycc should spawn");
        let stdout = text(&pycc_scratch::lf(&out.stdout)) + &text(&pycc_scratch::lf(&out.stderr));
        assert!(!out.status.success(), "{name}: {stdout}");
        assert!(
            stdout.contains("error[T0021]")
                && stdout.contains("comparing containers of class instances"),
            "{name}: {stdout}"
        );
    }
}
