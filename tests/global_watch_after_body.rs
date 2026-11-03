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

#[test]
fn compiled_reads_see_rebindings_during_and_after_the_body() {
    let compiled = ScratchDir::new("global_watch_ext").expect("scratch");
    copy("global_watch_after_body", &compiled, "m.py");
    copy("global_watch_after_body_driver", &compiled, "driver.py");
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
    let plain = ScratchDir::new("global_watch_src").expect("scratch");
    copy("global_watch_after_body", &plain, "m.py");
    copy("global_watch_after_body_driver", &plain, "driver.py");
    let run = |dir: &Path| {
        python()
            .args(["-B", "driver.py"])
            .current_dir(dir)
            .output()
            .expect("python should spawn")
    };
    let (ours, oracle) = (run(&compiled), run(&plain));
    assert_eq!(
        text(&oracle.stdout),
        "1 a\n2 a\n3 b\n5 5 b\n5 b\n10 b\n11 c\n12\n",
        "the oracle runs the driver"
    );
    assert_eq!(text(&ours.stdout), text(&oracle.stdout), "{}", text(&ours.stderr));
    assert_eq!(ours.status.code(), Some(0), "{}", text(&ours.stderr));
}
