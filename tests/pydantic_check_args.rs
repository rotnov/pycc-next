//! `bench/pydantic_check.py` profiles pydantic and then builds it with the
//! profile. A winning set `pycc profile`'s pass 3 measured is valid for a
//! build only with the same `--exclude` list, so the profile it runs must name
//! the excludes the package build names (`pydantic.v1`); with a different
//! list the build compiled nothing and the benchmark timed CPython against
//! CPython. The script must also import: its package description is a
//! `package_check.Package`.

use std::process::Command;

const SCRIPT: &str = r#"
import os, pathlib, subprocess, sys
sys.path.insert(0, os.path.join(sys.argv[1], "bench"))
import pydantic_check as pc

seen = []
def run(argv, **kwargs):
    seen.append(argv)
    return subprocess.CompletedProcess(argv, 0, "", "")
pc.subprocess.run = run
pc.training_profile("pycc", "python3", pathlib.Path("src"), pathlib.Path("out.json"))
argv = seen[0]
cut = argv.index("--")
print("profile", argv[1])
print("excludes", " ".join(argv[:cut][-len(pc.EXCLUDE):]) == " ".join(pc.EXCLUDE))
print("exclude", pc.EXCLUDE)
"#;

#[test]
#[ignore = "requires python3 (PYCC_PYTHON, default python3)"]
fn the_training_profile_names_the_builds_excludes() {
    let python = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    let out = Command::new(python)
        .args(["-I", "-B", "-c", SCRIPT, env!("CARGO_MANIFEST_DIR")])
        .output()
        .expect("python");
    let stdout = pycc_scratch::lf(&*String::from_utf8_lossy(&out.stdout)).replace("\r\n", "\n");
    assert!(out.status.success(), "{stdout}{}", pycc_scratch::lf(&*String::from_utf8_lossy(&out.stderr)));
    assert_eq!(stdout, "profile profile\nexcludes True\nexclude ['--exclude', 'v1']\n");
}
