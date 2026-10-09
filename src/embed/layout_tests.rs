use super::*;

fn probe() -> EmbedProbe {
    EmbedProbe {
        version: (3, 14, 7),
        executable: PathBuf::from("/opt/py/bin/python3.14"),
        include: PathBuf::from("/opt/py/include/python3.14"),
        stdlib: PathBuf::from("/opt/py/lib/python3.14"),
        base_prefix: PathBuf::from("/opt/py"),
        enable_shared: true,
        framework: String::new(),
        ldlibrary: "libpython3.14.so".to_string(),
        libdir: PathBuf::from("/nonexistent-pycc/lib"),
        instsoname: "libpython3.14.so.1.0".to_string(),
        gil_disabled: false,
    }
}

#[test]
fn the_host_platform_matches_the_build_host() {
    let expected = if cfg!(target_os = "macos") {
        EmbedPlatform::MacOs
    } else if cfg!(windows) {
        EmbedPlatform::Windows
    } else {
        EmbedPlatform::Linux
    };
    assert_eq!(EmbedPlatform::HOST, expected);
}

#[test]
fn the_sidecar_is_named_after_the_output_file() {
    assert_eq!(
        sidecar_name(Path::new("dist/app")),
        Ok("app.pycc".to_string())
    );
    assert_eq!(
        sidecar_name(Path::new("my app.bin")),
        Ok("my app.bin.pycc".to_string())
    );
}

#[test]
fn a_name_the_loader_cannot_carry_is_refused() {
    // `dir/a:b` rather than `a:b`: on Windows a leading `a:` is a drive
    // prefix, so `a:b`'s file name there is `b`. As a later component the
    // colon stays in the file name on every host.
    for bad in ["a$b", "dir/a:b", "$ORIGIN"] {
        let message = sidecar_name(Path::new(bad)).expect_err("refused");
        assert!(message.contains("contains `$` or `:`"), "{message}");
    }
    let message = sidecar_name(Path::new("/")).expect_err("no file name");
    assert!(message.contains("names no file"), "{message}");
}

#[cfg(unix)]
#[test]
fn a_non_utf8_name_is_refused() {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::OsStr::from_bytes(b"app\xff");
    let message = sidecar_name(Path::new(name)).expect_err("not UTF-8");
    assert!(message.contains("is not UTF-8"), "{message}");
}

#[test]
fn the_sidecar_parent_of_a_bare_name_is_the_current_directory() {
    assert_eq!(sidecar_parent(Path::new("app")), Path::new("."));
    assert_eq!(sidecar_parent(Path::new("dist/app")), Path::new("dist"));
}

#[test]
fn the_rpath_is_relative_to_the_executable_on_each_platform() {
    assert_eq!(
        rpath_args(EmbedPlatform::MacOs, "app.pycc"),
        [
            "-Xlinker",
            "-rpath",
            "-Xlinker",
            "@executable_path/app.pycc/lib"
        ]
    );
    assert_eq!(
        rpath_args(EmbedPlatform::Linux, "app.pycc"),
        ["-Xlinker", "-rpath", "-Xlinker", "$ORIGIN/app.pycc/lib"]
    );
}

#[test]
fn the_linux_preload_links_every_copied_library_by_name() {
    let dir = Path::new("/out/app.pycc/lib");
    let names = ["libnat1.so.1".to_string(), "libz.so.1".to_string()];
    assert_eq!(
        preload_args(EmbedPlatform::Linux, dir, &names),
        [
            "-Xlinker",
            "--push-state",
            "-Xlinker",
            "--no-as-needed",
            "-L",
            "/out/app.pycc/lib",
            "-l:libnat1.so.1",
            "-l:libz.so.1",
            "-Xlinker",
            "--pop-state",
            "-Xlinker",
            "-rpath-link",
            "-Xlinker",
            "/out/app.pycc/lib"
        ]
    );
    assert!(preload_args(EmbedPlatform::Linux, dir, &[]).is_empty());
    assert!(preload_args(EmbedPlatform::MacOs, dir, &names).is_empty());
}

#[test]
fn the_platform_follows_the_host_os() {
    assert_eq!(EmbedPlatform::for_os("macos"), EmbedPlatform::MacOs);
    assert_eq!(EmbedPlatform::for_os("linux"), EmbedPlatform::Linux);
    assert_eq!(EmbedPlatform::for_os("windows"), EmbedPlatform::Windows);
    assert_eq!(EmbedPlatform::for_os("freebsd"), EmbedPlatform::Linux);
}

