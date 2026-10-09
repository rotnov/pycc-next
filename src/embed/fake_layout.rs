//! A fake CPython installation for the embedded mode's effectful tests, in
//! the `build_pipeline::ext_wiring_tests` header-less-toolchain tradition:
//! every step up to the compiler spawn runs for real, with no CPython
//! installed, and a stub `Python.h` then fails the compile deterministically.
//!
//! On macOS the libraries are real Mach-O images built with `cc`, so the
//! `otool`/`install_name_tool`/`codesign` spawns run against them.
//!
//! [`fake_windows_layout`] is the python.org Windows installation's shape
//! (D-253): the interpreter DLLs beside `python.exe`, the import libraries
//! in `libs\`, `Lib\`, `DLLs\` and `Include\`, and a system directory
//! beside it ([`fake_windows_env`]). The DLLs are synthetic PE images
//! ([`super::pe::fixture`]) and the rest plain files, so the Windows probe,
//! import scan, bundle and plan run on every host.
//!
//! The tests that build a real Mach-O library or run the POSIX embed
//! wiring are gated off on Windows, so part of this module has no caller
//! on that host.
#![cfg_attr(windows, allow(dead_code))]

use super::EmbedProbe;
use super::WindowsEnv;
use super::pe::fixture::PeSpec;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A fake interpreter prefix: `<root>/prefix` holds `include/python3.14`,
/// `lib/libpython3.14.dylib` and `lib/python3.14`.
pub(crate) struct FakeLayout {
    pub(crate) prefix: PathBuf,
    pub(crate) probe: EmbedProbe,
}

impl FakeLayout {
    pub(crate) fn stdlib(&self) -> PathBuf {
        self.prefix.join("lib").join("python3.14")
    }

    pub(crate) fn dynload(&self) -> PathBuf {
        self.stdlib().join("lib-dynload")
    }

    pub(crate) fn library(&self) -> PathBuf {
        self.prefix.join("lib").join("libpython3.14.dylib")
    }
}

fn write(path: &Path, contents: impl AsRef<[u8]>) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("create the parent");
    std::fs::write(path, contents).expect("write a fake-layout file");
}

/// Builds the layout under `root`. The standard library holds one file of
/// every kind the copy filter keeps or skips. The library is a plain file
/// unless [`macho_library`] replaces it.
pub(crate) fn fake_layout(root: &Path) -> FakeLayout {
    let root = std::fs::canonicalize(root).expect("canonicalize the scratch root");
    let prefix = root.join("prefix");
    let include = prefix.join("include").join("python3.14");
    write(
        &include.join("Python.h"),
        "#error pycc test fixture: not a real Python.h\n",
    );
    let lib = prefix.join("lib");
    let stdlib = lib.join("python3.14");
    for (rel, text) in [
        ("os.py", "# os\n"),
        ("json/__init__.py", "# json\n"),
        ("json/__pycache__/__init__.cpython-314.pyc", "junk"),
        ("site-packages/numpy/__init__.py", "# numpy\n"),
        ("tkinter/__init__.py", "# tkinter\n"),
        ("turtle.py", "# turtle\n"),
        ("test/test_os.py", "# test\n"),
        ("lib-dynload/_tkinter.cpython-314-darwin.so", "junk"),
    ] {
        write(&stdlib.join(rel), text);
    }
    write(&lib.join("libpython3.14.dylib"), "not a real library");
    let probe = EmbedProbe {
        version: (3, 14, 7),
        executable: prefix.join("bin").join("python3.14"),
        include,
        stdlib,
        base_prefix: prefix.clone(),
        enable_shared: true,
        framework: String::new(),
        ldlibrary: "libpython3.14.dylib".to_string(),
        libdir: lib,
        instsoname: "libpython3.14.so.1.0".to_string(),
        gil_disabled: false,
    };
    FakeLayout { prefix, probe }
}

/// Runs `cc` with `args` in `dir`, panicking on failure.
pub(crate) fn cc(dir: &Path, args: &[&str]) {
    let output = Command::new("cc")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("spawn cc");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "cc {args:?} failed: {stderr}");
}

/// Builds `out` as a tiny dylib whose id is its own absolute path, linked
/// against `deps`.
pub(crate) fn dylib(out: &Path, symbol: &str, deps: &[&Path]) {
    let dir = out.parent().expect("a parent");
    std::fs::create_dir_all(dir).expect("create the library directory");
    let source = dir.join(format!("{symbol}.c"));
    std::fs::write(&source, format!("int {symbol}(void) {{ return 1; }}\n")).expect("write C");
    let out_str = out.to_str().expect("utf-8");
    let mut args = vec!["-dynamiclib", "-o", out_str, "-install_name", out_str];
    args.push(source.to_str().expect("utf-8"));
    args.extend(deps.iter().map(|dep| dep.to_str().expect("utf-8")));
    cc(dir, &args);
    std::fs::remove_file(source).expect("remove the C source");
}

