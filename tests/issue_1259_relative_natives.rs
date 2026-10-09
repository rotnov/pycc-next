//! Part 4 of #1225 (#1259): a locked closure whose macOS images reach
//! libraries through `@rpath` and `@loader_path` references outside their
//! payload, and through an absolute install name into it, is locked and
//! bundled so that it runs from the embedded sidecar after the virtual
//! environment and every directory outside it are moved away (the
//! pycc.lock decision entry, D-249 rule 8; `docs/CLI_SPEC.md`'s
//! `pycc.lock` section).
//!
//! The one test is `#[ignore]`d: it compiles C libraries and C extensions
//! with the host `cc` against the real interpreter's headers
//! (`PYCC_PYTHON`, default `python3.14`, must be CPython 3.14.7 with a
//! shared libpython), in a `python3.14 -m venv --without-pip` environment.
//! Nothing downloads anything. The unit tests in `src/embed/` cover every
//! classification arm and the refusals without an interpreter.

#![cfg(target_os = "macos")]

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// pycc's own SHA-256, shared so the fixtures' RECORD hashes need no
/// hashing crate.
#[allow(dead_code)]
#[path = "../src/embed/sha256.rs"]
mod sha256;

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
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
fn write_dist(site: &Path, name: &str, files: &[(String, Vec<u8>)], requires: &[&str]) {
    let dist_info = format!("{name}-1.0.dist-info");
    let mut metadata = format!("Metadata-Version: 2.1\nName: {name}\nVersion: 1.0\n");
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

fn cc(args: &[&str]) {
    let output = Command::new("cc").args(args).output().expect("spawn cc");
    assert!(output.status.success(), "{}", stderr_of(&output));
}

/// Builds the dylib `out`, whose `symbol` returns `body`, with the install
/// name `id`, linked against `links` and carrying `rpaths`; returns its
/// bytes.
fn dylib(
    out: &Path,
    symbol: &str,
    body: &str,
    id: &str,
    links: &[&Path],
    rpaths: &[&str],
) -> Vec<u8> {
    let c = out.with_extension("c");
    let externs: String = links
        .iter()
        .map(|link| format!("int {}(void);\n", stem(link)))
        .collect();
    write(
        &c,
        format!("{externs}int {symbol}(void) {{ return {body}; }}\n").as_bytes(),
    );
    let (out_s, c_s) = (out.display().to_string(), c.display().to_string());
    let mut args = vec!["-dynamiclib", "-o", &out_s, "-install_name", id, &c_s];
    let links: Vec<String> = links
        .iter()
        .map(|link| link.display().to_string())
        .collect();
    args.extend(links.iter().map(String::as_str));
    let rpaths: Vec<String> = rpaths
        .iter()
        .map(|rpath| format!("-Wl,-rpath,{rpath}"))
        .collect();
    args.extend(rpaths.iter().map(String::as_str));
    cc(&args);
    std::fs::read(out).expect("read the library")
}

/// `libo1.dylib` is `o1`: each library's one function is its name's stem.
fn stem(library: &Path) -> String {
    let name = library.file_stem().expect("a name").to_string_lossy();
    name.trim_start_matches("lib").to_string()
}

/// Builds the extension module `tinyrel.<module>`, whose `value()` returns
/// what the one library in `links` does; returns its bytes.
fn extension(
    build: &Path,
    include: &str,
    suffix: &str,
    module: &str,
    link: &Path,
    rpaths: &[&str],
) -> Vec<u8> {
    let out = build.join(format!("{module}{suffix}"));
    let c = build.join(format!("{module}.c"));
    let function = stem(link);
    let source = format!(
        "#include <Python.h>\nint {function}(void);\n\
         static PyObject *value(PyObject *self, PyObject *args) {{ return PyLong_FromLong({function}()); }}\n\
         static PyMethodDef methods[] = {{{{\"value\", value, METH_NOARGS, NULL}}, {{NULL}}}};\n\
         static struct PyModuleDef def = {{PyModuleDef_HEAD_INIT, \"{module}\", NULL, -1, methods}};\n\
         PyMODINIT_FUNC PyInit_{module}(void) {{ return PyModule_Create(&def); }}\n"
    );
    write(&c, source.as_bytes());
    let (out_s, c_s, link_s) = (
        out.display().to_string(),
        c.display().to_string(),
        link.display().to_string(),
    );
    let include = format!("-I{include}");
    let mut args = vec![
        "-bundle",
        "-undefined",
        "dynamic_lookup",
        "-o",
        &out_s,
        &include,
        &c_s,
        &link_s,
    ];
    let rpaths: Vec<String> = rpaths
        .iter()
        .map(|rpath| format!("-Wl,-rpath,{rpath}"))
        .collect();
    args.extend(rpaths.iter().map(String::as_str));
    cc(&args);
    std::fs::read(out).expect("read the extension")
}

/// One program imports seven extension modules, one per bundling case,
/// and prints their values as one number, digit by digit:
///
/// 1. `@rpath` resolved through an absolute rpath outside the payload;
/// 2. an absolute first rpath missing on the host, the payload match
///    rebound;
/// 3. an absolute first rpath present on the host (a payload decoy that
///    returns 9 comes second): a native;
/// 4. `@loader_path/../..` leaving the payload;
/// 5. a native whose `@rpath` dependency resolves through its own
///    `@loader_path` rpath (2) and whose `@loader_path/` dependency is a
///    sibling (3), summed;
/// 6. another locked distribution's payload file through `@rpath`;
/// 7. an absolute install name into the payload.
#[test]
#[ignore = "needs CPython 3.14.7 with a shared libpython (PYCC_PYTHON, default python3.14) and cc"]
fn relative_references_outside_the_payload_run_from_the_sidecar_and_match_cpython_3_14_7() {
    let dir = ScratchDir::new("relative_oracle").expect("scratch");
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
    let site = std::fs::canonicalize(site).expect("canonical site");
    let include = query(
        &python,
        "import sysconfig; print(sysconfig.get_path('include'))",
    );
    let suffix = query(
        &python,
        "import sysconfig; print(sysconfig.get_config_var('EXT_SUFFIX'))",
    );
    let build = dir.join("build");
    let natives = dir.join("natives");
    std::fs::create_dir_all(&build).expect("build dir");
    let ext = |module: &str, link: &Path, rpaths: &[&str]| {
        extension(&build, &include, &suffix, module, link, rpaths)
    };
    let mut rel: Vec<(String, Vec<u8>)> = Vec::new();

    // 1.
    let o1 = natives.join("rp/libo1.dylib");
    std::fs::create_dir_all(o1.parent().unwrap()).unwrap();
    dylib(&o1, "o1", "1", "@rpath/libo1.dylib", &[], &[]);
    let rp = natives.join("rp").display().to_string();
    rel.push((format!("tinyrel/_c1{suffix}"), ext("_c1", &o1, &[&rp])));
    // 2.
    let h = build.join("libh.dylib");
    let h_bytes = dylib(&h, "h", "2", "@rpath/libh.dylib", &[], &[]);
    let rpaths = ["/pycc-test-nowhere/lib", "@loader_path/.dylibs"];
    rel.push((format!("tinyrel/_c2{suffix}"), ext("_c2", &h, &rpaths)));
    rel.push(("tinyrel/.dylibs/libh.dylib".into(), h_bytes));
    // 3.
    let q = natives.join("q/libq.dylib");
    std::fs::create_dir_all(q.parent().unwrap()).unwrap();
    dylib(&q, "q", "3", "@rpath/libq.dylib", &[], &[]);
    let decoy = build.join("decoy/libq.dylib");
    std::fs::create_dir_all(decoy.parent().unwrap()).unwrap();
    let decoy = dylib(&decoy, "q", "9", "@rpath/libq.dylib", &[], &[]);
    let q_dir = natives.join("q").display().to_string();
    let rpaths = [q_dir.as_str(), "@loader_path/.dylibs"];
    rel.push((format!("tinyrel/_c3{suffix}"), ext("_c3", &q, &rpaths)));
    rel.push(("tinyrel/.dylibs/libq.dylib".into(), decoy));
    // 4: `<site>/tinyrel` up to `dir`, then `natives/`.
    let o3 = natives.join("libo3.dylib");
    let up = site.strip_prefix(&dir).unwrap().components().count() + 1;
    let o3_id = format!("@loader_path/{}natives/libo3.dylib", "../".repeat(up));
    dylib(&o3, "o3", "4", &o3_id, &[], &[]);
    rel.push((format!("tinyrel/_c4{suffix}"), ext("_c4", &o3, &[])));
    // 5.
    let n = natives.join("n");
    std::fs::create_dir_all(&n).unwrap();
    let (n1, n2, n3) = (
        n.join("libn1.dylib"),
        n.join("libn2.dylib"),
        n.join("libn3.dylib"),
    );
    dylib(&n2, "n2", "2", "@rpath/libn2.dylib", &[], &[]);
    dylib(&n3, "n3", "3", "@loader_path/libn3.dylib", &[], &[]);
    let n1_id = n1.display().to_string();
    dylib(
        &n1,
        "n1",
        "n2() + n3()",
        &n1_id,
        &[&n2, &n3],
        &["@loader_path"],
    );
    rel.push((format!("tinyrel/_c5{suffix}"), ext("_c5", &n1, &[])));
    // 6: `tinyb`, which `tinyrel` requires, installs `tinyb/libb.dylib`.
    let b = build.join("libb.dylib");
    let b_bytes = dylib(&b, "b", "6", "@rpath/libb.dylib", &[], &[]);
    rel.push((
        format!("tinyrel/_c6{suffix}"),
        ext("_c6", &b, &["@loader_path/../tinyb"]),
    ));
    // 7.
    let a = build.join("liba.dylib");
    let a_id = site
        .join("tinyrel/.dylibs/liba.dylib")
        .display()
        .to_string();
    let a_bytes = dylib(&a, "a", "7", &a_id, &[], &[]);
    rel.push((format!("tinyrel/_c7{suffix}"), ext("_c7", &a, &[])));
    rel.push(("tinyrel/.dylibs/liba.dylib".into(), a_bytes));

    let init = (1..=7)
        .map(|case| format!("from tinyrel._c{case} import value as v{case}\n"))
        .chain(["\n\ndef code():\n    return int(\"\".join(str(v()) for v in (v1, v2, v3, v4, v5, v6, v7)))\n".to_string()])
        .collect::<String>();
    rel.push(("tinyrel/__init__.py".into(), init.into_bytes()));
    write_dist(&site, "tinyrel", &rel, &["tinyb"]);
    let b_files = [
        ("tinyb/__init__.py".to_string(), b"".to_vec()),
        ("tinyb/libb.dylib".to_string(), b_bytes),
    ];
    write_dist(&site, "tinyb", &b_files, &[]);
    std::fs::write(
        dir.join("m.py"),
        "import tinyrel\n\nprint(int(tinyrel.code()))\n",
    )
    .expect("write");

    let output = run_in(&dir, &python, &["lock", "m.py"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let lock = std::fs::read_to_string(dir.join("pycc.lock")).expect("read the lock");
    let names: Vec<&str> = lock
        .lines()
        .filter_map(|line| line.strip_prefix("name = \"lib"))
        .collect();
    assert_eq!(
        names,
        [
            "n1.dylib\"",
            "n2.dylib\"",
            "n3.dylib\"",
            "o1.dylib\"",
            "o3.dylib\"",
            "q.dylib\""
        ],
        "{lock}"
    );
    let output = run_in(&dir, &python, &["build", "m.py", "-o", "app"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let oracle = Command::new(&python)
        .arg(dir.join("m.py"))
        .output()
        .expect("CPython runs the program");
    assert_eq!(oracle.status.code(), Some(0), "{}", stderr_of(&oracle));
    std::fs::rename(&venv, dir.join("venv-moved")).expect("move the environment away");
    std::fs::rename(&natives, dir.join("natives-moved")).expect("move the natives away");
    std::fs::rename(&build, dir.join("build-moved")).expect("move the build away");
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded binary runs");
    assert_eq!(embedded.status.code(), Some(0), "{}", stderr_of(&embedded));
    let stdout = String::from_utf8_lossy(&embedded.stdout);
    assert_eq!(stdout, "1234567\n");
    assert_eq!(stdout, String::from_utf8_lossy(&oracle.stdout));
}
