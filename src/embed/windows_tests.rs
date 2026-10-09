//! The Windows embedded build (D-253) on the Windows-shaped fake layout:
//! the probe, the sidecar, the plan and the refusals, run on every host,
//! so the coverage host drives every Windows arm.

use super::super::fake_layout::{FakeLayout, fake_windows_env, fake_windows_layout};
use super::super::pe::fixture::PeSpec;
use super::super::*;
use super::*;
use pycc_scratch::ScratchDir;

const HOST: (&str, &str) = ("x86_64", "windows");

fn layout(dir: &Path) -> FakeLayout {
    fake_windows_layout(dir)
}

fn toolchain(layout: &FakeLayout) -> EmbedToolchain {
    EmbedToolchain::with_probe("python3.14.exe", layout.probe.clone())
        .with_windows_env(fake_windows_env(layout))
}

fn typed(dir: &Path) -> pycc_hir::HirModule {
    let src = dir.join("m.py");
    std::fs::write(&src, "x = 1\n").expect("write source");
    crate::frontend::resolve_frontend(&src, Some(crate::frontend::NATIVE_MODULE_NAME))
        .unwrap_or_else(|_| panic!("the fixture must type-check"))
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

fn strings(args: &[OsString]) -> Vec<String> {
    args.iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn the_default_interpreter_is_python3_14_exe_on_windows_only() {
    assert_eq!(
        default_interpreter(EmbedPlatform::Windows),
        "python3.14.exe"
    );
    assert_eq!(default_interpreter(EmbedPlatform::MacOs), "python3.14");
    assert_eq!(default_interpreter(EmbedPlatform::Linux), "python3.14");
}

/// A Windows interpreter reports no shared libpython and keeps its DLL
/// beside `python.exe`, not under `LIBDIR`: the Windows probe accepts it,
/// and the POSIX probe refuses the same interpreter.
#[test]
fn the_probe_accepts_the_windows_installation_shape() {
    let dir = ScratchDir::new("embed_windows_probe").expect("scratch");
    let layout = layout(&dir);
    let toolchain = toolchain(&layout);
    assert_eq!(
        toolchain.probe(EmbedPlatform::Windows),
        Ok(layout.probe.clone())
    );
    let posix = toolchain.probe(EmbedPlatform::Linux).expect_err("POSIX");
    assert!(posix.contains("no shared libpython"), "{posix}");
}

#[test]
fn the_probe_refuses_each_missing_windows_file() {
    for (missing, expected) in [
        ("libs/python314.lib", "no import library"),
        ("libs/python3.lib", "no import library"),
        ("python314.dll", "no DLL"),
        ("python3.dll", "no DLL"),
        ("DLLs", "no extension-module directory"),
    ] {
        let dir = ScratchDir::new("embed_windows_probe_missing").expect("scratch");
        let layout = layout(&dir);
        let path = layout.prefix.join(missing);
        if path.is_dir() {
            std::fs::remove_dir_all(&path).expect("remove");
        } else {
            std::fs::remove_file(&path).expect("remove");
        }
        let message = toolchain(&layout)
            .probe(EmbedPlatform::Windows)
            .expect_err(missing);
        assert!(message.contains(expected), "{missing}: {message}");
        let name = Path::new(missing).file_name().expect("a name");
        assert!(
            message.contains(&*name.to_string_lossy()),
            "{missing}: {message}"
        );
    }
}

#[test]
fn the_windows_request_refuses_a_static_libpython_only() {
    let windows = EmbedPlatform::Windows;
    let (shared, stat) = (LibpythonLink::Shared, LibpythonLink::Static);
    assert_eq!(check_windows_request(windows, shared), Ok(()));
    let static_refusal = Err(STATIC_REFUSAL.to_string());
    assert_eq!(check_windows_request(windows, stat), static_refusal);
    for other in [EmbedPlatform::MacOs, EmbedPlatform::Linux] {
        assert_eq!(check_windows_request(other, stat), Ok(()));
    }
    assert!(STATIC_REFUSAL.contains("D-251"));
}

/// The closure-image selector (#1296): a `.pyd` or `.dll` name, or an `MZ`
/// head under any name but `.exe`, each suffix ASCII case-insensitively.
#[test]
fn the_closure_image_selector_matches_what_windows_would_load() {
    for (rel, head, image) in [
        ("fast.cp314-win_amd64.pyd", &b"xx"[..], true),
        ("pkg/FAST.PYD", b"", true),
        ("pkg/sub/x.dll", b"", true),
        ("pkg/sub/X.Dll", b"", true),
        ("pkg/lib/blob.bin", b"MZ\x90\x00", true),
        ("pkg/lib/blob", b"MZ", true),
        ("pkg/cli.exe", b"MZ\x90\x00", false),
        ("pkg/CLI.EXE", b"MZ", false),
        ("pkg/__init__.py", b"import x", false),
        ("pkg/data.bin", b"M", false),
        ("pkg/data.bin", b"", false),
        ("pkg/pyd", b"", false),
    ] {
        assert_eq!(is_windows_image(rel, head), image, "{rel}");
    }
}

/// The launcher's Windows arm appends the locked closure as the third
/// module search path, after `Lib` and `DLLs`, only under the closure
/// define (#1296).
#[test]
fn the_windows_launcher_appends_the_closure_after_lib_and_dlls() {
    let launcher = super::super::LAUNCHER_C;
    let windows = &launcher[launcher.find("#ifdef _WIN32").expect("a Windows arm")..];
    let lib = windows.find(r#"L"%ls\\Lib""#).expect("Lib");
    let dlls = windows.find(r#"L"%ls\\DLLs""#).expect("DLLs");
    let closure = windows.find(r#"L"%ls\\closure""#).expect("closure");
    assert!(lib < dlls && dlls < closure);
    let guard = windows.find("#ifdef PYCC_EMBED_CLOSURE").expect("a guard");
    assert!(guard < closure);
    let append = "PyWideStringList_Append(&config.module_search_paths, ";
    let appends: Vec<usize> = ["lib);", "dlls);", "closure);"]
        .iter()
        .map(|arg| windows.find(&format!("{append}{arg}")).expect(arg))
        .collect();
    assert!(appends[0] < appends[1] && appends[1] < appends[2]);
}

/// The launcher's Windows arm adds the sidecar as a DLL directory before
/// the interpreter initializes, and fails with exit 1 when it cannot
/// (#1305); `<windows.h>` comes from an outer arm of the POSIX-only
/// includes, not a second `#ifdef _WIN32` block.
#[test]
fn the_windows_launcher_adds_the_sidecar_dll_directory_first() {
    let launcher = super::super::LAUNCHER_C.replace("\r\n", "\n");
    assert!(launcher.contains("#endif\n#else\n#include <windows.h>\n#endif\n"));
    let main = &launcher[launcher.find("int pycc_embed_main(").expect("the entry")..];
    let add = main.find("AddDllDirectory(sidecar)").expect("the call");
    let init = main.find("PyConfig_InitIsolatedConfig").expect("the init");
    assert!(add < init);
    let message = "error: pycc could not add the DLL directory %ls (error %lu)";
    let failure = main.find(message).expect("the message");
    let exit = main[failure..].find("return 1;").expect("the exit") + failure;
    assert!(add < failure && exit < init);
}

/// With natives (#1306) the launcher adds `<sidecar>\natives` as a DLL
/// directory right after the sidecar, only under `PYCC_EMBED_NATIVES`, and
/// before the interpreter is configured; a failure exits 1.
#[test]
fn the_windows_launcher_adds_the_natives_dll_directory_under_its_define() {
    let launcher = super::super::LAUNCHER_C.replace("\r\n", "\n");
    let main = &launcher[launcher.find("int pycc_embed_main(").expect("the entry")..];
    let sidecar = main
        .find("AddDllDirectory(sidecar)")
        .expect("the sidecar call");
    let guard = main.find("#ifdef PYCC_EMBED_NATIVES").expect("the guard");
    let form = main.find("L\"%ls\\\\natives\"").expect("the natives path");
    let add = main
        .find("AddDllDirectory(natives)")
        .expect("the natives call");
    let end = guard + main[guard..].find("#endif").expect("the guard's end");
    let init = main.find("PyConfig_InitIsolatedConfig").expect("the init");
    assert!(sidecar < guard && guard < form && form < add && add < end && end < init);
    let failure = add + main[add..].find("return 1;").expect("the exit");
    assert!(failure < end);
}

/// A static libpython request on a Windows host is refused before any
/// interpreter is probed: the interpreter here does not exist.
#[test]
fn a_static_windows_build_is_refused_with_no_interpreter() {
    let dir = ScratchDir::new("embed_windows_static").expect("scratch");
    let toolchain = EmbedToolchain::with_interpreter("/nonexistent/pycc-test-python")
        .with_link(LibpythonLink::Static);
    let message = plan_embed(
        &dir.join("app"),
        &dir.join("m.py"),
        &typed(&dir),
        &toolchain,
        EmbedPlatform::Windows,
        HOST,
        &dir.join("main.o"),
    )
    .expect_err("refused");
    assert_eq!(message, STATIC_REFUSAL);
    assert!(!dir.join("app.pycc").exists());
}

/// The whole Windows plan: the sidecar holds the DLLs at its root and the
/// filtered `Lib\` and `DLLs\`, the marker records the interpreter DLL's
/// digest, the program DLL is compiled without `-fPIC` and linked
/// `-shared` against the import library into the sidecar, and the stub
/// `OUT` is compiled with the static CRT from its own source.
#[test]
fn a_windows_plan_bundles_the_dlls_and_links_a_program_dll_and_a_stub() {
    let dir = ScratchDir::new("embed_windows_plan").expect("scratch");
    let root = std::fs::canonicalize(&*dir).expect("canonicalize");
    std::fs::create_dir(root.join("py")).expect("layout root");
    let layout = layout(&root.join("py"));
    let (out, obj) = (root.join("app"), root.join("main.o"));
    let plan = plan_embed(
        &out,
        &root.join("m.py"),
        &typed(&root),
        &toolchain(&layout),
        EmbedPlatform::Windows,
        HOST,
        &obj,
    )
    .expect("planned");
    let sidecar = root.join("app.pycc");
    assert_eq!(
        relative_files(&sidecar),
        [
            "DLLs/_ssl.pyd",
            "DLLs/libssl-3.dll",
            "DLLs/py.ico",
            "Lib/json/__init__.py",
            "Lib/os.py",
            "PYCC-BUNDLE",
            "python3.dll",
            "python314.dll",
            "vcruntime140.dll",
        ]
    );
    let dll = std::fs::read(layout.prefix.join("python314.dll")).expect("read");
    let digest = sha256::sha256_hex(&dll);
    let marker = std::fs::read_to_string(sidecar.join(layout::MARKER_NAME)).expect("marker");
    assert_eq!(
        marker,
        layout::marker_text(&layout.probe, &digest, LibpythonLink::Shared)
    );
    let compile = strings(&plan.compile_args);
    assert!(!compile.iter().any(|arg| arg == "-fPIC"), "{compile:?}");
    assert_eq!(
        compile[..2],
        ["-I".to_string(), layout.probe.include.display().to_string()]
    );
    let libs = layout.prefix.join("libs").display().to_string();
    assert_eq!(
        strings(&plan.link_args),
        ["-shared", "-L", &libs, "-lpython314", "-Wl,/NOIMPLIB"]
    );
    assert_eq!(plan.artifact, Some(sidecar.join(layout::PROGRAM_DLL_NAME)));
    let stub = plan.stub.as_ref().expect("a Windows plan links a stub");
    let source = root.join(STUB_C_NAME);
    assert_eq!(std::fs::read_to_string(&source).expect("stub"), STUB_C);
    assert_eq!(
        strings(&stub_link_args(stub)),
        [
            "-fms-runtime-lib=static".to_string(),
            source.display().to_string(),
            "-I".to_string(),
            root.display().to_string(),
            "-o".to_string(),
            out.display().to_string(),
        ]
    );
    assert!(root.join(EMBED_CONFIG_INC_NAME).is_file());
}

/// The interpreter scan (#1305) runs in the plan before anything is
/// written: an interpreter image whose import would not resolve once moved
/// is refused, and no sidecar appears.
#[test]
fn a_windows_plan_refuses_an_unrelocatable_interpreter_image_before_staging() {
    let dir = ScratchDir::new("embed_windows_scan_refusal").expect("scratch");
    let root = std::fs::canonicalize(&*dir).expect("canonicalize");
    std::fs::create_dir(root.join("py")).expect("layout root");
    let layout = layout(&root.join("py"));
    let ssl = PeSpec::dll(&["python314.dll", "libcrypto-3.dll"]).bytes();
    std::fs::write(layout.prefix.join("DLLs").join("_ssl.pyd"), ssl).expect("write");
    let err = plan_embed(
        &root.join("app"),
        &root.join("m.py"),
        &typed(&root),
        &toolchain(&layout),
        EmbedPlatform::Windows,
        HOST,
        &root.join("main.o"),
    )
    .expect_err("refused");
    assert!(
        err.contains("is not relocatable: `DLLs\\_ssl.pyd` imports `libcrypto-3.dll`"),
        "{err}"
    );
    assert!(!root.join("app.pycc").exists());
    assert!(!root.join(STUB_C_NAME).exists());
}

/// The stub hard-codes the program DLL's name and includes the generated
/// sidecar header; both must stay in step with the Rust side.
#[test]
fn the_stub_source_loads_the_program_dll_through_the_sidecar_header() {
    assert!(STUB_C.contains(&format!("L\"{}\"", layout::PROGRAM_DLL_NAME)));
    assert!(STUB_C.contains(&format!("#include \"{EMBED_CONFIG_INC_NAME}\"")));
    assert!(STUB_C.contains("\"pycc_embed_main\""));
    assert!(!STUB_C.contains("Python.h"));
}

/// A Windows build with no closure vendors no native library: the
/// interpreter's images are scanned (#1305) and `DLLs\` is bundled as it
/// is; only a closure's images yield natives (#1306).
#[test]
fn a_windows_build_plans_no_native_libraries() {
    let dir = ScratchDir::new("embed_windows_natives").expect("scratch");
    let layout = layout(&dir);
    let plan = plan_natives(
        EmbedPlatform::Windows,
        &layout.probe,
        None,
        &LinuxEnv::host(),
        &fake_windows_env(&layout),
        true,
        LibpythonLink::Shared,
    )
    .expect("planned");
    assert!(plan.linux_vendor.is_empty());
    assert!(plan.locked().is_empty());
}

/// The Windows twin of `a_path_under_a_plain_file_is_an_environment_failure`:
/// Windows reads a sidecar under a plain file as absent, so the build fails
/// at staging instead, leaving neither `OUT` nor a sidecar behind.
#[cfg(windows)]
#[test]
fn a_windows_build_under_a_plain_file_fails_at_staging() {
    let dir = ScratchDir::new("embed_windows_not_a_dir").expect("scratch");
    let root = std::fs::canonicalize(&*dir).expect("canonicalize");
    std::fs::create_dir(root.join("py")).expect("layout root");
    let layout = layout(&root.join("py"));
    let file = root.join("file");
    std::fs::write(&file, "x").expect("write");
    let out = file.join("app");
    let message = plan_embed(
        &out,
        &root.join("m.py"),
        &typed(&root),
        &toolchain(&layout),
        EmbedPlatform::Windows,
        HOST,
        &root.join("main.o"),
    )
    .expect_err("a plain file cannot hold the sidecar");
    assert!(message.contains("could not"), "{message}");
    assert!(!out.exists());
    assert!(!file.join("app.pycc").exists());
}
