//! Part 2 of #1028 (#1224): the D-128 interop policies. `--interop-policy`,
//! `--pure` and the `[interop]` table of `pycc.toml` decide which
//! CPython-backed imports a native build admits, and a rejected import is
//! `I0402`.
//!
//! Every assertion holds on every host. Every admitted root here is in the
//! standard library, which a Windows host embeds too since #1286 (D-253);
//! a root outside it is embedded from `pycc.lock` there too since #1296.
//! An admission is observed without an interpreter: with `PYCC_PYTHON` naming
//! a missing file, an admitted `build` or `run` reaches the embedding step
//! and stops with an environment failure (exit 2) that names
//! `PYCC_PYTHON`, the technique `tests/issue_1223_embedded_executable.rs`
//! uses.
//!
//! Each test works in its own scratch directory holding its own
//! `pycc.toml`, and runs pycc from that directory with relative paths, so
//! a message names the manifest as `pycc.toml`. The upward manifest search
//! assumes no stray `pycc.toml` above the scratch root; none is tracked or
//! expected there.

use pycc_scratch::ScratchDir;
use std::process::{Command, Output};

const MISSING_PYTHON: &str = "/nonexistent/pycc-no-python";

/// A `pycc.toml` with a valid `[project]` and `extra` appended.
fn manifest(extra: &str) -> String {
    format!("[project]\nname = \"p\"\nentry = \"main.py\"\npython = \"3.14\"\n\n{extra}")
}

/// A scratch project: `main.py` holding `body`, and `pycc.toml` holding
/// `toml` when given.
fn project(category: &str, toml: Option<&str>, body: &str) -> ScratchDir {
    let dir = ScratchDir::new(category).expect("scratch");
    if let Some(toml) = toml {
        std::fs::write(dir.join("pycc.toml"), toml).expect("write pycc.toml");
    }
    std::fs::write(dir.join("main.py"), body).expect("write main.py");
    dir
}

/// Runs pycc in `dir` with `PYCC_PYTHON` naming a missing interpreter, so
/// an admitted CPython import can never start a real embedding.
fn pycc(dir: &ScratchDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(args)
        .current_dir(&**dir)
        .env("PYCC_PYTHON", MISSING_PYTHON)
        .output()
        .expect("pycc should spawn")
}

fn check(dir: &ScratchDir, flags: &[&str]) -> Output {
    let mut args = vec!["check", "main.py"];
    args.extend_from_slice(flags);
    pycc(dir, &args)
}

fn build(dir: &ScratchDir, flags: &[&str]) -> Output {
    let mut args = vec!["build", "main.py", "-o", "app"];
    args.extend_from_slice(flags);
    pycc(dir, &args)
}