#[test]
fn the_source_library_follows_the_framework_or_the_plain_layout() {
    let plain = probe();
    assert_eq!(
        source_library(&plain),
        PathBuf::from("/nonexistent-pycc/lib/libpython3.14.so")
    );
    let framework = EmbedProbe {
        framework: "Python".to_string(),
        libdir: PathBuf::from("/F/Python.framework/Versions/3.14/lib"),
        ..probe()
    };
    assert_eq!(
        source_library(&framework),
        PathBuf::from("/F/Python.framework/Versions/3.14/Python")
    );
}

#[test]
fn the_bundled_library_name_depends_on_the_platform() {
    assert_eq!(
        bundled_library_name(EmbedPlatform::MacOs, &probe()),
        "libpython3.14.dylib"
    );
    assert_eq!(
        bundled_library_name(EmbedPlatform::Linux, &probe()),
        "libpython3.14.so.1.0"
    );
    let no_soname = EmbedProbe {
        instsoname: String::new(),
        ..probe()
    };
    assert_eq!(
        bundled_library_name(EmbedPlatform::Linux, &no_soname),
        "libpython3.14.so"
    );
    assert_eq!(stdlib_dir_name(&probe()), "python3.14");
}

#[test]
fn the_marker_records_the_version_the_interpreter_and_the_digest() {
    let text = marker_text(&probe(), "abc123", LibpythonLink::Shared);
    assert_eq!(
        text,
        "pycc-bundle 1\npython 3.14.7\nexecutable /opt/py/bin/python3.14\nlibpython-sha256 abc123\n"
    );
    assert!(marker_is_current(&text));
    assert!(!marker_is_current("pycc-bundle 2\n"));
    assert!(!marker_is_current(""));
}

#[test]
fn a_static_marker_names_the_archive_digest_and_the_link_mode() {
    let text = marker_text(&probe(), "def456", LibpythonLink::Static);
    assert_eq!(
        text,
        "pycc-bundle 1\npython 3.14.7\nexecutable /opt/py/bin/python3.14\n\
         libpython-sha256 def456\nlibpython-link static\n"
    );
    assert!(marker_is_current(&text));
    assert_eq!(LibpythonLink::default(), LibpythonLink::Shared);
}

#[test]
fn a_static_link_loads_the_whole_archive_exports_and_appends_the_libs() {
    let archive = Path::new("/opt/py/lib/python3.14/config-3.14-darwin/libpython3.14.a");
    let libs = [
        "-ldl".to_string(),
        "-framework".to_string(),
        "CoreFoundation".to_string(),
    ];
    let args = |platform| -> Vec<String> {
        static_link_args(platform, archive, &libs)
            .into_iter()
            .map(|arg| arg.into_string().expect("utf-8"))
            .collect()
    };
    let archive = archive.display().to_string();
    assert_eq!(
        args(EmbedPlatform::MacOs),
        [
            "-Xlinker",
            "-force_load",
            "-Xlinker",
            &archive,
            "-Xlinker",
            "-export_dynamic",
            "-ldl",
            "-framework",
            "CoreFoundation",
        ]
    );
    assert_eq!(
        args(EmbedPlatform::Linux),
        [
            "-Xlinker",
            "--whole-archive",
            &archive,
            "-Xlinker",
            "--no-whole-archive",
            "-Xlinker",
            "--export-dynamic",
            "-ldl",
            "-framework",
            "CoreFoundation",
        ]
    );
}

#[test]
fn the_stdlib_copy_skips_exactly_the_filtered_paths() {
    for skipped in [
        "site-packages",
        "site-packages/numpy/__init__.py",
        "__pycache__",
        "json/__pycache__/decoder.cpython-314.pyc",
        "test",
        "test/test_os.py",
        "tkinter",
        "tkinter/ttk.py",
        "turtle.py",
        "idlelib",
        "turtledemo/__main__.py",
        "lib-dynload/_tkinter.cpython-314-darwin.so",
    ] {
        assert!(
            skip_in_stdlib_copy(Path::new(skipped)),
            "{skipped} is skipped"
        );
    }
    for kept in [
        "os.py",
        "json",
        "json/decoder.py",
        "lib-dynload",
        "lib-dynload/_json.cpython-314-darwin.so",
        "unittest/test/__init__.py",
        "turtledemo_notes.txt",
        "_ssl.cp314-win_amd64.pyd",
        "_ssl.cp3t-win_amd64.pyd",
        "_ssl.cp314x-win_amd64.pyd",
        "x.cp3",
        "_d.py",
        "",
    ] {
        assert!(!skip_in_stdlib_copy(Path::new(kept)), "{kept} is kept");
    }
}