/// Builds `out` as a tiny Mach-O bundle (the shape of a real extension
/// module, which has no id) linked against `deps`.
pub(crate) fn bundle(out: &Path, symbol: &str, deps: &[&Path]) {
    let source = out.with_extension("c");
    std::fs::write(&source, format!("int {symbol}(void) {{ return 2; }}\n")).expect("write C");
    let mut args = vec!["-bundle", "-o", out.to_str().expect("utf-8")];
    args.push(source.to_str().expect("utf-8"));
    args.extend(deps.iter().map(|dep| dep.to_str().expect("utf-8")));
    cc(out.parent().expect("a parent"), &args);
    std::fs::remove_file(source).expect("remove the C source");
}

/// Replaces the layout's placeholder library with a real dylib and adds a
/// real extension module that links it by its absolute id (the python.org
/// framework shape the bundled arm rewrites).
pub(crate) fn macho_library(layout: &FakeLayout) {
    std::fs::remove_file(layout.library()).expect("remove the placeholder");
    dylib(&layout.library(), "pycc_fake_python", &[]);
    bundle(
        &layout.dynload().join("_json.cpython-314-darwin.so"),
        "pycc_fake_json",
        &[&layout.library()],
    );
}

/// Builds the Windows installation shape under `root`: `base\python314.dll`,
/// `python3.dll` and `vcruntime140.dll` (no `vcruntime140_1.dll`, so the
/// optional-copy skip runs), `libs\python314.lib` and `python3.lib`, a
/// `Lib\` and a `DLLs\` holding one file of every kind the Windows copy
/// filter keeps or skips, a stub `Include\Python.h`, and `System32\`
/// beside `base\` with `KERNEL32.dll`, `ADVAPI32.dll` and `WS2_32.dll`.
/// Every kept DLL is a PE32+ image whose imports resolve, with the case
/// mismatches (`VCRUNTIME140.dll`) and API sets a real install has.
pub(crate) fn fake_windows_layout(root: &Path) -> FakeLayout {
    let root = std::fs::canonicalize(root).expect("canonicalize the scratch root");
    let prefix = root.join("base");
    let include = prefix.join("Include");
    write(
        &include.join("Python.h"),
        "#error pycc test fixture: not a real Python.h\n",
    );
    let crt = "api-ms-win-crt-runtime-l1-1-0.dll";
    for (rel, imports, delay) in [
        (
            "python314.dll",
            &["KERNEL32.dll", "VCRUNTIME140.dll", crt][..],
            &[][..],
        ),
        ("python3.dll", &["python314.dll"], &[]),
        ("vcruntime140.dll", &["KERNEL32.dll", crt], &[]),
        (
            "DLLs/_ssl.pyd",
            &[
                "python314.dll",
                "libssl-3.dll",
                "KERNEL32.dll",
                "VCRUNTIME140.dll",
                crt,
            ],
            &[],
        ),
        (
            "DLLs/libssl-3.dll",
            &["KERNEL32.dll", "ADVAPI32.dll"],
            &["WS2_32.dll"],
        ),
    ] {
        write(&prefix.join(rel), PeSpec::dll(imports).delay(delay).bytes());
    }
    for name in ["KERNEL32.dll", "ADVAPI32.dll", "WS2_32.dll"] {
        write(&root.join("System32").join(name), PeSpec::dll(&[]).bytes());
    }
    for (rel, text) in [
        ("libs/python314.lib", "import library"),
        ("libs/python3.lib", "import library"),
        ("Lib/os.py", "# os\n"),
        ("Lib/json/__init__.py", "# json\n"),
        ("Lib/json/__pycache__/__init__.cpython-314.pyc", "junk"),
        ("Lib/site-packages/x/__init__.py", "# x\n"),
        ("Lib/tkinter/__init__.py", "# tkinter\n"),
        ("DLLs/py.ico", "icon"),
        ("DLLs/_tkinter.pyd", "tkinter"),
        ("DLLs/tcl86t.dll", "tcl"),
    ] {
        write(&prefix.join(rel), text);
    }
    let probe = EmbedProbe {
        version: (3, 14, 7),
        executable: prefix.join("python.exe"),
        include,
        stdlib: prefix.join("Lib"),
        base_prefix: prefix.clone(),
        enable_shared: false,
        framework: String::new(),
        ldlibrary: "python314.dll".to_string(),
        libdir: prefix.join("libs"),
        instsoname: String::new(),
        gil_disabled: false,
    };
    FakeLayout { prefix, probe }
}

/// Writes an x86-64 PE32+ DLL importing `imports` at `path`, creating its
/// directory: a closure image or native in a fake site (#1306).
pub(crate) fn write_pe_dll(path: &Path, imports: &[&str]) {
    write(path, PeSpec::dll(imports).bytes());
}

/// The system directory [`fake_windows_layout`] writes beside `base\`.
pub(crate) fn fake_windows_env(layout: &FakeLayout) -> WindowsEnv {
    let root = layout.prefix.parent().expect("the layout root");
    WindowsEnv {
        system_dirs: vec![root.join("System32")],
    }
}