fn run(dir: &ScratchDir, flags: &[&str]) -> Output {
    let mut args = vec!["run", "main.py"];
    args.extend_from_slice(flags);
    pycc(dir, &args)
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// Asserts `check` rejects the program with exactly one `I0402` whose
/// message carries `fragment`, and returns the rendered output.
fn assert_check_rejects(dir: &ScratchDir, flags: &[&str], fragment: &str) -> String {
    let output = check(dir, flags);
    let rendered = stdout_of(&output);
    assert_eq!(output.status.code(), Some(1), "{flags:?}: {rendered}");
    assert_eq!(rendered.matches("error[I0402]").count(), 1, "{rendered}");
    assert!(rendered.contains(fragment), "{flags:?}: {rendered}");
    assert!(!rendered.contains("I0403"), "{rendered}");
    rendered
}

/// Asserts a native `build` admitted the program's CPython import: it got
/// past the policy and stopped only for want of an interpreter.
fn assert_build_admits(dir: &ScratchDir, flags: &[&str]) {
    let output = build(dir, flags);
    let rendered = stderr_of(&output);
    assert_eq!(output.status.code(), Some(2), "{flags:?}: {rendered}");
    assert!(rendered.contains("PYCC_PYTHON"), "{rendered}");
    assert!(rendered.contains(MISSING_PYTHON), "{rendered}");
    assert!(!rendered.contains("I0402"), "{rendered}");
    let checked = check(dir, flags);
    assert!(checked.status.success(), "{}", stdout_of(&checked));
}

const JSON: &str = "import json\n\nprint(str(json.dumps(1)))\n";
const PPRINT: &str = "import pprint\n\npprint.pprint(1)\n";
const ALLOW_JSON: &str = "[interop]\npolicy = \"allowlist\"\nallow = [\"json\"]\n";

// ---------------------------------------------------------------------
// Precedence (D-128 rule 2): an explicit CLI policy, then `[interop]`,
// then `auto`.
// ---------------------------------------------------------------------

#[test]
fn an_explicit_policy_overrides_a_configured_allowlist() {
    let dir = project("1224_over_allowlist", Some(&manifest(ALLOW_JSON)), JSON);
    assert_check_rejects(
        &dir,
        &["--interop-policy", "deny"],
        "the `deny` interop policy (set by `--interop-policy deny`) rejects",
    );
    assert_check_rejects(
        &dir,
        &["--pure"],
        "the `deny` interop policy (set by `--pure`) rejects",
    );
}

#[test]
fn a_cli_switch_to_allowlist_from_a_configured_auto_or_deny_rejects_every_root() {
    for configured in [
        "[interop]\npolicy = \"auto\"\n",
        "[interop]\npolicy = \"deny\"\n",
    ] {
        let dir = project("1224_switch_allowlist", Some(&manifest(configured)), JSON);
        assert_check_rejects(
            &dir,
            &["--interop-policy", "allowlist"],
            "(set by `--interop-policy allowlist`) rejects: its root `json` is not in `[interop] allow`",
        );
    }
}

#[test]
fn a_configured_allowlist_rejects_an_unlisted_standard_library_root() {
    let dir = project("1224_unlisted", Some(&manifest(ALLOW_JSON)), PPRINT);
    assert_check_rejects(
        &dir,
        &[],
        "the `allowlist` interop policy (set by `pycc.toml`) rejects: its root `pprint` is not in `[interop] allow`",
    );
    // An explicit `allowlist` over a configured one keeps the configured
    // roots and names the flag as the policy's source.
    assert_check_rejects(
        &dir,
        &["--interop-policy", "allowlist"],
        "(set by `--interop-policy allowlist`) rejects: its root `pprint`",
    );
}

#[test]
fn a_configured_allowlist_admits_a_listed_root_and_an_explicit_policy_can_widen_it() {
    let dir = project("1224_listed", Some(&manifest(ALLOW_JSON)), JSON);
    assert_build_admits(&dir, &[]);
    assert_build_admits(&dir, &["--interop-policy", "allowlist"]);
    let dir = project("1224_widened", Some(&manifest(ALLOW_JSON)), PPRINT);
    assert_build_admits(&dir, &["--interop-policy", "auto"]);
    let deny = manifest("[interop]\npolicy = \"deny\"\n");
    let dir = project("1224_over_deny", Some(&deny), JSON);
    assert_build_admits(&dir, &["--interop-policy", "auto"]);
}

// ---------------------------------------------------------------------
// The same outcome from `check`, `build` and `run`.
// ---------------------------------------------------------------------

#[test]
fn check_build_and_run_report_the_same_i0402() {
    let allowlist = manifest(ALLOW_JSON);
    let deny = manifest("[interop]\npolicy = \"deny\"\n");
    let cases: [(&str, Option<&str>, &str, &[&str]); 4] = [
        ("1224_parity_allowlist", Some(&allowlist), PPRINT, &[]),
        ("1224_parity_deny", Some(&deny), JSON, &[]),
        ("1224_parity_pure", None, JSON, &["--pure"]),
        (
            "1224_parity_flag",
            None,
            PPRINT,
            &["--interop-policy", "allowlist"],
        ),
    ];
    for (category, toml, body, flags) in cases {
        let dir = project(category, toml, body);
        let checked = check(&dir, flags);
        let built = build(&dir, flags);
        let ran = run(&dir, flags);
        for output in [&checked, &built, &ran] {
            assert_eq!(output.status.code(), Some(1), "{category}");
        }
        let expected = stdout_of(&checked);
        assert!(expected.contains("error[I0402]"), "{category}: {expected}");
        assert_eq!(stderr_of(&built), expected, "{category}: build");
        assert_eq!(stderr_of(&ran), expected, "{category}: run");
        assert!(!dir.join("app").exists(), "{category}");
    }
}

/// The only case that exercises a manifest-sourced `deny`: the message
/// names the manifest, as the path pycc was given spells it.
#[test]
fn a_configured_deny_names_the_manifest_path() {
    let deny = manifest("[interop]\npolicy = \"deny\"\n");
    let dir = project("1224_configured_deny", Some(&deny), JSON);
    assert_check_rejects(
        &dir,
        &[],
        "the `deny` interop policy (set by `pycc.toml`) rejects: it admits no CPython import",
    );
    // Run from the parent directory, the manifest is `<dir>/pycc.toml`.
    let parent = dir.parent().expect("a scratch directory has a parent");
    let name = dir.file_name().unwrap().to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["check", &format!("{name}/main.py")])
        .current_dir(parent)
        .output()
        .expect("pycc should spawn");
    let rendered = stdout_of(&output);
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    assert!(
        rendered.contains(&format!("(set by `{name}/pycc.toml`)")),
        "{rendered}"
    );
}

