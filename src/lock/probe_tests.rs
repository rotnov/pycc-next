use super::*;

/// The canned lock-probe output for `site`, as a fake interpreter prints it.
pub(crate) fn probe_lines(purelib: &std::path::Path, platlib: &std::path::Path) -> String {
    format!(
        "pycc-lock-probe 1\n{}\n{}\ncpython-314\nmacosx-11.0-arm64\ncpython\n3.14.7\nposix\narm64\nCPython\n25.0.0\nDarwin\nDarwin Kernel Version 25.0.0\n3.14.7\n3.14\ndarwin\n",
        purelib.display(),
        platlib.display()
    )
}

#[test]
fn canned_output_parses() {
    let text =
        probe_lines(std::path::Path::new("/p"), std::path::Path::new("/q")).replace('\n', "\r\n");
    let probe = parse_lock_probe(&text).unwrap();
    assert_eq!(probe.purelib, PathBuf::from("/p"));
    assert_eq!(probe.platlib, PathBuf::from("/q"));
    assert_eq!(probe.cache_tag, "cpython-314");
    assert_eq!(probe.platform, "macosx-11.0-arm64");
    assert_eq!(probe.markers, crate::lock::marker::tests::env());
}

#[test]
fn truncated_garbage_or_empty_output_is_refused() {
    let good = probe_lines(std::path::Path::new("/p"), std::path::Path::new("/q"));
    let lines: Vec<&str> = good.lines().collect();
    assert!(parse_lock_probe(&lines[..15].join("\n")).is_none());
    assert!(parse_lock_probe(&format!("{good}extra\n")).is_none());
    assert!(parse_lock_probe(&good.replace("pycc-lock-probe 1", "pycc-lock-probe 2")).is_none());
    assert!(parse_lock_probe(&good.replace("cpython-314\n", "\n")).is_none());
    assert!(parse_lock_probe("").is_none());
    assert!(parse_lock_probe("garbage").is_none());
}

#[cfg(unix)]
pub(crate) fn fake_interpreter(dir: &std::path::Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("fake-python");
    std::fs::write(&script, format!("#!/bin/sh\n{body}")).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[cfg(unix)]
#[test]
fn a_spawned_probe_runs_isolated_and_parses() {
    let dir = pycc_scratch::ScratchDir::new("lock_probe_spawn").unwrap();
    let lines = probe_lines(&dir.join("p"), &dir.join("q"));
    // The fake checks it was started with `-I -c <the lock probe>` and
    // without a variable that redirects `sysconfig`.
    let script = fake_interpreter(
        &dir,
        &format!(
            "[ -n \"$_PYTHON_HOST_PLATFORM$_PYTHON_SYSCONFIGDATA_NAME\" ] && exit 3\n\
             [ \"$1\" = -I ] && [ \"$2\" = -c ] || exit 4\n\
             case \"$3\" in *pycc-lock-probe*) ;; *) exit 5 ;; esac\n\
             cat <<'PYCC'\n{lines}PYCC\n"
        ),
    );
    let probe = run_lock_probe(script.as_os_str()).unwrap();
    assert_eq!(probe.purelib, dir.join("p"));
}

#[cfg(unix)]
#[test]
fn a_failing_or_missing_interpreter_is_refused() {
    let dir = pycc_scratch::ScratchDir::new("lock_probe_fail").unwrap();
    let script = fake_interpreter(&dir, "echo pycc-lock-probe 1\nexit 7\n");
    let err = run_lock_probe(script.as_os_str()).unwrap_err();
    assert!(
        err.contains("did not report") && err.contains("exit 7"),
        "{err}"
    );
    let err = run_lock_probe(dir.join("absent").as_os_str()).unwrap_err();
    assert!(err.contains("could not run the lock interpreter"), "{err}");
}
