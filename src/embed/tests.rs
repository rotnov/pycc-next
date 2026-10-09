//! Tests for the embed probe, `plan_embed` and the sidecar assembly. The
//! effectful ones drive a fake interpreter layout (`fake_layout.rs`), so
//! they run on the coverage host with no CPython installed.

use super::fake_layout::{FakeLayout, fake_layout};
use super::*;
use pycc_scratch::ScratchDir;

fn probe_lines(layout: &FakeLayout) -> String {
    let probe = &layout.probe;
    format!(
        "3.14.7\n{}\n{}\n{}\n{}\n1\n\nlibpython3.14.dylib\n{}\nlibpython3.14.so.1.0\n0\n",
        probe.executable.display(),
        probe.include.display(),
        probe.stdlib.display(),
        probe.base_prefix.display(),
        probe.libdir.display()
    )
}

/// The host the lock section is selected by; a stdlib-only program with no
/// lock never uses it.
const HOST: (&str, &str) = ("aarch64", "macos");

fn typed(dir: &Path) -> pycc_hir::HirModule {
    let src = dir.join("m.py");
    std::fs::write(&src, "class Boom(Exception):\n    pass\n\nx = 1\n").expect("write source");
    crate::frontend::resolve_frontend(&src, Some(crate::frontend::NATIVE_MODULE_NAME))
        .unwrap_or_else(|_| panic!("the fixture must type-check"))
}

#[test]
fn the_probe_output_parses_into_every_field() {
    let dir = ScratchDir::new("embed_parse").expect("scratch");
    let layout = fake_layout(&dir);
    assert_eq!(parse_embed_probe(&probe_lines(&layout)), Some(layout.probe));
}

#[test]
fn a_framework_build_reports_shared_through_its_framework_name() {
    let text = "3.14.6\n/x/python3.14\n/x/include\n/x/lib/python3.14\n/x\n0\nPython\n\
                Python.framework/Versions/3.14/Python\n/x/lib\n\n1\n";
    let probe = parse_embed_probe(text).expect("parses");
    assert!(!probe.enable_shared);
    assert_eq!(probe.framework, "Python");
    assert!(probe.instsoname.is_empty());
    assert!(probe.gil_disabled);
}

#[test]
fn a_truncated_or_malformed_probe_output_does_not_parse() {
    let dir = ScratchDir::new("embed_parse_bad").expect("scratch");
    let lines = probe_lines(&fake_layout(&dir));
    let truncated: String = lines
        .lines()
        .take(10)
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(parse_embed_probe(&truncated), None);
    assert_eq!(
        parse_embed_probe(&lines.replacen("3.14.7", "3.x.7", 1)),
        None
    );
    assert_eq!(
        parse_embed_probe(&lines.replacen("3.14.7", "3.14", 1)),
        None
    );
    assert_eq!(parse_embed_probe(""), None);
}

#[test]
fn only_the_3_14_line_is_accepted() {
    assert_eq!(check_embed_version((3, 14, 0)), Ok(()));
    assert_eq!(check_embed_version((3, 14, 7)), Ok(()));
    let older = check_embed_version((3, 13, 5)).expect_err("3.13 is refused");
    assert!(
        older.contains("CPython 3.13.5") && older.contains("PYCC_PYTHON"),
        "{older}"
    );
    assert!(check_embed_version((3, 15, 0)).is_err());
}

#[test]
fn a_library_is_shared_when_enabled_or_when_it_is_a_framework() {
    assert!(check_shared(true, ""));
    assert!(check_shared(false, "Python"));
    assert!(!check_shared(false, ""));
}

#[test]
fn the_production_toolchain_reads_the_environment_without_probing() {
    assert!(EmbedToolchain::from_env().probe_override.is_none());
}

#[test]
fn a_complete_layout_passes_every_probe_check() {
    let dir = ScratchDir::new("embed_probe_ok").expect("scratch");
    let layout = fake_layout(&dir);
    let toolchain = EmbedToolchain::with_probe("unused", layout.probe.clone());
    assert_eq!(toolchain.probe(EmbedPlatform::MacOs), Ok(layout.probe));
}

