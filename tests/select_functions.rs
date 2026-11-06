//! A non-strict `--ext` build compiles only its selected functions
//! (docs/design/compile-little.md, sections 2.1-2.4): the module runs as
//! CPython bytecode and each selected function is patched into the function
//! CPython made. The module must behave as its source does (output, tracebacks,
//! `__file__`), and only what was selected is compiled.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

const FIXTURE: &str = include_str!("fixtures/select_functions.py");

fn python() -> std::ffi::OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into())
}

fn pycc(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pycc")).args(args).output().expect("pycc runs")
}

fn text(o: &Output) -> String {
    // Line endings as on POSIX: Windows' Python writes `\r\n`.
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).replace('\r', "")
}

/// `import m; m.main()` from `dir`, with `dir` replaced by `DIR` in the output.
fn run_main(dir: &Path) -> String {
    let o = Command::new(python())
        .args(["-I", "-c", "import sys; sys.path.insert(0, sys.argv[1]); import m; m.main()"])
        .arg(dir)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", text(&o));
    String::from_utf8_lossy(&o.stdout).replace('\r', "").replace(dir.to_str().unwrap(), "DIR")
}

/// Two copies of the fixture as `m.py`: one built with `flags`, one left alone.
fn build(scratch: &Path, flags: &[&str]) -> (std::path::PathBuf, Output) {
    let dir = scratch.join("built");
    let plain = scratch.join("plain");
    for d in [&dir, &plain] {
        std::fs::create_dir_all(d).unwrap();
        std::fs::write(d.join("m.py"), FIXTURE).unwrap();
    }
    let py = dir.join("m.py");
    let out = dir.join("m");
    let mut args: Vec<&std::ffi::OsStr> = vec!["build".as_ref(), py.as_os_str(), "-o".as_ref(), out.as_os_str()];
    args.extend(["--ext", "--non-strict", "--report-fallbacks"].map(std::ffi::OsStr::new));
    args.extend(flags.iter().map(std::ffi::OsStr::new));
    let o = pycc(&args);
    assert!(o.status.success(), "{}", text(&o));
    (dir, o)
}

fn extension(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .unwrap()
        .any(|e| {
            let name = e.unwrap().file_name().to_string_lossy().into_owned();
            name.ends_with(".so") || name.ends_with(".pyd")
        })
}

#[test]
fn select_all_compiles_every_eligible_function_and_behaves_as_the_source() {
    let scratch = ScratchDir::new("select_fn").unwrap();
    let (dir, o) = build(&scratch, &["--select", "all"]);
    assert!(extension(&dir));
    let stderr = String::from_utf8_lossy(&o.stderr);
    let compiled: Vec<&str> = stderr.lines().filter(|l| l.starts_with("compiled: ")).collect();
    assert_eq!(
        compiled,
        [
            "compiled: m:hot",
            "compiled: m:Base.greet",
            "compiled: m:Box.__init__",
            "compiled: m:Box.total",
            "compiled: m:Box.greet",
            "compiled: m:Box.size",
            "compiled: m:Box.twice",
            "compiled: m:wrapped",
            "compiled: m:boom",
        ],
        "{stderr}"
    );
    // A function that reads the module's globals by a string stays
    // bytecode, as does one that does not compile (`deco` nests a `def`), and
    // a nested function is not eligible.
    assert!(stderr.contains("not compiled: m:main: it reaches the module's globals by a string"), "{stderr}");
    assert!(stderr.contains("not compiled: m:deco: C0001"), "{stderr}");
    assert!(!stderr.contains("m:deco.<locals>.wrapper"), "{stderr}");
    assert_eq!(run_main(&dir), run_main(&scratch.join("plain")));
    // The compiled functions are patched in: the module's own functions,
    // not new ones.
    let o = Command::new(python())
        .args([
            "-I",
            "-c",
            "import sys; sys.path.insert(0, sys.argv[1]); import m\n\
             print(type(m.__spec__.loader).__name__, m.__file__.endswith('m.py'))\n\
             print(m.hot.__code__.co_qualname, m.hot.__module__, m.Box.total.__qualname__)",
        ])
        .arg(&dir)
        .output()
        .unwrap();
    assert_eq!(text(&o), "ExtensionFileLoader True\nhot m Box.total\n");
}

