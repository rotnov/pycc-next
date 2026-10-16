//! `pycc.build`, the PEP 517 build backend in `python/` (ROADMAP v0.6 accept 1).
//!
//! The backend's own unit tests (`python/tests`, which stand a fake compiler in
//! for pycc) run here, and one project is built for real: `pip wheel` in a fresh
//! venv, with pip's isolated build environment installing the backend's wheel
//! from a local directory (no index), the compiler under test named by `PYCC`.
//! The installed wheel's compiled modules must print what CPython prints for
//! the sources, and a module pycc refuses must fail the build with pycc's
//! diagnostic. `bench/wheel_check.py` does the same for real PyPI packages in CI.

#![cfg(unix)]

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn python() -> String {
    std::env::var("PYCC_PYTHON").unwrap_or_else(|_| "python3".to_string())
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn run_ok(cmd: &mut Command) -> String {
    let output = cmd.output().expect("spawn");
    assert!(
        output.status.success(),
        "{cmd:?} failed:\n{}",
        text(&output)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn write(dir: &Path, rel: &str, contents: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

#[test]
fn the_backend_unit_tests_pass() {
    run_ok(
        Command::new(python())
            .args(["-B", "-m", "unittest", "discover", "-s"])
            .arg(root().join("python/tests"))
            .current_dir(root()),
    );
}

/// The backend's wheel, built from `python/` by itself as `scripts/package.sh` does.
fn backend_wheel(scratch: &Path) -> PathBuf {
    let dir = scratch.join("backend");
    std::fs::create_dir_all(&dir).unwrap();
    run_ok(
        Command::new(python())
            .args([
                "-I",
                "-B",
                "-c",
                "import sys; sys.path.insert(0, '.'); from pycc import build; \
                 build.build_wheel(sys.argv[1])",
            ])
            .arg(&dir)
            .current_dir(root().join("python")),
    );
    dir
}

const PYPROJECT: &str = r#"[build-system]
requires = ["pycc-build"]
build-backend = "pycc.build"

[project]
name = "geo-demo"
version = "0.1.0"
requires-python = ">=3.13"

[project.scripts]
geo-demo = "geo.cli:main"

[tool.pycc]
pure-python = ["geo.cli"]

[tool.pycc.overrides."geo.__init__"]
strict = false

[tool.pycc.overrides."geo.loose"]
strict = false
"#;

const LOADERS: &str = "\
import geo, geo.shapes, geo.loose, geo.cli
print(*(type(m.__loader__).__name__ for m in (geo, geo.shapes, geo.loose, geo.cli)))
";

const DRIVER: &str = "\
import geo, geo.shapes, geo.loose, geo.cli
print(geo.area(3, 4), geo.shapes.perimeter(3, 4), geo.loose.scale([1, 2], 3), geo.VERSION)
try:
    geo.shapes.area(-1, 2)
except ValueError as e:
    print('ValueError', e)
";

fn project(dir: &Path) {
    write(dir, "pyproject.toml", PYPROJECT);
    write(
        dir,
        "src/geo/__init__.py",
        "from .shapes import area\n\nVERSION = '0.1.0'\n",
    );
    write(
        dir,
        "src/geo/shapes.py",
        "def area(w: int, h: int) -> int:\n    \
         if w < 0 or h < 0:\n        raise ValueError(f'negative side {min(w, h)}')\n    \
         return w * h\n\n\ndef perimeter(w: int, h: int) -> int:\n    return 2 * (w + h)\n",
    );
    write(
        dir,
        "src/geo/loose.py",
        "def scale(xs, k):\n    return [x * k for x in xs]\n",
    );
    write(
        dir,
        "src/geo/cli.py",
        "from . import area\n\n\ndef main() -> None:\n    print(area(2, 5))\n",
    );
    write(dir, "src/geo/py.typed", "");
}

/// A fresh venv; pip there builds `proj` with the backend from `backend`.
fn pip_wheel(scratch: &Path, backend: &Path, proj: &Path, wheels: &Path) -> (PathBuf, Output) {
    let venv = scratch.join("venv");
    if !venv.exists() {
        run_ok(Command::new(python()).args(["-I", "-m", "venv"]).arg(&venv));
    }
    let vpy = venv.join("bin/python");
    let output = Command::new(&vpy)
        .args([
            "-I",
            "-m",
            "pip",
            "wheel",
            "--no-index",
            "--no-deps",
            "--no-cache-dir",
        ])
        .arg("--find-links")
        .arg(backend)
        .arg("-w")
        .arg(wheels)
        .arg(proj)
        .env("PYCC", env!("CARGO_BIN_EXE_pycc"))
        .env_remove("PIP_REQUIRE_VIRTUALENV")
        .current_dir(scratch)
        .output()
        .expect("run pip");
    (vpy, output)
}

/// Needs CPython 3.13+ with headers, `venv` and `pip` (`--include-ignored`, as CI runs).
#[test]
#[ignore = "needs CPython 3.13+ with headers, venv and pip"]
fn pip_builds_installs_and_runs_a_compiled_wheel() {
    let scratch = ScratchDir::new("pep517_wheel").expect("scratch");
    let backend = backend_wheel(&scratch);
    let proj = scratch.join("proj");
    project(&proj);
    let wheels = scratch.join("wheels");
    let (vpy, output) = pip_wheel(&scratch, &backend, &proj, &wheels);
    assert!(
        output.status.success(),
        "pip wheel failed:\n{}",
        text(&output)
    );
    let built: Vec<PathBuf> = std::fs::read_dir(&wheels)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(built.len(), 1, "{built:?}");
    let name = built[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with("geo_demo-0.1.0-cp313-abi3-"),
        "unexpected wheel name {name}"
    );
    run_ok(
        Command::new(&vpy)
            .args(["-I", "-m", "pip", "install", "--no-index", "--no-deps"])
            .arg(&built[0]),
    );
    let cwd = scratch.join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    let compiled = run_ok(
        Command::new(&vpy)
            .args(["-I", "-c", DRIVER])
            .current_dir(&cwd),
    );
    let oracle = run_ok(
        Command::new(python())
            .args(["-c", DRIVER])
            .current_dir(proj.join("src")),
    );
    assert_eq!(compiled, oracle);
    let loaders = run_ok(
        Command::new(&vpy)
            .args(["-I", "-c", LOADERS])
            .current_dir(&cwd),
    );
    assert_eq!(
        loaders, "ExtensionFileLoader ExtensionFileLoader ExtensionFileLoader SourceFileLoader\n",
        "geo, geo.shapes and geo.loose must be compiled, geo.cli Python"
    );
    let script = run_ok(&mut Command::new(scratch.join("venv/bin/geo-demo")));
    assert_eq!(script, "10\n");
}

#[test]
#[ignore = "needs CPython 3.13+ with headers, venv and pip"]
fn a_module_pycc_refuses_fails_the_build_with_its_diagnostic() {
    let scratch = ScratchDir::new("pep517_refused").expect("scratch");
    let backend = backend_wheel(&scratch);
    let proj = scratch.join("proj");
    project(&proj);
    // Strict mode: a public function without annotations is T0001.
    write(&proj, "src/geo/untyped.py", "def f(x):\n    return x\n");
    let wheels = scratch.join("wheels");
    let (_, output) = pip_wheel(&scratch, &backend, &proj, &wheels);
    let log = text(&output);
    assert!(!output.status.success(), "pip wheel succeeded:\n{log}");
    assert!(log.contains("pycc could not compile geo.untyped"), "{log}");
    assert!(log.contains("T0001"), "{log}");
    assert!(
        !wheels.exists() || std::fs::read_dir(&wheels).unwrap().next().is_none(),
        "a failed build left a wheel"
    );
}