/// Each refusal the probe makes, one layout defect at a time.
#[test]
fn each_probe_refusal_names_its_reason() {
    let dir = ScratchDir::new("embed_probe_refusals").expect("scratch");
    let layout = fake_layout(&dir);
    let refusal = |edit: &dyn Fn(&mut EmbedProbe)| {
        let mut probe = layout.probe.clone();
        edit(&mut probe);
        EmbedToolchain::with_probe("pyfake", probe)
            .probe(EmbedPlatform::MacOs)
            .expect_err("the defect is refused")
    };
    assert!(refusal(&|p| p.version = (3, 13, 1)).contains("CPython 3.13.1"));
    let free_threaded = refusal(&|p| p.gil_disabled = true);
    assert!(free_threaded.contains("free-threaded") && free_threaded.contains("Py_GIL_DISABLED=1"));
    let static_only = refusal(&|p| p.enable_shared = false);
    assert!(
        static_only.contains("no shared libpython") && static_only.contains("Py_ENABLE_SHARED=0")
    );
    let no_header = refusal(&|p| p.include = p.include.join("missing"));
    assert!(no_header.contains("no `Python.h`"), "{no_header}");
    let no_library = refusal(&|p| p.ldlibrary = "libmissing.dylib".to_string());
    assert!(no_library.contains("libmissing.dylib") && no_library.contains("does not exist"));
    let no_stdlib = refusal(&|p| p.stdlib = p.stdlib.join("missing"));
    assert!(no_stdlib.contains("standard library") && no_stdlib.contains("does not exist"));
}

#[test]
fn an_interpreter_that_cannot_start_is_an_environment_failure() {
    let message = EmbedToolchain::with_interpreter("/nonexistent/pycc-test-python")
        .probe(EmbedPlatform::MacOs)
        .expect_err("nothing to run");
    assert!(
        message.contains("could not run the embed interpreter"),
        "{message}"
    );
    assert!(message.contains("PYCC_PYTHON"), "{message}");
}

/// Writes an executable shell script standing in for an interpreter.
#[cfg(unix)]
fn fake_interpreter(dir: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("fake-python");
    std::fs::write(&script, format!("#!/bin/sh\n{body}")).expect("write the script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    script
}

/// The probe's success path through a real spawn: `with_probe` skips the
/// spawn, and the coverage host has no `python3.14` to reach it.
#[cfg(unix)]
#[test]
fn a_spawned_probe_reports_the_layout_and_passes_every_check() {
    let dir = ScratchDir::new("embed_probe_spawn").expect("scratch");
    let layout = fake_layout(&dir);
    let lines = probe_lines(&layout);
    let script = fake_interpreter(&dir, &format!("cat <<'PYCC'\n{lines}PYCC\n"));
    let probe = EmbedToolchain::with_interpreter(script).probe(EmbedPlatform::MacOs);
    assert_eq!(probe, Ok(layout.probe));
}

#[cfg(unix)]
#[test]
fn a_spawned_probe_that_fails_or_prints_garbage_is_refused() {
    let dir = ScratchDir::new("embed_probe_garbage").expect("scratch");
    let garbage = fake_interpreter(&dir, "echo garbage\n");
    let message = EmbedToolchain::with_interpreter(&garbage)
        .probe(EmbedPlatform::MacOs)
        .expect_err("garbage");
    assert!(message.contains("did not report a configuration") && message.contains("exit 0"));
    let failing = fake_interpreter(&dir, "exit 3\n");
    let message = EmbedToolchain::with_interpreter(failing)
        .probe(EmbedPlatform::MacOs)
        .expect_err("a failed probe");
    assert!(message.contains("exit 3"), "{message}");
}

#[test]
fn an_unwritable_build_source_is_an_environment_failure() {
    let dir = ScratchDir::new("embed_write").expect("scratch");
    // A directory cannot be overwritten as a file.
    let message = write_source(&dir, "x").expect_err("a directory");
    assert!(
        message.contains("could not write the embedded build source"),
        "{message}"
    );
}

#[test]
fn an_unmarked_sidecar_is_refused_before_the_probe_and_left_intact() {
    let dir = ScratchDir::new("embed_unmarked").expect("scratch");
    let sidecar = dir.join("app.pycc");
    std::fs::create_dir(&sidecar).expect("create");
    std::fs::write(sidecar.join("mine.txt"), "user data").expect("write");
    // The interpreter does not exist: reaching the probe would fail with a
    // different message, so this also pins the order.
    let toolchain = EmbedToolchain::with_interpreter("/nonexistent/pycc-test-python");
    let message = plan_embed(
        &dir.join("app"),
        &dir.join("m.py"),
        &typed(&dir),
        &toolchain,
        EmbedPlatform::Linux,
        HOST,
        &dir.join("main.o"),
    )
    .expect_err("unmarked");
    assert!(
        message.contains("not a bundle this pycc wrote"),
        "{message}"
    );
    assert_eq!(
        std::fs::read_to_string(sidecar.join("mine.txt")).expect("intact"),
        "user data"
    );
}

#[test]
fn an_old_format_marker_and_a_plain_file_are_both_unmarked() {
    let dir = ScratchDir::new("embed_old_marker").expect("scratch");
    let sidecar = dir.join("app.pycc");
    std::fs::create_dir(&sidecar).expect("create");
    std::fs::write(sidecar.join(layout::MARKER_NAME), "pycc-bundle 0\n").expect("write");
    assert!(bundle::check_existing(&sidecar).is_err());
    let file = dir.join("file.pycc");
    std::fs::write(&file, "x").expect("write");
    assert!(bundle::check_existing(&file).is_err());
    assert_eq!(bundle::check_existing(&dir.join("absent.pycc")), Ok(false));
}

/// A path that cannot even be inspected (its parent is a plain file) is an
/// environment failure, not an absent sidecar. POSIX reports `ENOTDIR`
/// there; Windows reports the path as not found, so it reads as absent and
/// the build fails one step later, at staging (D-253's
/// `a_windows_build_under_a_plain_file_fails_at_staging` twin).
#[cfg(unix)]
#[test]
fn a_path_under_a_plain_file_is_an_environment_failure() {
    let dir = ScratchDir::new("embed_not_a_dir").expect("scratch");
    let file = dir.join("file.pycc");
    std::fs::write(&file, "x").expect("write");
    let err = bundle::check_existing(&file.join("app.pycc")).expect_err("not a directory");
    assert!(err.contains("could not inspect"), "{err}");
}

#[cfg(unix)]
#[test]
fn a_symlinked_sidecar_is_refused_and_not_followed() {
    let dir = ScratchDir::new("embed_symlink").expect("scratch");
    let real = dir.join("real");
    std::fs::create_dir(&real).expect("create");
    std::fs::write(real.join(layout::MARKER_NAME), "pycc-bundle 1\n").expect("write");
    let link = dir.join("app.pycc");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");
    assert!(bundle::check_existing(&link).is_err());
    assert!(bundle::check_existing(&real).is_ok_and(|marked| marked));
}

#[test]
fn a_sidecar_name_the_loader_cannot_carry_is_refused_first() {
    let dir = ScratchDir::new("embed_bad_name").expect("scratch");
    let toolchain = EmbedToolchain::with_interpreter("/nonexistent/pycc-test-python");
    let message = plan_embed(
        &dir.join("a$b"),
        &dir.join("m.py"),
        &typed(&dir),
        &toolchain,
        EmbedPlatform::Linux,
        HOST,
        &dir.join("main.o"),
    )
    .expect_err("`$` is refused");
    assert!(message.contains("contains `$` or `:`"), "{message}");
}

fn relative_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(root).expect("under root");
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}