/// Where the selected functions cannot be patched (a CPython whose layouts
/// the frames probe does not know, simulated by `PYCC_NO_PATCH`), they run
/// as their bytecode, never silently: the import warns, and under
/// `PYCC_REQUIRE_PATCH` it fails.
#[test]
fn an_interpreter_that_cannot_patch_says_so() {
    let scratch = ScratchDir::new("select_fn").unwrap();
    let (dir, _) = build(&scratch, &["--select", "all"]);
    let run = |env: &[(&str, &str)]| {
        Command::new(python())
            .args(["-I", "-c", "import sys; sys.path.insert(0, sys.argv[1]); import m; m.main()"])
            .arg(&dir)
            .envs(env.iter().copied())
            .output()
            .unwrap()
    };
    let o = run(&[("PYCC_NO_PATCH", "1")]);
    assert!(o.status.success(), "{}", text(&o));
    let stderr = String::from_utf8_lossy(&o.stderr).replace('\r', "");
    assert!(
        stderr.contains("RuntimeWarning: pycc: m: the selected functions run as bytecode: patching them needs"),
        "{stderr}"
    );
    assert_eq!(
        String::from_utf8_lossy(&o.stdout).replace('\r', "").replace(dir.to_str().unwrap(), "DIR"),
        run_main(&scratch.join("plain"))
    );
    let o = run(&[("PYCC_NO_PATCH", "1"), ("PYCC_REQUIRE_PATCH", "1")]);
    assert!(!o.status.success());
    assert!(
        text(&o).contains("ImportError: pycc: m: the selected functions cannot be patched on this interpreter"),
        "{}",
        text(&o)
    );
    // Patched, it neither warns nor fails.
    let o = run(&[("PYCC_REQUIRE_PATCH", "1")]);
    assert!(o.status.success() && !text(&o).contains("RuntimeWarning"), "{}", text(&o));
}

#[test]
fn without_a_profile_nothing_is_selected_and_no_extension_is_built() {
    let scratch = ScratchDir::new("select_fn").unwrap();
    let (dir, o) = build(&scratch, &[]);
    assert!(!extension(&dir));
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(stderr.contains("note: no profile (`--profile`): no function is selected"), "{stderr}");
    assert!(stderr.contains("no extension is built; `"), "{stderr}");
    assert!(stderr.contains("` is the module, which CPython imports as it is"), "{stderr}");
    let (dir, _) = build(&scratch, &["--select", "none"]);
    assert!(!extension(&dir));
}

#[test]
fn with_nothing_selected_the_output_is_the_module_as_source() {
    let scratch = ScratchDir::new("select_fn").unwrap();
    let src = scratch.join("src");
    std::fs::create_dir_all(&src).unwrap();
    let py = src.join("m.py");
    std::fs::write(&py, FIXTURE).unwrap();
    let dist = scratch.join("dist");
    let out = dist.join("named");
    std::fs::create_dir_all(&dist).unwrap();
    let build = |select: &str| {
        let o = pycc(&[
            "build".as_ref(),
            py.as_os_str(),
            "-o".as_ref(),
            out.as_os_str(),
            "--ext".as_ref(),
            "--non-strict".as_ref(),
            "--select".as_ref(),
            select.as_ref(),
        ]);
        assert!(o.status.success(), "{}", text(&o));
    };
    let has_extension = || extension(&dist);
    // An extension of an earlier build is replaced by the source, which
    // would otherwise be shadowed by it.
    build("all");
    assert!(has_extension());
    build("none");
    assert!(!has_extension());
    assert_eq!(std::fs::read_to_string(dist.join("named.py")).unwrap(), FIXTURE);
    let run = |dir: &Path, name: &str| {
        let o = Command::new(python())
            .args(["-I", "-c", &format!("import sys; sys.path.insert(0, sys.argv[1]); import {name}; {name}.main()")])
            .arg(dir)
            .output()
            .unwrap();
        (o.status.success(), String::from_utf8_lossy(&o.stdout).replace('\r', "").replace(dir.to_str().unwrap(), "DIR"))
    };
    let (ok, got) = run(&dist, "named");
    assert!(ok, "{got}");
    assert_eq!(got.replace("named", "m"), run(&src, "m").1);
}

