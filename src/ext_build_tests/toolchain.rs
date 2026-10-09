//! Host-toolchain tests for the `--ext` build seam: the interpreter version
//! parser and its stable-ABI floor, the `ExtProbe`/`ExtToolchain` discovery
//! path, and the per-platform compile and link argument sets.
//!
//! Split out of `ext_build_tests.rs` by #1049 under `AGENTS.md`'s
//! decomposability rule; the tests themselves are unchanged.

use super::*;

#[test]
fn a_major_minor_line_parses_and_extra_components_are_ignored() {
    assert_eq!(parse_version("3.13"), Some((3, 13)));
    assert_eq!(parse_version(" 3.14.7 \r"), Some((3, 14)));
    assert_eq!(parse_version("4.0"), Some((4, 0)));
}

#[test]
fn a_version_line_without_two_numeric_components_is_rejected() {
    assert_eq!(parse_version("3"), None);
    assert_eq!(parse_version("x.13"), None);
    assert_eq!(parse_version("3.x"), None);
    assert_eq!(parse_version(""), None);
}

#[test]
fn the_floor_admits_the_stable_abi_version_and_everything_newer() {
    assert!(check_floor(MIN_PYTHON).is_ok());
    assert!(check_floor((3, 14)).is_ok());
    assert!(check_floor((4, 0)).is_ok());
}

#[test]
fn the_floor_rejects_an_interpreter_older_than_the_declared_limited_api() {
    let message = check_floor((3, 12)).expect_err("3.12 is below the 3.13 floor");
    assert!(message.contains("3.13"), "{message}");
    assert!(message.contains("3.12"), "{message}");
    assert!(message.contains("Py_LIMITED_API"), "{message}");
    assert!(check_floor((2, 7)).is_err());
}

// ------------------------------------------------------------------ probe

#[test]
fn a_three_line_probe_parses_into_its_three_fields() {
    let parsed = parse_probe_output("3.13\n/usr/include/python3.13\n/opt/py/libs\n")
        .expect("a well-formed probe parses");
    assert_eq!(parsed.version, (3, 13));
    assert_eq!(parsed.include, Path::new("/usr/include/python3.13"));
    assert_eq!(parsed.libs, Path::new("/opt/py/libs"));
}

#[test]
fn a_truncated_or_empty_probe_is_rejected_rather_than_half_believed() {
    assert_eq!(parse_probe_output(""), None);
    assert_eq!(parse_probe_output("3.13\n"), None);
    assert_eq!(parse_probe_output("3.13\n/usr/include\n"), None);
    assert_eq!(parse_probe_output("nope\n/usr/include\n/libs\n"), None);
    // A blank include line parses as three lines but names no directory.
    assert_eq!(parse_probe_output("3.13\n\n/libs\n"), None);
}

#[test]
fn an_override_answers_without_running_any_interpreter() {
    let dir = pycc_scratch::ScratchDir::new("ext-probe").expect("scratch");
    // The probe insists on the header itself, not just the directory.
    std::fs::write(dir.join("Python.h"), "").expect("the fixture header");
    let toolchain =
        ExtToolchain::with_probe("definitely-not-an-interpreter-1036", probe((3, 14), &dir));
    let resolved = toolchain.probe().expect("an override skips the spawn");
    assert_eq!(resolved.version, (3, 14));
    assert_eq!(resolved.include, *dir);
    assert_eq!(
        toolchain.interpreter(),
        std::ffi::OsStr::new("definitely-not-an-interpreter-1036")
    );
}

#[test]
fn an_override_is_still_subject_to_the_version_floor() {
    let dir = pycc_scratch::ScratchDir::new("ext-probe-floor").expect("scratch");
    let toolchain = ExtToolchain::with_probe("python3", probe((3, 12), &dir));
    let message = toolchain.probe().expect_err("3.12 is below the floor");
    assert!(message.contains("3.12"), "{message}");
}

#[test]
fn a_header_directory_that_does_not_exist_is_reported_before_the_compiler_sees_it() {
    let dir = pycc_scratch::ScratchDir::new("ext-probe-missing").expect("scratch");
    let missing = dir.join("no-such-include-dir");
    let toolchain = ExtToolchain::with_probe("python3", probe((3, 13), &missing));
    let message = toolchain
        .probe()
        .expect_err("a missing include dir is an error");
    assert!(message.contains("PYCC_PYTHON_INCLUDE"), "{message}");
}