/// The Linux arm end to end on any host: no Mach-O tool runs, the library
/// keeps its SONAME, and the executable finds it through `$ORIGIN`.
#[test]
fn a_linux_bundle_copies_the_filtered_stdlib_and_the_soname_library() {
    let dir = ScratchDir::new("embed_linux").expect("scratch");
    let layout = fake_layout(&dir);
    let toolchain = EmbedToolchain::with_probe("pyfake", layout.probe.clone());
    let out = dir.join("app");
    let obj = dir.join("main.o");
    let plan = plan_embed(
        &out,
        &dir.join("m.py"),
        &typed(&dir),
        &toolchain,
        EmbedPlatform::Linux,
        HOST,
        &obj,
    )
    .expect("planned");
    let sidecar = dir.join("app.pycc");
    assert_eq!(
        relative_files(&sidecar),
        [
            "PYCC-BUNDLE",
            "lib/libpython3.14.so.1.0",
            "lib/python3.14/json/__init__.py",
            "lib/python3.14/os.py",
        ]
    );
    let marker = std::fs::read_to_string(sidecar.join("PYCC-BUNDLE")).expect("marker");
    assert!(layout::marker_is_current(&marker));
    assert!(marker.contains("python 3.14.7\n"), "{marker}");
    assert!(
        marker.contains(&sha256::sha256_hex(b"not a real library")),
        "{marker}"
    );
    let library = sidecar.join("lib").join("libpython3.14.so.1.0");
    assert_eq!(plan.link_args[0], library.into_os_string());
    assert_eq!(
        plan.link_args.last(),
        Some(&OsString::from("$ORIGIN/app.pycc/lib"))
    );
    assert_eq!(
        plan.compile_args[..3],
        [
            OsString::from("-I"),
            layout.probe.include.into_os_string(),
            OsString::from("-fPIC")
        ]
    );
    let shim = obj.with_file_name(ext_build::SHIM_C_NAME);
    let launcher = obj.with_file_name(LAUNCHER_C_NAME);
    assert_eq!(
        plan.compile_args[3..],
        [shim.into_os_string(), launcher.into_os_string()]
    );
    let config = std::fs::read_to_string(obj.with_file_name(EMBED_CONFIG_INC_NAME)).expect("inc");
    assert!(
        config.contains("#define PYCC_EMBED_SIDECAR \"app.pycc\""),
        "{config}"
    );
    let exports =
        std::fs::read_to_string(obj.with_file_name(ext_build::EXPORTS_INC_NAME)).expect("inc");
    assert!(
        exports.contains("#define PYCC_EXT_MODULE_NAME __main__"),
        "{exports}"
    );
    assert!(
        exports.contains("Boom"),
        "the user exception class is registered: {exports}"
    );

    // A rebuild replaces the marked sidecar and leaves nothing beside it.
    std::fs::write(sidecar.join("stale.txt"), "old").expect("write");
    plan_embed(
        &out,
        &dir.join("m.py"),
        &typed(&dir),
        &toolchain,
        EmbedPlatform::Linux,
        HOST,
        &obj,
    )
    .expect("replanned");
    assert!(!sidecar.join("stale.txt").exists());
    let names: Vec<String> = std::fs::read_dir(&*dir)
        .expect("read_dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.starts_with("app.pycc"))
        .collect();
    assert_eq!(names, ["app.pycc"]);
}

#[test]
fn a_failed_assembly_removes_its_staging_directory() {
    let dir = ScratchDir::new("embed_staging").expect("scratch");
    let layout = fake_layout(&dir);
    // The probe passed, but the library vanished before the copy.
    std::fs::remove_file(layout.library()).expect("remove");
    let message = bundle::assemble(
        &layout.probe,
        EmbedPlatform::Linux,
        &dir,
        "app.pycc",
        false,
        None,
        &native::NativePlan::default(),
        None,
    )
    .expect_err("no library");
    assert!(message.contains("could not read"), "{message}");
    let leftovers: Vec<_> = std::fs::read_dir(&*dir)
        .expect("read_dir")
        .map(|entry| entry.expect("entry").file_name())
        .filter(|name| name.to_string_lossy().starts_with("app.pycc"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn a_failed_replacement_removes_its_staging_directory() {
    let dir = ScratchDir::new("embed_replace_staging").expect("scratch");
    let layout = fake_layout(&dir);
    // Asked to replace a sidecar that is not there: moving it aside fails
    // after the staging directory was fully populated.
    let message = bundle::assemble(
        &layout.probe,
        EmbedPlatform::Linux,
        &dir,
        "app.pycc",
        true,
        None,
        &native::NativePlan::default(),
        None,
    )
    .expect_err("nothing to move aside");
    assert!(message.contains("could not move aside"), "{message}");
    let leftovers: Vec<_> = std::fs::read_dir(&*dir)
        .expect("read_dir")
        .map(|entry| entry.expect("entry").file_name())
        .filter(|name| name.to_string_lossy().starts_with("app.pycc"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn a_failed_final_move_restores_the_previous_sidecar() {
    let dir = ScratchDir::new("embed_restore").expect("scratch");
    let sidecar = dir.join("app.pycc");
    std::fs::create_dir(&sidecar).expect("sidecar");
    std::fs::write(sidecar.join("PYCC-BUNDLE"), "previous").expect("marker");
    let missing_staging = dir.join("app.pycc.tmp-missing");
    let message = bundle::swap_into_place(&missing_staging, &sidecar, &dir, "app.pycc", true)
        .expect_err("no staging directory");
    assert!(message.contains("could not move into place"), "{message}");
    assert_eq!(
        std::fs::read_to_string(sidecar.join("PYCC-BUNDLE")).expect("restored"),
        "previous"
    );
    let names: Vec<_> = std::fs::read_dir(&*dir)
        .expect("read_dir")
        .map(|entry| entry.expect("entry").file_name())
        .filter(|name| name.to_string_lossy().starts_with("app.pycc"))
        .collect();
    assert_eq!(names, ["app.pycc"]);
}

#[test]
fn a_failed_restore_names_where_the_previous_sidecar_was_left() {
    let old = Path::new("/out/app.pycc.old-7");
    assert_eq!(bundle::stranded_note(true, old), "");
    assert_eq!(
        bundle::stranded_note(false, old),
        "; the previous sidecar is left at `/out/app.pycc.old-7`"
    );
}

#[cfg(unix)]
#[test]
fn a_tool_that_fails_or_cannot_start_is_an_environment_failure() {
    let failed = bundle::run_tool(
        "sh",
        &[
            OsString::from("-c"),
            OsString::from("echo nope >&2; exit 4"),
        ],
    )
    .expect_err("exit 4");
    assert!(failed.contains("`sh` failed") && failed.contains("exit 4") && failed.contains("nope"));
    let missing = bundle::run_tool("pycc-no-such-tool", &[]).expect_err("missing");
    assert!(
        missing.contains("could not run `pycc-no-such-tool`"),
        "{missing}"
    );
    assert_eq!(
        bundle::run_tool("sh", &[OsString::from("-c"), OsString::from("echo hi")]),
        Ok("hi\n".to_string())
    );
}

#[cfg(target_os = "macos")]
#[path = "macos_tests.rs"]
mod macos_tests;

#[path = "lock_tests.rs"]
mod lock_tests;

#[path = "static_embed_tests.rs"]
mod static_embed_tests;
