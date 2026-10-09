//! Part 3 of #1225 (#1243): a locked closure whose extension module
//! links a native library outside the interpreter, which links another in
//! turn, is locked with both as `[[target.native]]` entries and runs from
//! the embedded sidecar after the directory holding them is moved away
//! (the pycc.lock decision entry, D-249 rule 8; `docs/CLI_SPEC.md`'s
//! `pycc.lock` section).
//!
//! The one test is `#[ignore]`d: it compiles two C libraries and a C
//! extension with the host `cc` against the real interpreter's headers
//! (`PYCC_PYTHON`, default `python3.14`, must be CPython 3.14.7 with a
//! shared libpython), in a `python3.14 -m venv --without-pip` environment.
//! Nothing downloads anything. The unit tests in `src/embed/` cover the
//! derivation, the refusals and the copy without a C compiler.

#![cfg_attr(windows, allow(dead_code, unused_imports))]

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// pycc's own SHA-256, shared so the fixtures' RECORD hashes need no
/// hashing crate.
#[allow(dead_code)]
#[path = "../src/embed/sha256.rs"]
mod sha256;

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("create the parent");
    std::fs::write(path, bytes).expect("write a fixture file");
}

/// Unpadded urlsafe base64.
fn urlsafe_b64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let mut acc = 0u32;
        for (i, byte) in chunk.iter().enumerate() {
            acc |= u32::from(*byte) << (16 - 8 * i);
        }
        for i in 0..=chunk.len() {
            out.push(ALPHABET[((acc >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

fn record_hash(bytes: &[u8]) -> String {
    let hex = sha256::sha256_hex(bytes);
    let raw: Vec<u8> = (0..32)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hex"))
        .collect();
    format!("sha256={}", urlsafe_b64(&raw))
}

/// Writes an installed distribution the way `pip` does: payload `files`,
/// METADATA with `requires`, INSTALLER, and a RECORD listing them all.
fn write_dist(site: &Path, name: &str, version: &str, files: &[(&str, &[u8])], requires: &[&str]) {
    let dist_info = format!("{name}-{version}.dist-info");
    let mut metadata = format!("Metadata-Version: 2.1\nName: {name}\nVersion: {version}\n");
    for requirement in requires {
        metadata.push_str(&format!("Requires-Dist: {requirement}\n"));
    }
    let mut record = String::new();
    let mut add = |path: &str, bytes: &[u8]| {
        write(&site.join(path), bytes);
        record.push_str(&format!("{path},{},{}\n", record_hash(bytes), bytes.len()));
    };
    for (path, bytes) in files {
        add(path, bytes);
    }
    add(&format!("{dist_info}/METADATA"), metadata.as_bytes());
    add(&format!("{dist_info}/INSTALLER"), b"pip\n");
    record.push_str(&format!("{dist_info}/RECORD,,\n"));
    write(&site.join(&dist_info).join("RECORD"), record.as_bytes());
}

fn run_in(dir: &Path, python: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(args)
        .current_dir(dir)
        .env("PYCC_PYTHON", python)
        .output()
        .expect("pycc should spawn")
}

fn query(python: &Path, code: &str) -> String {
    let output = Command::new(python)
        .args(["-c", code])
        .output()
        .expect("spawn the interpreter");
    assert!(output.status.success(), "{}", stderr_of(&output));
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Compiles `source` into the shared library `out`, linking `libs`, which
/// it finds at run time in their own directories.
fn compile(out: &Path, source: &str, extra: &[&str], libs: &[&Path]) {
    let c = out.with_extension("c");
    write(&c, source.as_bytes());
    let mut command = Command::new("cc");
    command.args(["-shared", "-fPIC", "-o"]).arg(out).arg(&c);
    if cfg!(target_os = "macos") {
        command.arg("-install_name").arg(out);
    } else {
        let name = out.file_name().expect("a name").to_string_lossy();
        command.arg(format!("-Wl,-soname,{name}"));
        // Each linked library resolves from its own directory.
        for lib in libs {
            let dir = lib.parent().expect("a parent").display().to_string();
            command.arg(format!("-Wl,-rpath,{dir}"));
        }
    }
    command.args(extra).args(libs);
    let output = command.output().expect("spawn cc");
    assert!(output.status.success(), "{}", stderr_of(&output));
}

/// `libnat2` returns 21, `libnat1` doubles it, and the extension
/// `tinynat._nat.value()` returns what `libnat1` does.
#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 with a shared libpython (PYCC_PYTHON, default python3.14) and cc"]
fn a_closure_needing_outside_natives_runs_from_the_sidecar_and_matches_cpython_3_14_7() {
    let dir = ScratchDir::new("native_oracle").expect("scratch");
    let dir = std::fs::canonicalize(&*dir).expect("canonicalize");
    let base = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3.14".into());
    let venv = dir.join("venv");
    let status = Command::new(&base)
        .args(["-m", "venv", "--without-pip"])
        .arg(&venv)
        .status()
        .expect("spawn the base interpreter");
    assert!(status.success());
    let python = venv.join("bin").join("python");
    let site = PathBuf::from(query(
        &python,
        "import sysconfig; print(sysconfig.get_path('platlib'))",
    ));
    let include = query(
        &python,
        "import sysconfig; print(sysconfig.get_path('include'))",
    );
    let suffix = query(
        &python,
        "import sysconfig; print(sysconfig.get_config_var('EXT_SUFFIX'))",
    );
    let (ext, undefined): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("dylib", &["-undefined", "dynamic_lookup"])
    } else {
        ("so", &[])
    };
    let natives = dir.join("natives");
    let nat2 = natives.join(format!("libnat2.{ext}"));
    let nat1 = natives.join(format!("libnat1.{ext}"));
    compile(&nat2, "int nat2(void) { return 21; }\n", &[], &[]);
    let nat1_source = "int nat2(void);\nint nat1(void) { return nat2() * 2; }\n";
    compile(&nat1, nat1_source, &[], &[&nat2]);
    let module = "#include <Python.h>\nint nat1(void);\n\
         static PyObject *value(PyObject *self, PyObject *args) { return PyLong_FromLong(nat1()); }\n\
         static PyMethodDef methods[] = {{\"value\", value, METH_NOARGS, NULL}, {NULL}};\n\
         static struct PyModuleDef def = {PyModuleDef_HEAD_INIT, \"_nat\", NULL, -1, methods};\n\
         PyMODINIT_FUNC PyInit__nat(void) { return PyModule_Create(&def); }\n";
    let built = dir.join("build").join(format!("_nat{suffix}"));
    let mut flags = vec!["-I", include.as_str()];
    flags.extend(undefined);
    compile(&built, module, &flags, &[&nat1]);
    let so = std::fs::read(&built).expect("read the extension");
    let so_path = format!("tinynat/_nat{suffix}");
    let init = b"from tinynat._nat import value\n";
    let files: [(&str, &[u8]); 2] = [("tinynat/__init__.py", init), (&so_path, &so)];
    write_dist(&site, "tinynat", "1.0", &files, &[]);
    std::fs::write(
        dir.join("m.py"),
        "import tinynat\n\nprint(int(tinynat.value()))\n",
    )
    .expect("write");

    let output = run_in(&dir, &python, &["lock", "m.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let lock = std::fs::read_to_string(dir.join("pycc.lock")).expect("read the lock");
    for name in [format!("libnat1.{ext}"), format!("libnat2.{ext}")] {
        let entry = format!("[[target.native]]\nname = \"{name}\"\n");
        assert!(lock.contains(&entry), "{lock}");
    }
    let output = run_in(&dir, &python, &["build", "m.py", "-o", "app"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert!(dir.join(format!("app.pycc/lib/libnat1.{ext}")).is_file());
    assert!(dir.join(format!("app.pycc/lib/libnat2.{ext}")).is_file());
    let oracle = Command::new(&python)
        .arg(dir.join("m.py"))
        .output()
        .expect("CPython runs the program");
    assert_eq!(oracle.status.code(), Some(0), "{}", stderr_of(&oracle));
    std::fs::rename(&venv, dir.join("venv-moved")).expect("move the environment away");
    std::fs::rename(&natives, dir.join("natives-moved")).expect("move the natives away");
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded binary runs");
    assert_eq!(embedded.status.code(), Some(0), "{}", stderr_of(&embedded));
    let stdout = String::from_utf8_lossy(&embedded.stdout).replace("\r\n", "\n");
    let oracle_stdout = String::from_utf8_lossy(&oracle.stdout).replace("\r\n", "\n");
    assert_eq!(stdout, "42\n");
    assert_eq!(stdout, oracle_stdout);
}