/// The directory exists and holds files, but not the one that matters. This
/// is what an interpreter installed without its development package reports,
/// and catching it here is what keeps it an environment failure (exit 2)
/// instead of a C compiler error the shared tail reports at exit 1.
#[test]
fn a_header_directory_without_python_h_is_an_environment_failure_not_a_compile_error() {
    let dir = pycc_scratch::ScratchDir::new("ext-probe-headerless").expect("scratch");
    std::fs::write(dir.join("pyconfig.h"), "").expect("a decoy header");
    let toolchain = ExtToolchain::with_probe("python3", probe((3, 13), &dir));
    let message = toolchain
        .probe()
        .expect_err("a directory without Python.h is an error");
    assert!(message.contains("Python.h"), "{message}");
    assert!(message.contains("PYCC_PYTHON_INCLUDE"), "{message}");
}

#[test]
fn an_interpreter_that_cannot_be_spawned_is_an_environment_failure() {
    let toolchain = ExtToolchain::with_interpreter("pycc-no-such-interpreter-1036");
    let message = toolchain
        .probe()
        .expect_err("a missing interpreter is an error");
    assert!(
        message.contains("pycc-no-such-interpreter-1036"),
        "{message}"
    );
    assert!(message.contains("PYCC_PYTHON"), "{message}");
}

/// `/bin/sh` exists and spawns, but rejects the probe script, so this is the
/// one arm that needs a real child process to reach. Unix-only because
/// `/bin/sh` is: the coverage job runs on macOS, so the line is covered.
#[cfg(unix)]
#[test]
fn an_interpreter_that_rejects_the_probe_script_is_reported_with_its_exit_status() {
    let toolchain = ExtToolchain::with_interpreter("/bin/sh");
    let message = toolchain
        .probe()
        .expect_err("sh cannot run the probe script");
    assert!(
        message.contains("failed to report its own configuration"),
        "{message}"
    );
}

/// `/bin/echo` spawns, exits 0, and prints something that is not a probe
/// result -- the unparseable-output arm, distinct from the non-zero one.
#[cfg(unix)]
#[test]
fn an_interpreter_that_prints_nonsense_is_reported_as_unparseable() {
    let toolchain = ExtToolchain::with_interpreter("/bin/echo");
    let message = toolchain.probe().expect_err("echo prints no probe result");
    assert!(
        message.contains("could not\n                 parse") || message.contains("not parse"),
        "{message}"
    );
}

#[test]
fn the_environment_constructor_defaults_to_python3_and_no_override() {
    // Reads the ambient environment deliberately: this is the production
    // constructor, and the assertion holds for either state of the two
    // variables -- an override is honoured when set, and the default
    // interpreter name is `python3` when `PYCC_PYTHON` is not. Expressed as
    // the same `unwrap_or_else` the constructor itself uses rather than as a
    // `match`, because only one arm of a `match` on the ambient environment
    // can ever execute in a given run and the other would be an uncovered
    // line under the D-014/D-242 diff-coverage gate.
    let toolchain = ExtToolchain::from_env();
    let expected =
        std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| std::ffi::OsString::from("python3"));
    assert_eq!(toolchain.interpreter(), expected);
}

// --------------------------------------------------------------- platform

#[test]
fn each_triple_family_selects_its_own_link_platform() {
    assert_eq!(
        ExtLinkPlatform::from_target_triple("x86_64-pc-windows-msvc"),
        ExtLinkPlatform::Windows
    );
    assert_eq!(
        ExtLinkPlatform::from_target_triple("aarch64-apple-darwin"),
        ExtLinkPlatform::MacOs
    );
    assert_eq!(
        ExtLinkPlatform::from_target_triple("x86_64-unknown-linux-gnu"),
        ExtLinkPlatform::Linux
    );
    // An unrecognized triple falls to the ELF default rather than panicking.
    assert_eq!(
        ExtLinkPlatform::from_target_triple("riscv64-unknown-none"),
        ExtLinkPlatform::Linux
    );
}

#[test]
fn no_target_resolves_to_the_host_and_a_target_overrides_it() {
    assert_eq!(ExtLinkPlatform::resolve(None), ExtLinkPlatform::HOST);
    assert_eq!(
        ExtLinkPlatform::resolve(Some("x86_64-pc-windows-msvc")),
        ExtLinkPlatform::Windows
    );
    // Whatever this host is, it is one of the three.
    assert!(matches!(
        ExtLinkPlatform::HOST,
        ExtLinkPlatform::MacOs | ExtLinkPlatform::Linux | ExtLinkPlatform::Windows
    ));
}

