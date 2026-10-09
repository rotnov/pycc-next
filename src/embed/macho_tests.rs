use super::*;

/// Captured from uv's CPython 3.14.7 (`lib/libpython3.14.dylib`): a dylib
/// lists its own id first.
const UV_LIBPYTHON: &str = "/u/lib/libpython3.14.dylib:
\t/u/lib/libpython3.14.dylib (compatibility version 3.14.0, current version 3.14.0)
\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1351.0.0)
\t/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation (compatibility version 150.0.0, current version 3038.1.255)
";

/// Captured from Homebrew's CPython 3.14.6 (`lib-dynload/_ssl`): an
/// extension module has no id.
const HOMEBREW_SSL: &str = "/h/lib-dynload/_ssl.cpython-314-darwin.so:
\t/opt/homebrew/opt/openssl@3/lib/libssl.3.dylib (compatibility version 3.0.0, current version 3.0.0)
\t/opt/homebrew/opt/openssl@3/lib/libcrypto.3.dylib (compatibility version 3.0.0, current version 3.0.0)
\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1356.0.0)
";

#[test]
fn otool_output_parses_into_install_names() {
    assert_eq!(
        parse_otool_l(UV_LIBPYTHON),
        [
            "/u/lib/libpython3.14.dylib",
            "/usr/lib/libSystem.B.dylib",
            "/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation",
        ]
    );
    assert_eq!(
        parse_otool_l(HOMEBREW_SSL),
        [
            "/opt/homebrew/opt/openssl@3/lib/libssl.3.dylib",
            "/opt/homebrew/opt/openssl@3/lib/libcrypto.3.dylib",
            "/usr/lib/libSystem.B.dylib",
        ]
    );
    assert_eq!(
        parse_otool_l("/x.so:\n\n\t@rpath/libz.dylib\n"),
        ["@rpath/libz.dylib"]
    );
    assert!(parse_otool_l("").is_empty());
}

/// The shape of `otool -L` on a universal image: a header per slice, each
/// slice repeating the same names.
const UNIVERSAL_DYLIB: &str = "/u/lib/libpython3.14.dylib (architecture x86_64):
	/u/lib/libpython3.14.dylib (compatibility version 3.14.0, current version 3.14.0)
	/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1351.0.0)
/u/lib/libpython3.14.dylib (architecture arm64):
	/u/lib/libpython3.14.dylib (compatibility version 3.14.0, current version 3.14.0)
	/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1351.0.0)
";

#[test]
fn universal_otool_output_skips_slice_headers_and_repeats() {
    assert_eq!(
        parse_otool_l(UNIVERSAL_DYLIB),
        ["/u/lib/libpython3.14.dylib", "/usr/lib/libSystem.B.dylib"]
    );
}

fn classify(dep: &str, own_id: Option<&str>) -> MachoDep {
    let bundle_lib = [
        PathBuf::from("/prefix/Python.framework/Versions/3.14/Python"),
        PathBuf::from("/real/prefix/Python.framework/Versions/3.14/Python"),
    ];
    classify_macho_dep(
        dep,
        Path::new(dep),
        own_id,
        Path::new("/prefix"),
        &bundle_lib,
        "libpython3.14.dylib",
    )
}

#[test]
fn system_libraries_and_bundled_names_are_kept() {
    assert_eq!(classify("/usr/lib/libSystem.B.dylib", None), MachoDep::Keep);
    assert_eq!(
        classify(
            "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation",
            None
        ),
        MachoDep::Keep
    );
    assert_eq!(classify("@rpath/libpython3.14.dylib", None), MachoDep::Keep);
    assert_eq!(
        classify("@rpath/libssl.3.dylib", Some("@rpath/libssl.3.dylib")),
        MachoDep::Keep
    );
}

#[test]
fn the_source_libpython_is_rewritten_never_vendored_though_it_is_under_the_prefix() {
    assert_eq!(
        classify("/prefix/Python.framework/Versions/3.14/Python", None),
        MachoDep::RewriteToBundled
    );
    let by_realpath = classify_macho_dep(
        "/link/Python",
        Path::new("/real/prefix/Python.framework/Versions/3.14/Python"),
        None,
        Path::new("/prefix"),
        &[PathBuf::from(
            "/real/prefix/Python.framework/Versions/3.14/Python",
        )],
        "libpython3.14.dylib",
    );
    assert_eq!(by_realpath, MachoDep::RewriteToBundled);
}

