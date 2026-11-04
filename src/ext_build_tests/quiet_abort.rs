//! v0.12: an ELF module's panic and allocation-failure handlers are the
//! runtime's (`crates/pycc_rt/src/abort.rs`), wired by
//! [`quiet_abort_args`]: the archive's symbol index names the standard
//! library's handlers, and a program linked with the arguments reports the
//! failure in one line and aborts.

use super::*;
use std::path::PathBuf;

/// A System V symbol index of `names`, its words `width` bytes wide (the
/// offsets are not read).
fn index(names: &[&str], width: usize) -> Vec<u8> {
    let word = |value: usize| value.to_be_bytes()[std::mem::size_of::<usize>() - width..].to_vec();
    let mut out = word(names.len());
    for _ in names {
        out.extend(word(0));
    }
    for name in names {
        out.extend(name.as_bytes());
        out.push(0);
    }
    out
}

#[test]
fn a_symbol_index_lists_its_names_in_either_width() {
    for width in [4, 8] {
        let bytes = index(&["a", "_RNvCsX_7___rustc17rust_begin_unwind"], width);
        assert_eq!(
            parse_symbol_index(&bytes, width),
            vec!["a".to_string(), "_RNvCsX_7___rustc17rust_begin_unwind".to_string()]
        );
    }
    // Truncated or empty, it lists nothing.
    let bytes = index(&["a", "b"], 4);
    assert!(parse_symbol_index(&bytes[..bytes.len() - 2], 4).is_empty());
    assert!(parse_symbol_index(&bytes[..6], 4).is_empty());
    assert!(parse_symbol_index(&[], 4).is_empty());
}

/// An archive of one empty member whose symbol index (`/`) names `names`.
fn archive(dir: &Path, names: &[&str]) -> PathBuf {
    let body = index(names, 4);
    let mut bytes = b"!<arch>\n".to_vec();
    bytes.extend(format!("{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n", "/", 0, 0, 0, 0, body.len()).as_bytes());
    bytes.extend(&body);
    let path = dir.join("libfake.a");
    std::fs::write(&path, bytes).expect("write the archive");
    path
}

#[test]
fn only_an_elf_link_routes_the_handlers_its_archive_defines() {
    let scratch = pycc_scratch::ScratchDir::new("quiet_abort_index").expect("scratch");
    let both = archive(
        &scratch,
        &[
            "pycc_rt_panic_abort",
            "_RNvCsAb1_7___rustc17rust_begin_unwind",
            "_RNvCsAb1_7___rustc26___rust_alloc_error_handler",
        ],
    );
    let args: Vec<String> = quiet_abort_args(ExtLinkPlatform::Linux, &both)
        .into_iter()
        .map(|arg| arg.into_string().expect("utf-8"))
        .collect();
    assert_eq!(
        args,
        [
            "-Wl,--wrap=_RNvCsAb1_7___rustc17rust_begin_unwind",
            "-Wl,--defsym=__wrap__RNvCsAb1_7___rustc17rust_begin_unwind=pycc_rt_panic_abort",
            "-Wl,--wrap=_RNvCsAb1_7___rustc26___rust_alloc_error_handler",
            "-Wl,--defsym=__wrap__RNvCsAb1_7___rustc26___rust_alloc_error_handler=\
             pycc_rt_alloc_error_abort",
        ]
    );
    for platform in [ExtLinkPlatform::MacOs, ExtLinkPlatform::Windows] {
        assert!(quiet_abort_args(platform, &both).is_empty());
    }
    // A handler the archive does not define keeps the standard library's,
    // and a file that is no archive routes nothing.
    let one = archive(&scratch, &["_RNvCsAb1_7___rustc17rust_begin_unwind"]);
    assert_eq!(quiet_abort_args(ExtLinkPlatform::Linux, &one).len(), 2);
    let not_archive = scratch.join("plain.a");
    std::fs::write(&not_archive, "not an archive").expect("write");
    assert!(quiet_abort_args(ExtLinkPlatform::Linux, &not_archive).is_empty());
    assert!(quiet_abort_args(ExtLinkPlatform::Linux, &scratch.join("missing.a")).is_empty());
}

/// The host's runtime archive, from the profile these tests were built in.
#[cfg(target_os = "linux")]
fn runtime_archive() -> PathBuf {
    crate::build_pipeline::find_pycc_rt_lib_dir(None, !cfg!(debug_assertions))
        .expect("the runtime archive")
        .join("libpycc_rt.a")
}

/// The real runtime defines both handlers, and a program linked with the
/// arguments reports a runtime panic and an allocation failure in one line
/// each and aborts, with no backtrace (and no `RUST_BACKTRACE` note) even
/// when one is asked for.
#[cfg(target_os = "linux")]
#[test]
fn a_runtime_panic_or_allocation_failure_reports_one_line_and_aborts() {
    let archive = runtime_archive();
    let args = quiet_abort_args(ExtLinkPlatform::Linux, &archive);
    assert_eq!(args.len(), 4, "{args:?}");
    let symbols = archive_symbols(&archive);
    let handler = |tail: &str| {
        symbols
            .iter()
            .find(|name| name.starts_with("_R") && name.ends_with(tail))
            .expect("the archive defines the handler")
            .clone()
    };
    let alloc_handler = handler(QUIET_ABORT_HANDLERS[1].0);
    let scratch = pycc_scratch::ScratchDir::new("quiet_abort_link").expect("scratch");
    let source = scratch.join("main.c");
    // `pycc_rt_ext_int_from_hex` panics on text that is no hex literal; the
    // allocation-failure handler is called directly, as `handle_alloc_error`
    // calls it.
    std::fs::write(
        &source,
        format!(
            "#include <stddef.h>\n#include <string.h>\n\
             long long pycc_rt_ext_int_from_hex(const char *, size_t);\n\
             void {alloc_handler}(size_t, size_t);\n\
             int main(int argc, char **argv) {{\n\
               if (strcmp(argv[1], \"panic\") == 0) pycc_rt_ext_int_from_hex(\"zz\", 2);\n\
               else {alloc_handler}(4096, 8);\n\
               return 0;\n}}\n"
        ),
    )
    .expect("write the program");
    let program = scratch.join("main");
    let linked = std::process::Command::new("cc")
        .arg(&source)
        .args(["-Wl,--gc-sections"])
        .args(&args)
        .arg(&archive)
        .args(["-lm", "-lpthread", "-ldl", "-o"])
        .arg(&program)
        .status()
        .expect("run cc");
    assert!(linked.success());
    for (mode, expected) in [
        ("panic", "pycc runtime panicked at "),
        ("alloc", "memory allocation of 4096 bytes failed\n"),
    ] {
        let output = std::process::Command::new(&program)
            .arg(mode)
            .env("RUST_BACKTRACE", "1")
            .output()
            .expect("run the program");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.starts_with(expected), "{mode}: {stderr}");
        assert!(!stderr.contains("backtrace"), "{mode}: {stderr}");
        assert_eq!(stderr.lines().count(), if mode == "panic" { 2 } else { 1 }, "{stderr}");
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(output.status.signal(), Some(6), "{mode}: {:?}", output.status);
    }
    let output = std::process::Command::new(&program).arg("panic").output().expect("run");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("the ext shim passes CPython's hex text"), "{stderr}");
    assert!(stderr.contains("crates/pycc_rt/src/ext_bridge.rs:"), "{stderr}");
}