#[test]
fn a_c_string_literal_escapes_everything_outside_printable_ascii() {
    assert_eq!(c_string_literal("app.pycc"), "\"app.pycc\"");
    assert_eq!(c_string_literal("my app"), "\"my app\"");
    assert_eq!(c_string_literal("a\"b\\c"), "\"a\\042b\\134c\"");
    assert_eq!(c_string_literal("é1"), "\"\\303\\2511\"");
    assert_eq!(c_string_literal("a\nb"), "\"a\\012b\"");
    assert!(
        embed_config_inc("app.pycc", false, false)
            .contains("#define PYCC_EMBED_SIDECAR \"app.pycc\"\n")
    );
}

#[test]
fn the_closure_define_is_written_only_for_a_closure() {
    assert!(!embed_config_inc("app.pycc", false, false).contains("PYCC_EMBED_CLOSURE"));
    assert!(embed_config_inc("app.pycc", true, false).ends_with("#define PYCC_EMBED_CLOSURE 1\n"));
}

/// Each of the four combinations of the closure and natives defines: the
/// natives define follows the closure's, and neither changes the rest.
#[test]
fn the_natives_define_is_written_only_for_natives() {
    let base = embed_config_inc("app.pycc", false, false);
    let closure = "#define PYCC_EMBED_CLOSURE 1\n";
    let natives = "#define PYCC_EMBED_NATIVES 1\n";
    assert!(!base.contains("PYCC_EMBED_NATIVES"));
    assert_eq!(
        embed_config_inc("app.pycc", true, false),
        format!("{base}{closure}")
    );
    assert_eq!(
        embed_config_inc("app.pycc", false, true),
        format!("{base}{natives}")
    );
    assert_eq!(
        embed_config_inc("app.pycc", true, true),
        format!("{base}{closure}{natives}")
    );
    assert_eq!(WINDOWS_NATIVES_DIR, "natives");
}

#[test]
fn a_closure_loader_relative_reference_climbs_to_the_sidecar_lib() {
    assert_eq!(
        closure_loader_relative("ext.so", "libffi.8.dylib"),
        "@loader_path/../lib/libffi.8.dylib"
    );
    assert_eq!(
        closure_loader_relative("pkg/sub/_ext.so", "libffi.8.dylib"),
        "@loader_path/../../../lib/libffi.8.dylib"
    );
}

#[test]
fn a_loader_relative_reference_climbs_to_the_library_directory() {
    assert_eq!(
        loader_relative(Path::new("libvendor.dylib"), "libinner.dylib"),
        "@loader_path/libinner.dylib"
    );
    assert_eq!(
        loader_relative(
            Path::new("python3.14/lib-dynload/_ssl.so"),
            "libssl.3.dylib"
        ),
        "@loader_path/../../libssl.3.dylib"
    );
}

#[test]
fn a_sidecar_loader_relative_reference_climbs_only_past_the_shared_directories() {
    // The same directory, a deeper one, a shallower one, and from `lib/`
    // into `closure/`.
    assert_eq!(
        sidecar_loader_relative("closure/pkg/_ext.so", "closure/pkg/libx.dylib"),
        "@loader_path/libx.dylib"
    );
    assert_eq!(
        sidecar_loader_relative("closure/pkg/_ext.so", "closure/pkg/.dylibs/libx.dylib"),
        "@loader_path/.dylibs/libx.dylib"
    );
    assert_eq!(
        sidecar_loader_relative("closure/pkg/sub/_ext.so", "closure/other/libx.dylib"),
        "@loader_path/../../other/libx.dylib"
    );
    assert_eq!(
        sidecar_loader_relative("lib/libout.dylib", "closure/pkg/.dylibs/libx.dylib"),
        "@loader_path/../closure/pkg/.dylibs/libx.dylib"
    );
}

/// A Windows-shaped probe: the DLL beside `python.exe`, the import
/// libraries in `libs`.
fn windows_probe() -> EmbedProbe {
    EmbedProbe {
        executable: PathBuf::from("C:/Py/python.exe"),
        include: PathBuf::from("C:/Py/Include"),
        stdlib: PathBuf::from("C:/Py/Lib"),
        base_prefix: PathBuf::from("C:/Py"),
        enable_shared: false,
        ldlibrary: "python314.dll".to_string(),
        libdir: PathBuf::from("C:/Py/libs"),
        instsoname: String::new(),
        ..probe()
    }
}