#[test]
fn the_three_link_platforms_collapse_onto_the_two_suffix_families() {
    assert!(matches!(
        ExtLinkPlatform::Windows.suffix_platform(),
        ExtPlatform::Windows
    ));
    assert!(matches!(
        ExtLinkPlatform::MacOs.suffix_platform(),
        ExtPlatform::Unix
    ));
    assert!(matches!(
        ExtLinkPlatform::Linux.suffix_platform(),
        ExtPlatform::Unix
    ));
}

#[test]
fn macos_links_a_bundle_with_deferred_symbol_lookup_and_no_libpython() {
    let args = ext_link_args(ExtLinkPlatform::MacOs, Path::new("/unused"));
    assert_eq!(
        args,
        vec![
            OsString::from("-bundle"),
            OsString::from("-undefined"),
            OsString::from("dynamic_lookup"),
        ]
    );
    assert!(
        !args
            .iter()
            .any(|arg| arg.to_string_lossy().contains("python"))
    );
}

#[test]
fn linux_links_a_plain_shared_object_with_no_libpython() {
    let args = ext_link_args(ExtLinkPlatform::Linux, Path::new("/unused"));
    assert_eq!(
        args,
        vec![OsString::from("-shared"), OsString::from("-Wl,-Bsymbolic")]
    );
}

#[test]
fn only_the_elf_arm_asks_the_linker_to_bind_its_own_symbols_first() {
    // `-Bsymbolic` closes ELF's global-scope interposition, which is an ELF
    // problem alone: Mach-O records the defining library per reference and a
    // PE exports only what it declares. `ld64` and `link.exe` both reject
    // the flag, so leaking it onto either arm would break those two hosts
    // outright -- assert its absence rather than trusting the arms not to
    // drift. Reachable from any host because the platform is a parameter.
    for platform in [ExtLinkPlatform::MacOs, ExtLinkPlatform::Windows] {
        let args = ext_link_args(platform, Path::new("/unused"));
        assert!(
            !args
                .iter()
                .any(|arg| arg.to_string_lossy().contains("-Wl,")),
            "{args:?}"
        );
    }
}

#[test]
fn windows_links_against_the_stable_abi_import_library_in_the_probed_directory() {
    let libs = Path::new("C:").join("Python313").join("libs");
    let args = ext_link_args(ExtLinkPlatform::Windows, &libs);
    // Compared as an `OsString` built from `Path::join`, never a rendered
    // path: the separator is the host's, not the target's.
    assert_eq!(
        args,
        vec![
            OsString::from("-shared"),
            OsString::from("-L"),
            libs.as_os_str().to_os_string(),
            OsString::from("-lpython3"),
        ]
    );
}

#[test]
fn the_elf_compile_args_name_the_include_directory_the_shim_and_fpic() {
    let include = Path::new("inc").join("python3.13");
    let shim = Path::new("scratch").join(SHIM_C_NAME);
    let args = ext_compile_args(ExtLinkPlatform::Linux, &include, &shim);
    assert_eq!(
        args,
        vec![
            OsString::from("-I"),
            include.as_os_str().to_os_string(),
            OsString::from("-fPIC"),
            shim.as_os_str().to_os_string(),
        ]
    );
}

/// The Mach-O arm states `-fPIC` too. clang already defaults to it there,
/// so this pins the flag as a property of the artifact rather than one
/// inherited from whichever host happened to build it.
#[test]
fn the_mach_o_compile_args_state_fpic_rather_than_inherit_it() {
    let include = Path::new("inc").join("python3.13");
    let shim = Path::new("scratch").join(SHIM_C_NAME);
    let args = ext_compile_args(ExtLinkPlatform::MacOs, &include, &shim);
    assert_eq!(
        args,
        vec![
            OsString::from("-I"),
            include.as_os_str().to_os_string(),
            OsString::from("-fPIC"),
            shim.as_os_str().to_os_string(),
        ]
    );
}

