//! Empty containers typed from producers that read unannotated functions
//! (`s.add(c.find(i))`, `self.d[k] = v`), `set.update`/`list.extend` of any
//! iterable, and a tuple display passed to `sorted`/`min`/`max`/`sum`,
//! compared with CPython in strict and `--non-strict` builds (the latter
//! with nothing falling back).

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

#[test]
fn solver_typed_containers_match_cpython() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/solver_typed_containers.py");
    let oracle = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    let expected = Command::new(oracle).arg(&fixture).output().expect("python3 should spawn");
    assert!(expected.status.success(), "{}", text(&pycc_scratch::lf(&expected.stderr)));
    let dir = ScratchDir::new("solver_typed_containers").expect("scratch");
    for (n, flags) in [&[][..], &["--non-strict", "--report-fallbacks"][..]].iter().enumerate() {
        let binary = dir.join(format!("app{n}"));
        let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
            .arg("build")
            .arg(&fixture)
            .arg("-o")
            .arg(&binary)
            .arg("--release")
            .args(*flags)
            .output()
            .expect("pycc should spawn");
        assert!(build.status.success(), "{flags:?}: {}", text(&pycc_scratch::lf(&build.stderr)));
        assert!(!text(&pycc_scratch::lf(&build.stderr)).contains("fallback:"), "{}", text(&pycc_scratch::lf(&build.stderr)));
        let run = Command::new(&binary).output().expect("the binary runs");
        assert!(run.status.success(), "{flags:?}: {}", text(&pycc_scratch::lf(&run.stderr)));
        assert_eq!(text(&pycc_scratch::lf(&run.stdout)), text(&pycc_scratch::lf(&expected.stdout)), "{flags:?}");
    }
}