#[test]
fn a_prefix_internal_library_is_vendored_and_anything_else_refused() {
    assert_eq!(
        classify("/prefix/lib/libssl.3.dylib", None),
        MachoDep::Vendor(PathBuf::from("/prefix/lib/libssl.3.dylib"))
    );
    assert_eq!(
        classify("/opt/homebrew/opt/openssl@3/lib/libssl.3.dylib", None),
        MachoDep::Refuse
    );
    assert_eq!(classify("@rpath/libtcl9.0.dylib", None), MachoDep::Refuse);
    assert_eq!(
        classify("/prefixed-sibling/lib/x.dylib", None),
        MachoDep::Refuse
    );
}

#[test]
fn the_tool_argument_lists_are_exact() {
    let image = Path::new("/b/lib/libpython3.14.dylib");
    assert_eq!(
        set_id_args("@rpath/libpython3.14.dylib", image),
        [
            "-id",
            "@rpath/libpython3.14.dylib",
            "/b/lib/libpython3.14.dylib"
        ]
    );
    assert_eq!(
        change_args("/old", "@rpath/new", image),
        [
            "-change",
            "/old",
            "@rpath/new",
            "/b/lib/libpython3.14.dylib"
        ]
    );
    assert_eq!(
        codesign_args(image),
        ["-f", "-s", "-", "/b/lib/libpython3.14.dylib"]
    );
}

#[test]
fn a_mach_o_header_is_recognized_by_every_magic_and_nothing_else() {
    for magic in [
        [0xfe, 0xed, 0xfa, 0xce],
        [0xfe, 0xed, 0xfa, 0xcf],
        [0xce, 0xfa, 0xed, 0xfe],
        [0xcf, 0xfa, 0xed, 0xfe],
    ] {
        assert!(is_macho_header(&magic), "{magic:x?}");
    }
    // Fat headers, big-endian and byte-swapped, 32- and 64-bit, with one
    // and with 29 slices.
    assert!(is_macho_header(&[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 2]));
    assert!(is_macho_header(&[0xca, 0xfe, 0xba, 0xbf, 0, 0, 0, 29]));
    assert!(is_macho_header(&[0xbe, 0xba, 0xfe, 0xca, 1, 0, 0, 0]));
    assert!(is_macho_header(&[0xbf, 0xba, 0xfe, 0xca, 3, 0, 0, 0]));
    // A Java `.class` file shares `0xcafebabe` but carries its version.
    assert!(!is_macho_header(&[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 0x34]));
    assert!(!is_macho_header(&[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 0]));
    assert!(!is_macho_header(&[0xca, 0xfe, 0xba, 0xbe, 0, 0]));
    assert!(!is_macho_header(&[0xfe, 0xed]));
    assert!(!is_macho_header(b"#!/bin/sh\n"));
    assert!(!is_macho_header(b""));
}

#[test]
fn otool_d_output_yields_the_install_name_if_there_is_one() {
    assert_eq!(
        parse_otool_d("/c/libx.dylib:\n@rpath/libx.dylib\n"),
        Some("@rpath/libx.dylib".to_string())
    );
    assert_eq!(
        parse_otool_d("/c/libx.dylib (architecture arm64):\n@rpath/libx.dylib\n"),
        Some("@rpath/libx.dylib".to_string())
    );
    assert_eq!(parse_otool_d("/c/ext.so:\n"), None);
    assert_eq!(parse_otool_d(""), None);
}

/// An `otool -l` excerpt: two `LC_RPATH` commands (the second repeated, as
/// a universal image repeats them per slice) around other commands whose
/// own `path`/`name` lines must not be taken for an rpath.
const OTOOL_L_RPATHS: &str = "/c/ext.so:
Load command 11
          cmd LC_LOAD_DYLIB
      cmdsize 56
         name @rpath/libx.dylib (offset 24)
Load command 12
          cmd LC_RPATH
      cmdsize 32
         path /abs/lib (offset 12)
Load command 13
          cmd LC_RPATH
      cmdsize 40
         path @loader_path/.dylibs (offset 12)
Load command 14
          cmd LC_DYLD_ENVIRONMENT
      cmdsize 40
         path not-an-rpath (offset 12)
Load command 15
          cmd LC_RPATH
      cmdsize 40
         path @loader_path/.dylibs (offset 12)
";

#[test]
fn otool_l_output_yields_the_rpaths_in_order_once_each() {
    assert_eq!(
        parse_otool_rpaths(OTOOL_L_RPATHS),
        ["/abs/lib", "@loader_path/.dylibs"]
    );
    assert!(parse_otool_rpaths("/c/ext.so:\n").is_empty());
}