/// Every linker-argument helper a Windows embedded build reaches, or
/// provably does not reach, gives its Windows value (D-253).
#[test]
fn the_windows_link_helpers_add_no_rpath_preload_pic_or_static_arguments() {
    let names = ["libz.so.1".to_string()];
    assert!(rpath_args(EmbedPlatform::Windows, "app.pycc").is_empty());
    assert!(preload_args(EmbedPlatform::Windows, Path::new("lib"), &names).is_empty());
    assert!(pic_args(EmbedPlatform::Windows).is_empty());
    assert_eq!(pic_args(EmbedPlatform::MacOs), ["-fPIC"]);
    assert_eq!(pic_args(EmbedPlatform::Linux), ["-fPIC"]);
    assert!(archive_load_args(EmbedPlatform::Windows, Path::new("python314.lib")).is_empty());
    assert!(export_args(EmbedPlatform::Windows).is_empty());
}

/// The Windows interpreter DLL sits beside `python.exe` and keeps its own
/// name at the sidecar root; the other platforms keep `<sidecar>/lib`.
#[test]
fn the_windows_library_is_the_dll_beside_python_at_the_sidecar_root() {
    let windows = windows_probe();
    assert_eq!(
        windows_interpreter_dll(&windows),
        PathBuf::from("C:/Py/python314.dll")
    );
    assert_eq!(
        bundled_library_name(EmbedPlatform::Windows, &windows),
        "python314.dll"
    );
    let sidecar = Path::new("out/app.pycc");
    assert_eq!(
        bundled_library_path(EmbedPlatform::Windows, sidecar, &windows),
        sidecar.join("python314.dll")
    );
    assert_eq!(
        bundled_library_path(EmbedPlatform::Linux, sidecar, &probe()),
        sidecar.join("lib").join("libpython3.14.so.1.0")
    );
    assert_eq!(PROGRAM_DLL_NAME, "pycc_program.dll");
    assert_eq!(
        windows_runtime_dlls(),
        ["vcruntime140.dll", "vcruntime140_1.dll"]
    );
}

/// The root set is the interpreter's DLL, the stable-ABI DLL, then only
/// the VC runtime DLLs present in `base_prefix`.
#[test]
fn the_windows_root_dlls_are_the_interpreter_pair_and_the_present_runtime() {
    let dir = pycc_scratch::ScratchDir::new("embed_windows_root_dlls").expect("scratch");
    std::fs::write(dir.join("vcruntime140_1.dll"), b"MZ").expect("write");
    let probe = EmbedProbe {
        base_prefix: dir.to_path_buf(),
        ldlibrary: "python314.dll".to_string(),
        ..probe()
    };
    assert_eq!(
        windows_root_dlls(&probe),
        ["python314.dll", "python3.dll", "vcruntime140_1.dll"]
    );
}

#[test]
fn the_windows_stdlib_copy_skips_exactly_the_filtered_paths() {
    for skipped in [
        "__pycache__",
        "json/__pycache__/decoder.cpython-314.pyc",
        "site-packages",
        "Site-Packages/numpy/__init__.py",
        "test",
        "test/test_os.py",
        "tkinter",
        "TkInter/ttk.py",
        "turtle.py",
        "idlelib",
        "_tkinter.pyd",
        "_tkinter.pdb",
        "tcl86t.dll",
        "TK86T.DLL",
        "tcl9tk9.0.dll",
        "_ssl.cp314t-win_amd64.pyd",
        "sub/_Ctypes.CP314T-win_amd64.PYD",
        "_ssl_d.pyd",
        "_SSL_D.PYD",
        "python314_d.dll",
    ] {
        assert!(
            skip_in_windows_stdlib_copy(Path::new(skipped)),
            "{skipped} is skipped"
        );
    }
    for kept in [
        "os.py",
        "json",
        "json/decoder.py",
        "_ssl.pyd",
        "libssl-3.dll",
        "tclsh.exe",
        "tcl86t.lib",
        "tk.dll",
        "unittest/test/__init__.py",
        "turtledemo_notes.txt",
        "_ssl.cp314-win_amd64.pyd",
        "_ssl.cp3t-win_amd64.pyd",
        "_ssl.cp314x-win_amd64.pyd",
        "x.cp3",
        "_d.py",
        "",
    ] {
        assert!(
            !skip_in_windows_stdlib_copy(Path::new(kept)),
            "{kept} is kept"
        );
    }
}