/// A manifest whose `entry` locates no source root still governs the
/// policy: it is recorded before the source-root check.
#[test]
fn a_manifest_whose_entry_finds_no_source_root_still_sets_the_policy() {
    let without_root = "[project]\nname = \"p\"\nentry = \"src/main.py\"\npython = \"3.14\"\n\n\
                        [interop]\npolicy = \"deny\"\n";
    let dir = project("1224_no_source_root", Some(without_root), JSON);
    assert!(!dir.join("src").exists());
    assert_check_rejects(&dir, &[], "(set by `pycc.toml`)");
    let built = build(&dir, &[]);
    assert_eq!(built.status.code(), Some(1), "{}", stderr_of(&built));
    assert!(stderr_of(&built).contains("error[I0402]"));
}

// ---------------------------------------------------------------------
// Evaluation order: the policy is decided before embedding.
// ---------------------------------------------------------------------

/// A `memoryview` parameter keeps its own `I0405`, and a policy-rejected
/// root that no embedding could serve is `I0402`, never `I0403`.
#[test]
fn a_rejected_numpy_import_is_i0402_beside_the_buffer_gate() {
    let body = "import numpy\n\n\ndef f(v: memoryview) -> int:\n    return 1\n";
    let dir = project("1224_buffer", None, body);
    let output = build(&dir, &["--pure"]);
    let rendered = stderr_of(&output);
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    assert!(rendered.contains("error[I0402]"), "{rendered}");
    assert!(rendered.contains("`import numpy`"), "{rendered}");
    assert!(rendered.contains("error[I0405]"), "{rendered}");
    assert!(!rendered.contains("I0403"), "{rendered}");
}

/// `--target` refuses every embedding with `I0403`, but the policy is
/// decided first and host-independently.
#[test]
fn a_target_build_under_deny_is_i0402_rather_than_i0403() {
    let dir = project("1224_target", None, JSON);
    let output = build(
        &dir,
        &[
            "--interop-policy",
            "deny",
            "--target",
            "x86_64-apple-darwin",
        ],
    );
    let rendered = stderr_of(&output);
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    assert!(rendered.contains("error[I0402]"), "{rendered}");
    assert!(!rendered.contains("I0403"), "{rendered}");
}

