//! A non-strict `--ext` module's compiled reads of a rebindable global see
//! every rebinding, whether other code makes it while the module body runs
//! (`globals()[...]`, `exec`) or after (`m.X = ...`, `vars(m)[...]`). On
//! CPython 3.15 the module dict is watched only once the body has run, and
//! until then every compiled read looks the name up (`global_sync.c`); on
//! 3.13 the dict's version tag decides throughout. The oracle is importing
//! the module's source.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&pycc_scratch::lf(bytes)).replace("\r\n", "\n")
}

fn copy(fixture: &str, dir: &Path, as_name: &str) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{fixture}.py"));
    std::fs::copy(source, dir.join(as_name)).expect("copy the fixture");
}

/// Builds `fixture` as `m` with `--ext --non-strict` and runs `driver`
/// (a fixture run as the program; None imports `m`) next to the artifact
/// and next to the source: the output of the artifact's run and the
/// oracle's. `global_watch_hook.py` is there as `hook`, a CPython module
/// (copied after the build, so it is not compiled into `m`).
fn compiled_and_oracle(fixture: &str, driver: Option<&str>) -> (String, String) {
    let compiled = ScratchDir::new("global_watch_ext").expect("scratch");
    let plain = ScratchDir::new("global_watch_src").expect("scratch");
    for dir in [&compiled, &plain] {
        copy(fixture, dir, "m.py");
        match driver {
            Some(driver) => copy(driver, dir, "driver.py"),
            None => std::fs::write(dir.join("driver.py"), "import m\n").expect("write the driver"),
        }
    }
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--non-strict", "--report-fallbacks"])
        .arg(compiled.join("m.py"))
        .arg("-o")
        .arg(compiled.join("m"))
        .output()
        .expect("pycc should spawn");
    let report = text(&build.stderr);
    assert!(build.status.success(), "{report}");
    // The readers are compiled: only the statements rebinding by a string
    // run in CPython.
    assert!(!report.contains("`get`") && !report.contains("`name`"), "{report}");
    std::fs::remove_file(compiled.join("m.py")).expect("import the artifact, not the source");
    for dir in [&compiled, &plain] {
        copy("global_watch_hook", dir, "hook.py");
    }
    let run = |dir: &Path| {
        python()
            .args(["-B", "driver.py"])
            .current_dir(dir)
            .output()
            .expect("python should spawn")
    };
    let (ours, oracle) = (run(&compiled), run(&plain));
    assert_eq!(ours.status.code(), Some(0), "{}", text(&ours.stderr));
    (text(&ours.stdout), text(&oracle.stdout))
}

/// Rebindings by fallback code during the body and by Python after it.
#[test]
fn compiled_reads_see_rebindings_during_and_after_the_body() {
    let (ours, oracle) =
        compiled_and_oracle("global_watch_after_body", Some("global_watch_after_body_driver"));
    assert_eq!(
        oracle, "1 a\n2 a\n3 b\n5 5 b\n5 b\n10 b\n11 c\n12\n",
        "the oracle runs the driver"
    );
    assert_eq!(ours, oracle);
}

/// The module imported a second time after leaving `sys.modules`, its
/// global bound by another module in each body: the reader's cache, which
/// outlives the first exec, must not match the second one's version.
#[test]
fn a_second_exec_of_the_module_reads_its_own_globals() {
    let (ours, oracle) =
        compiled_and_oracle("global_watch_reimport", Some("global_watch_reimport_driver"));
    assert_eq!(oracle, "run 0\nrun 1\nFalse 1\n", "the oracle runs the driver");
    assert_eq!(ours, oracle);
}

/// A read whose own lookup frees the value it replaces, whose `__del__`
/// rebinds the global and reads it again before the outer read returns.
#[test]
fn a_read_run_by_a_reads_destructor_looks_again() {
    let (ours, oracle) = compiled_and_oracle("global_watch_reentry", None);
    assert_eq!(
        oracle, "Last\nin __del__: set by __del__\nset by __del__\n",
        "the oracle runs the fixture"
    );
    assert_eq!(ours, oracle);
}
