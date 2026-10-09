use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

#[test]
fn quick_start_fixture_builds_and_prints_the_documented_sequence() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/quick_start.py");
    let dir = ScratchDir::new("quick_start").expect("scratch dir for the quick-start build");
    let out = dir.join("hello");
    let status = Command::new(pycc_bin())
        .args([
            "build",
            fixture.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "`pycc build` failed for quick_start fixture"
    );

    let output = Command::new(&out).output().unwrap();
    assert!(
        output.status.success(),
        "quick_start binary exited non-zero"
    );
    // The documented stdout lives in `tests/fixtures/quick_start.expected.txt`,
    // the single source of truth shared with README.md, site/index.html, and
    // docs/WEBSITE.md (issue #197). Git on Windows checks that fixture out with
    // `\r\n` under the default `core.autocrlf` text conversion, so the file's
    // bytes are normalized before comparison against the binary's own stdout,
    // which is always `\n`-terminated.
    let expected = include_str!("fixtures/quick_start.expected.txt").replace("\r\n", "\n");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        expected,
        "quick_start fixture stdout must match the README/site documented output"
    );
}