/// PE/COFF is position independent by construction and its drivers warn
/// that `-fPIC` is ignored, so the Windows arm must not emit it.
#[test]
fn the_windows_compile_args_omit_fpic() {
    let include = Path::new("inc").join("python3.13");
    let shim = Path::new("scratch").join(SHIM_C_NAME);
    let args = ext_compile_args(ExtLinkPlatform::Windows, &include, &shim);
    assert_eq!(
        args,
        vec![
            OsString::from("-I"),
            include.as_os_str().to_os_string(),
            shim.as_os_str().to_os_string(),
        ]
    );
}

/// The C shim maps a pending exception's type tag to a `PyExc_*` object
/// with a `switch` over literal tag values, because it cannot see
/// `pycc_hir::BUILTIN_EXCEPTION_CLASSES` from C. `pycc_rt`'s own
/// `exception_type_tags_match_the_c_shims_hardcoded_switch` pins the flat
/// seven against that crate's constants; this pins the *whole* array against
/// the switch itself, so adding, removing, or reordering a builtin class --
/// which renumbers every tag after it -- fails here instead of silently
/// raising the wrong CPython class out of a built artifact.
#[test]
fn every_exception_tag_the_c_shim_switches_on_still_names_that_class() {
    let mut pending: Option<usize> = None;
    let mut seen = Vec::new();
    for line in SHIM_C.lines().map(str::trim) {
        // A later `switch` in the same file dispatches on named constants;
        // only a decimal label belongs to the exception table.
        if let Some(tag) = line
            .strip_prefix("case ")
            .and_then(|r| r.strip_suffix(':'))
            .and_then(|r| r.parse().ok())
        {
            pending = Some(tag);
        } else if let Some(class) = line
            .strip_prefix("exc_type = PyExc_")
            .and_then(|r| r.strip_suffix(';'))
        {
            // The `default:` arm sets `exc_type` too, and carries no tag.
            if let Some(tag) = pending.take() {
                assert_eq!(
                    pycc_hir::BUILTIN_EXCEPTION_CLASSES.get(tag).copied(),
                    Some(class),
                    "src/ext/pycc_ext_module.c raises {class} for tag {tag}"
                );
                seen.push(tag);
            }
        }
    }
    // Tag 0 (`Exception`) reaches the same `PyExc_Exception` as the unnamed
    // tags through `default:`, and 23..=24 (`BaseExceptionGroup`/
    // `ExceptionGroup`) stay there deliberately -- the shim's own comment
    // carries why. The expected set is therefore non-contiguous: 1..=22 plus
    // 25..=29 (`OverflowError`, #1063; `ImportError`/`ModuleNotFoundError`,
    // #1292; `AssertionError`, #1369; `AttributeError`, #1388), with the
    // 23..=24 hole in between. Widening this to `1..=29` would swallow that hole and stop detecting a
    // group tag that drifted into the switch.
    let mut expected = (1..=22).collect::<Vec<_>>();
    expected.extend([25, 26, 27, 28, 29]);
    assert_eq!(seen, expected);
}

/// The failed-import bridge (#1293) raises its two tags through named
/// `#define`s rather than through the switch above, so that guard cannot see
/// them. This one parses each tag and its class-name literal and pins both
/// against `BUILTIN_EXCEPTION_CLASSES`: renumbering the array, or a name that
/// drifts from its tag, fails here instead of raising a pycc exception that
/// `except ImportError` silently stops matching.
#[test]
fn the_c_shims_import_error_bridge_tags_still_name_their_classes() {
    let define = |name: &str| -> &str {
        let prefix = format!("#define {name} ");
        SHIM_C
            .lines()
            .find_map(|line| line.trim().strip_prefix(prefix.as_str()))
            .expect("src/ext/pycc_ext_module.c defines every bridge constant")
            .trim()
    };
    for (tag, name, class) in [
        (
            "PYCC_EXT_TAG_IMPORT_ERROR",
            "PYCC_EXT_NAME_IMPORT_ERROR",
            "ImportError",
        ),
        (
            "PYCC_EXT_TAG_MODULE_NOT_FOUND_ERROR",
            "PYCC_EXT_NAME_MODULE_NOT_FOUND_ERROR",
            "ModuleNotFoundError",
        ),
    ] {
        let tag: usize = define(tag).parse().expect("a decimal tag");
        assert_eq!(
            pycc_hir::BUILTIN_EXCEPTION_CLASSES.get(tag).copied(),
            Some(class),
            "tag {tag}"
        );
        assert_eq!(define(name), format!("\"{class}\""));
    }
}