/// A natively implemented module is not a CPython import: it builds and
/// runs fully native under both `--pure` and `deny`.
#[test]
fn a_native_module_import_stays_native_under_pure_and_deny() {
    for flags in [&["--pure"][..], &["--interop-policy", "deny"]] {
        let dir = project(
            "1224_native",
            None,
            "import math\n\nprint(math.sqrt(4.0))\n",
        );
        let output = build(&dir, flags);
        assert!(output.status.success(), "{flags:?}: {}", stderr_of(&output));
        assert!(!dir.join("app.pycc").exists());
        let exe = dir.join("app");
        let ran = Command::new(&exe).output().expect("the native binary runs");
        assert_eq!(stdout_of(&ran), "2.0\n");
        let via_run = run(&dir, flags);
        assert!(via_run.status.success(), "{}", stderr_of(&via_run));
        assert_eq!(stdout_of(&via_run), "2.0\n");
        #[cfg(not(windows))]
        {
            let nm = Command::new("nm")
                .arg("-u")
                .arg(&exe)
                .output()
                .expect("nm runs");
            assert!(nm.status.success(), "{}", stderr_of(&nm));
            for symbol in String::from_utf8_lossy(&nm.stdout).split_whitespace() {
                assert!(
                    !symbol.trim_start_matches('_').starts_with("Py"),
                    "a native build must not reference CPython: {symbol}"
                );
            }
        }
    }
}

#[test]
fn auto_still_admits_a_standard_library_import_with_no_policy_configured() {
    let dir = project("1224_auto", None, JSON);
    assert_build_admits(&dir, &[]);
    let ran = run(&dir, &[]);
    assert_eq!(ran.status.code(), Some(2), "{}", stderr_of(&ran));
    assert!(stderr_of(&ran).contains("PYCC_PYTHON"));
}

// ---------------------------------------------------------------------
// `[interop]` validation (exit 2, naming the manifest).
// ---------------------------------------------------------------------

/// Asserts `check` and `build` of `import json` beside `toml` both stop
/// with an input error naming `pycc.toml` and carrying `fragment`.
fn assert_invalid_manifest(category: &str, toml: &str, flags: &[&str], fragment: &str) {
    let dir = project(category, Some(toml), JSON);
    for output in [check(&dir, flags), build(&dir, flags)] {
        let rendered = stderr_of(&output);
        assert_eq!(output.status.code(), Some(2), "{category}: {rendered}");
        assert!(rendered.contains("pycc.toml"), "{category}: {rendered}");
        assert!(rendered.contains(fragment), "{category}: {rendered}");
    }
}

#[test]
fn an_invalid_interop_table_is_an_input_error_naming_the_manifest() {
    let cases = [
        ("policy = \"strict\"", "strict"),
        ("policy = 1", "policy"),
        ("allow = \"json\"", "allow"),
        ("allow = [1]", "allow"),
        ("policy = \"allowlist\"\nallow = [\"\"]", "allow"),
        (
            "policy = \"allowlist\"\nallow = [\"json.decoder\"]",
            "json.decoder",
        ),
        ("mode = \"deny\"", "mode"),
        ("policy = \"auto\"\nallow = [\"json\"]", "allow"),
        ("policy = \"deny\"\nallow = [\"json\"]", "allow"),
        ("allow = [\"json\"]", "allow"),
    ];
    for (table, fragment) in cases {
        let toml = manifest(&format!("[interop]\n{table}\n"));
        assert_invalid_manifest("1224_invalid", &toml, &[], fragment);
    }
    // A top-level `interop` key that is not a table. It precedes the first
    // table header, or TOML would place it inside `[project]`.
    let top_level = format!("interop = \"deny\"\n{}", manifest(""));
    assert_invalid_manifest("1224_not_a_table", &top_level, &[], "interop");
}

/// The table is validated even when an explicit flag overrides it.
#[test]
fn an_invalid_interop_table_is_refused_even_under_an_explicit_policy() {
    let toml = manifest("[interop]\npolicy = \"strict\"\n");
    for flags in [&["--interop-policy", "auto"][..], &["--pure"]] {
        assert_invalid_manifest("1224_invalid_flag", &toml, flags, "strict");
    }
}