#[test]
fn compile_and_no_compile_adjust_the_selection() {
    let scratch = ScratchDir::new("select_fn").unwrap();
    let (dir, o) = build(&scratch, &["--select", "none", "--compile", "m:hot", "--compile", "m:Box.greet"]);
    let stderr = String::from_utf8_lossy(&o.stderr);
    let compiled: Vec<&str> = stderr.lines().filter(|l| l.starts_with("compiled: ")).collect();
    assert_eq!(compiled, ["compiled: m:hot", "compiled: m:Box.greet"], "{stderr}");
    assert_eq!(run_main(&dir), run_main(&scratch.join("plain")));
    let (_, o) = build(&scratch, &["--select", "all", "--no-compile", "m:hot"]);
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(!stderr.contains("compiled: m:hot\n") && stderr.contains("compiled: m:boom\n"), "{stderr}");
    // A function that is not in the sources is an error.
    let scratch = ScratchDir::new("select_fn").unwrap();
    let dir = scratch.join("d");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("m.py"), FIXTURE).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(dir.join("m.py"))
        .arg("-o")
        .arg(dir.join("m"))
        .args(["--ext", "--non-strict", "--compile", "m:nope"])
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(text(&o).contains("--compile `m:nope`: no such function in the build's sources"), "{}", text(&o));
}

#[test]
fn a_profile_selects_the_hot_functions() {
    let scratch = ScratchDir::new("select_fn").unwrap();
    let work_dir = scratch.join("w");
    std::fs::create_dir_all(&work_dir).unwrap();
    std::fs::write(work_dir.join("m.py"), FIXTURE).unwrap();
    let work = scratch.join("work.py");
    std::fs::write(
        &work,
        "import m\nfor i in range(3000):\n    m.hot({1: 2}, 1)\n    m.Box.twice(i)\n",
    )
    .unwrap();
    let profile = scratch.join("p.json");
    let o = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("profile")
        .arg("--no-verify")
        .arg(work_dir.join("m.py"))
        .arg("-o")
        .arg(&profile)
        .args(["--runs", "1", "--select-share", "0"])
        .arg("--")
        .arg(python())
        .arg(&work)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", text(&o));
    let p = profile.to_str().unwrap();
    let (dir, o) = build(&scratch, &["--profile", p, "--select-share", "0"]);
    let stderr = String::from_utf8_lossy(&o.stderr);
    let compiled: Vec<&str> = stderr.lines().filter(|l| l.starts_with("compiled: ")).collect();
    assert_eq!(compiled, ["compiled: m:hot", "compiled: m:Box.twice"], "{stderr}");
    assert_eq!(run_main(&dir), run_main(&scratch.join("plain")));
}

#[test]
fn a_package_serves_only_its_modules_with_selected_functions() {
    let scratch = ScratchDir::new("select_fn").unwrap();
    let pkg = scratch.join("pkg");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("__init__.py"), "from .a import f\n").unwrap();
    std::fs::write(pkg.join("a.py"), "def f(x):\n    return [x] * 2\n").unwrap();
    std::fs::write(pkg.join("b.py"), "def g(x):\n    return x + 1\n").unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&pkg)
        .arg("-o")
        .arg(pkg.join("__init__"))
        .args(["--ext", "--non-strict", "--report-fallbacks", "--select", "none", "--compile", "pkg.a:f"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("compiled: pkg.a:f\n"), "{}", text(&o));
    let o = Command::new(python())
        .args([
            "-I",
            "-c",
            "import os, sys; sys.path.insert(0, sys.argv[1]); import pkg, pkg.a, pkg.b\n\
             for m in (pkg, pkg.a, pkg.b):\n    print(m.__name__, type(m.__spec__.loader).__name__, os.path.relpath(m.__file__, sys.argv[1]).replace(os.sep, '/'))\n\
             print(pkg.f(3), pkg.b.g(1))",
        ])
        .arg(scratch.join(""))
        .output()
        .unwrap();
    assert_eq!(
        text(&o),
        // The package's `__init__` is the extension that installs the finder;
        // its module runs as bytecode too.
        "pkg ExtensionFileLoader pkg/__init__.py\n\
         pkg.a _PyccFinder pkg/a.py\n\
         pkg.b SourceFileLoader pkg/b.py\n\
         [3, 3] 2\n"
    );
    // Rebuilt with nothing selected, the package is its sources again: the
    // earlier extension, which CPython would import ahead of `__init__.py`,
    // is removed.
    let o = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&pkg)
        .arg("-o")
        .arg(pkg.join("__init__"))
        .args(["--ext", "--non-strict", "--select", "none"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", text(&o));
    assert!(!extension(&pkg));
    let o = Command::new(python())
        .args([
            "-I",
            "-c",
            "import sys; sys.path.insert(0, sys.argv[1]); import pkg, pkg.a\n\
             print(type(pkg.__spec__.loader).__name__, type(pkg.a.__spec__.loader).__name__, pkg.f(3))",
        ])
        .arg(scratch.join(""))
        .output()
        .unwrap();
    assert_eq!(text(&o), "SourceFileLoader SourceFileLoader [3, 3]\n");
}