/// #1316: the foreign-operation bridge maps a CPython exception to a pycc
/// tag through one `PYCC_EXT_OBJ_TAG(PyExc_X, tag, "Name")` line per class,
/// first match wins. This pins every line's tag and name against
/// `BUILTIN_EXCEPTION_CLASSES`, pins the list to cover every builtin but the
/// two groups, and pins subclass-before-base order through
/// `builtin_exception_parent`: a base listed ahead of its subclass would
/// silently swallow it (`ConnectionError` ahead of `BrokenPipeError` makes
/// `except BrokenPipeError:` stop matching a foreign one).
#[test]
fn the_c_shims_foreign_error_mapping_names_its_classes() {
    let mut mapped: Vec<(usize, &str)> = Vec::new();
    for line in SHIM_C.lines().map(str::trim) {
        let Some(args) = line
            .strip_prefix("PYCC_EXT_OBJ_TAG(")
            .and_then(|r| r.strip_suffix(')'))
        else {
            continue;
        };
        let parts: Vec<&str> = args.split(", ").collect();
        assert_eq!(parts.len(), 3, "malformed mapping line: {line}");
        let (py_class, tag, literal) = (parts[0], parts[1], parts[2]);
        let tag: usize = tag.parse().expect("a decimal tag");
        let class = pycc_hir::BUILTIN_EXCEPTION_CLASSES[tag];
        assert_eq!(py_class, format!("PyExc_{class}"), "tag {tag}");
        assert_eq!(literal, format!("\"{class}\""), "tag {tag}");
        mapped.push((tag, class));
    }
    let mut tags: Vec<usize> = mapped.iter().map(|(tag, _)| *tag).collect();
    tags.sort_unstable();
    let mut expected = (0..=22).collect::<Vec<_>>();
    expected.extend([25, 26, 27, 28, 29]);
    assert_eq!(tags, expected);
    for (position, (_, class)) in mapped.iter().enumerate() {
        let mut ancestor = pycc_hir::builtin_exception_parent(class);
        while let Some(base) = ancestor {
            let base_position = mapped.iter().position(|(_, c)| *c == base);
            assert!(
                base_position.is_none_or(|p| p > position),
                "{base} is mapped before its subclass {class}"
            );
            ancestor = pycc_hir::builtin_exception_parent(base);
        }
    }
    // `Exception` is the last line, so everything that falls past it is a
    // non-`Exception` `BaseException`, which takes the reserved tag.
    assert_eq!(mapped.last(), Some(&(0, "Exception")));
    let foreign_base = SHIM_C
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("#define PYCC_EXT_TAG_FOREIGN_BASE ")
        })
        .expect("the shim defines the reserved tag");
    assert_eq!(
        foreign_base.trim().parse::<u8>().ok(),
        Some(pycc_hir::FOREIGN_BASE_EXCEPTION_TYPE_TAG)
    );
    assert!(
        shim_c().contains(
            "    case PYCC_EXT_TAG_FOREIGN_BASE:\n        exc_type = PyExc_BaseException;"
        ),
        "the raising switch falls back to BaseException for the reserved tag"
    );
}

#[test]
fn the_probe_runs_the_interpreter_in_isolated_mode() {
    let command = probe_command(OsStr::new("python3"), "print(1)");
    let args: Vec<&OsStr> = command.get_args().collect();
    assert_eq!(args, ["-I", "-c", "print(1)"]);
}

#[test]
fn a_shadow_sysconfig_in_the_build_directory_never_runs_during_the_probe() {
    // A project directory that plants `sysconfig.py` must not get it
    // executed by `pycc build` run from inside it.
    let dir = pycc_scratch::ScratchDir::new("probe_shadow").expect("create the build directory");
    std::fs::write(
        dir.join("sysconfig.py"),
        "open(__file__ + '.ran', 'w').close()\n",
    )
    .expect("write the shadow module");
    let python = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    let output = probe_command(&python, "import sysconfig; print(sysconfig.__file__)")
        .current_dir(&*dir)
        .output()
        .expect("run the host interpreter");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let ran = dir.join("sysconfig.py.ran").exists();
    assert!(output.status.success(), "{output:?}");
    assert!(!ran, "the planted sysconfig.py executed");
    assert!(!stdout.contains("probe_shadow"), "{stdout}");
}