/// The policy is resolved only for a program with a CPython-backed import:
/// a bad table cannot break a program without one, even one whose project
/// import made the loader discover the manifest.
#[test]
fn the_policy_is_resolved_only_for_a_program_with_a_cpython_import() {
    let toml = manifest("[interop]\npolicy = \"strict\"\n");
    let dir = project("1224_lazy_plain", Some(&toml), "print(1)\n");
    let output = build(&dir, &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));

    let dir = project(
        "1224_lazy_project_import",
        Some(&toml),
        "from dep import g\n\nprint(g())\n",
    );
    std::fs::write(dir.join("dep.py"), "def g() -> int:\n    return 7\n").unwrap();
    let output = build(&dir, &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    let ran = Command::new(dir.join("app")).output().expect("runs");
    assert_eq!(stdout_of(&ran), "7\n");
    let checked = check(&dir, &[]);
    assert!(checked.status.success(), "{}", stdout_of(&checked));

    let dir = project("1224_lazy_foreign", Some(&toml), JSON);
    let output = build(&dir, &[]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
}

/// A manifest the shared parser refuses (no `[project]`) keeps its
/// existing behavior: only a program that makes the loader discover it
/// fails.
#[test]
fn a_manifest_without_a_project_table_fails_only_a_program_that_discovers_it() {
    let toml = "[interop]\npolicy = \"deny\"\n";
    let dir = project("1224_no_project_json", Some(toml), JSON);
    let output = build(&dir, &[]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
    assert!(stderr_of(&output).contains("pycc.toml"));
    let dir = project("1224_no_project_plain", Some(toml), "print(1)\n");
    let output = build(&dir, &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));
}

// ---------------------------------------------------------------------
// `--ext` (D-244 rule 3) and flag conflicts (D-128 rule 2).
// ---------------------------------------------------------------------

/// `--ext` ignores `[interop]`: a bad table does not stop the build, which
/// reaches its own boundary check and refuses the function with `C0003`.
#[test]
fn an_ext_build_ignores_the_interop_table() {
    let toml = manifest("[interop]\npolicy = \"strict\"\n");
    let body = "import json\n\n\ndef greet(who: list[int]) -> list[int]:\n    return who\n";
    let dir = project("1224_ext", Some(&toml), body);
    // No suffix on `-o`: `--ext` appends the one its target calls for.
    let output = pycc(&dir, &["build", "main.py", "-o", "main", "--ext"]);
    let rendered = stderr_of(&output);
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    assert!(rendered.contains("error[C0003]"), "{rendered}");
    assert!(!rendered.contains("pycc.toml"), "{rendered}");
}

#[test]
fn conflicting_interop_flags_are_usage_errors() {
    let dir = project("1224_conflicts", None, JSON);
    for flags in [&["--pure"][..], &["--interop-policy", "deny"]] {
        let mut args = vec!["build", "main.py", "-o", "main", "--ext"];
        args.extend_from_slice(flags);
        let output = pycc(&dir, &args);
        assert_eq!(output.status.code(), Some(2), "{flags:?}");
        assert!(stderr_of(&output).contains("cannot be used with"));
    }
    for policy in ["auto", "allowlist", "deny"] {
        for flags in [
            ["--pure", "--interop-policy", policy],
            ["--interop-policy", policy, "--pure"],
        ] {
            for output in [check(&dir, &flags), build(&dir, &flags), run(&dir, &flags)] {
                assert_eq!(output.status.code(), Some(2), "{flags:?}");
                assert!(stderr_of(&output).contains("cannot be used with"));
            }
        }
    }
}

// ---------------------------------------------------------------------
// `pycc run`'s forwarding window.
// ---------------------------------------------------------------------

/// After `--`, `--pure` is the program's argument, so `auto` admits the
/// import; right after `PATH`, it is pycc's flag and rejects it.
#[test]
fn run_forwards_pure_after_a_separator_and_consumes_it_after_path() {
    let dir = project("1224_run_window", None, JSON);
    let forwarded = run(&dir, &["--", "--pure"]);
    assert_eq!(
        forwarded.status.code(),
        Some(2),
        "{}",
        stderr_of(&forwarded)
    );
    assert!(stderr_of(&forwarded).contains("PYCC_PYTHON"));
    let consumed = run(&dir, &["--pure"]);
    assert_eq!(consumed.status.code(), Some(1), "{}", stderr_of(&consumed));
    assert!(stderr_of(&consumed).contains("error[I0402]"));
}
