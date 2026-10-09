//! The Windows interpreter-image scan on the Windows-shaped fake layout,
//! with its system directory injected: every resolution class, every
//! refusal, and the host-independent paths in the messages. Runs on every
//! host.

use super::super::fake_layout::{FakeLayout, fake_windows_env, fake_windows_layout};
use super::super::pe::fixture::PeSpec;
use super::*;
use pycc_scratch::ScratchDir;

const CRT: &str = "api-ms-win-crt-runtime-l1-1-0.dll";

struct Fixture {
    _dir: ScratchDir,
    layout: FakeLayout,
    env: WindowsEnv,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir = ScratchDir::new(tag).expect("scratch");
        let layout = fake_windows_layout(&dir);
        let env = fake_windows_env(&layout);
        Self {
            _dir: dir,
            layout,
            env,
        }
    }

    /// Writes `bytes` at `rel` under the interpreter prefix.
    fn put(&self, rel: &str, bytes: impl AsRef<[u8]>) {
        let path = self.layout.prefix.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    /// Replaces `DLLs\_ssl.pyd` with a DLL importing `imports` and
    /// delay-importing `delay`.
    fn ssl(&self, imports: &[&str], delay: &[&str]) {
        self.put("DLLs/_ssl.pyd", PeSpec::dll(imports).delay(delay).bytes());
    }

    fn scan(&self) -> Result<(), String> {
        scan_interpreter(&self.layout.probe, &self.env)
    }

    fn refused(&self) -> String {
        self.scan().expect_err("refused")
    }
}

#[test]
fn the_system_directory_is_under_the_system_root_or_the_default() {
    let from = |root: Option<&str>| WindowsEnv::from_system_root(root.map(OsString::from));
    assert_eq!(
        from(Some("D:\\Win")).system_dirs,
        [PathBuf::from("D:\\Win\\System32")]
    );
    assert_eq!(
        from(None).system_dirs,
        [PathBuf::from("C:\\Windows\\System32")]
    );
    assert_eq!(
        WindowsEnv::host(),
        WindowsEnv::from_system_root(std::env::var_os("SystemRoot"))
    );
}

/// The fake install scans clean: root DLLs importing each other, API sets,
/// system files, a case-mismatched `VCRUNTIME140.dll` against the on-disk
/// `vcruntime140.dll`, a sibling, a delay-loaded system DLL, and a
/// non-image `py.ico` beside them.
#[test]
fn the_fake_install_scans_clean() {
    let fx = Fixture::new("native_windows_clean");
    assert!(fx.layout.prefix.join("DLLs/py.ico").is_file());
    assert_eq!(fx.scan(), Ok(()));
}

/// An import outside every class is refused, naming the interpreter, the
/// image and the import.
#[test]
fn an_unknown_import_is_refused() {
    let fx = Fixture::new("native_windows_unknown");
    fx.ssl(&["python314.dll", "libcrypto-3.dll"], &[]);
    let exe = fx.layout.probe.executable.display();
    assert_eq!(
        fx.refused(),
        format!(
            "the embed interpreter `{exe}` is not relocatable: `DLLs\\_ssl.pyd` imports \
             `libcrypto-3.dll`, which is neither a system DLL, a DLL the sidecar bundles, nor a \
             sibling in `DLLs\\`; pycc cannot bundle it"
        )
    );
}

/// The Tcl DLL the copy skips is no sibling; neither is a kept file that is
/// not an image.
#[test]
fn a_skipped_or_non_image_sibling_is_refused() {
    let fx = Fixture::new("native_windows_skipped_sibling");
    fx.ssl(&["tcl86t.dll"], &[]);
    assert!(fx.refused().contains("imports `tcl86t.dll`"));
    fx.put("DLLs/notes.txt", "notes");
    fx.ssl(&["notes.txt"], &[]);
    assert!(fx.refused().contains("imports `notes.txt`"));
}

/// A system-directory file resolves, case-insensitively; without the
/// system directory it does not, and a missing directory counts as empty.
#[test]
fn a_system_directory_file_resolves_only_when_present() {
    let fx = Fixture::new("native_windows_system");
    fx.ssl(&["ws2_32.DLL"], &[]);
    assert_eq!(fx.scan(), Ok(()));
    let env = WindowsEnv {
        system_dirs: vec![fx.layout.prefix.join("no-such-dir")],
    };
    let err = scan_interpreter(&fx.layout.probe, &env).expect_err("refused");
    assert!(
        err.contains("`python314.dll` imports `KERNEL32.dll`"),
        "{err}"
    );
}

/// `vcruntime140_1.dll` is a root DLL only when `base_prefix` has it;
/// otherwise it must be a system file.
#[test]
fn an_absent_optional_runtime_dll_must_be_a_system_file() {
    let fx = Fixture::new("native_windows_vcruntime_1");
    fx.ssl(&["VCRUNTIME140_1.dll"], &[]);
    assert!(fx.refused().contains("imports `VCRUNTIME140_1.dll`"));
    let system = fx.env.system_dirs[0].join("vcruntime140_1.dll");
    std::fs::write(&system, "MZ").expect("write");
    assert_eq!(fx.scan(), Ok(()));
    std::fs::remove_file(&system).expect("remove");
    fx.put("vcruntime140_1.dll", PeSpec::dll(&["KERNEL32.dll"]).bytes());
    assert_eq!(fx.scan(), Ok(()));
}

#[test]
fn an_image_of_the_wrong_kind_is_refused() {
    let fx = Fixture::new("native_windows_kinds");
    let libssl = |spec: PeSpec| fx.put("DLLs/libssl-3.dll", spec.bytes());
    libssl(PeSpec::dll(&[]).machine(0xaa64));
    assert_eq!(
        fx.refused(),
        "`DLLs\\libssl-3.dll` is not an x86-64 PE32+ image"
    );
    libssl(PeSpec::dll(&[]).pe32().machine(pe::MACHINE_AMD64));
    assert_eq!(
        fx.refused(),
        "`DLLs\\libssl-3.dll` is not an x86-64 PE32+ image"
    );
    libssl(PeSpec::dll(&[]).not_dll());
    assert_eq!(fx.refused(), "`DLLs\\libssl-3.dll` is not a DLL");
    fx.put("DLLs/libssl-3.dll", "not an image");
    assert_eq!(
        fx.refused(),
        "cannot scan `DLLs\\libssl-3.dll`: not a PE image"
    );
    fx.put("DLLs/libssl-3.dll", b"MZ");
    assert!(
        fx.refused()
            .starts_with("cannot scan `DLLs\\libssl-3.dll`: ")
    );
}

/// A delay import resolves only as an API set, a system file or the
/// interpreter's DLL: never as a sibling or another root DLL.
#[test]
fn a_delay_import_resolves_only_through_the_standard_search() {
    let fx = Fixture::new("native_windows_delay");
    fx.ssl(&[], &["PYTHON314.dll", CRT, "ws2_32.dll"]);
    assert_eq!(fx.scan(), Ok(()));
    fx.ssl(&[], &["libssl-3.dll"]);
    let err = fx.refused();
    assert!(
        err.contains(
            "`DLLs\\_ssl.pyd` delay-loads `libssl-3.dll`, which is neither a system DLL nor \
             `python314.dll`"
        ),
        "{err}"
    );
    fx.ssl(&[], &["vcruntime140.dll"]);
    assert!(fx.refused().contains("delay-loads `vcruntime140.dll`"));
}

/// A nested image is scanned under its `\`-joined path, and its siblings
/// are its own directory's images only.
#[test]
fn a_nested_image_is_scanned_with_its_own_siblings() {
    let fx = Fixture::new("native_windows_nested");
    fx.put("DLLs/sub/y.dll", PeSpec::dll(&[CRT]).bytes());
    fx.put(
        "DLLs/sub/x.pyd",
        PeSpec::dll(&["y.dll", "python3.dll"]).bytes(),
    );
    assert_eq!(fx.scan(), Ok(()));
    fx.put("DLLs/sub/x.pyd", PeSpec::dll(&["libssl-3.dll"]).bytes());
    assert!(
        fx.refused()
            .contains("`DLLs\\sub\\x.pyd` imports `libssl-3.dll`")
    );
}

/// Images are scanned root DLLs first, then `DLLs\` in sorted order, so
/// the first failure named is deterministic.
#[test]
fn the_first_failure_in_scan_order_is_named() {
    let fx = Fixture::new("native_windows_order");
    fx.put("DLLs/b.pyd", PeSpec::dll(&["b-missing.dll"]).bytes());
    fx.put("DLLs/a.pyd", PeSpec::dll(&["a-missing.dll"]).bytes());
    assert!(
        fx.refused()
            .contains("`DLLs\\a.pyd` imports `a-missing.dll`")
    );
    fx.put("python3.dll", PeSpec::dll(&["root-missing.dll"]).bytes());
    assert!(
        fx.refused()
            .contains("`python3.dll` imports `root-missing.dll`")
    );
}

/// The copy skips free-threaded and debug images, so the scan does too.
#[test]
fn an_image_of_another_abi_is_neither_copied_nor_scanned() {
    let fx = Fixture::new("native_windows_abi");
    fx.put(
        "DLLs/_ssl.cp314t-win_amd64.pyd",
        PeSpec::dll(&["python314t.dll"]).bytes(),
    );
    fx.put("DLLs/_ssl_d.pyd", PeSpec::dll(&["python314_d.dll"]).bytes());
    assert_eq!(fx.scan(), Ok(()));
}

/// A kept `Lib\` file that is a PE image, by name or by `MZ`, is refused;
/// an `.exe` launcher and a skipped directory's image are not.
#[test]
fn a_pe_image_under_lib_is_refused() {
    let fx = Fixture::new("native_windows_lib");
    fx.put("Lib/venv/scripts/nt/python.exe", b"MZ");
    fx.put("Lib/site-packages/x/_x.pyd", b"MZ");
    assert_eq!(fx.scan(), Ok(()));
    fx.put("Lib/json/_speedups.pyd", "text");
    assert_eq!(
        fx.refused(),
        "`Lib\\json\\_speedups.pyd` is a PE image outside `DLLs\\`; pycc cannot scan it"
    );
    std::fs::remove_file(fx.layout.prefix.join("Lib/json/_speedups.pyd")).expect("remove");
    fx.put("Lib/json/helper.bin", b"MZ\x90\x00");
    assert!(
        fx.refused()
            .starts_with("`Lib\\json\\helper.bin` is a PE image")
    );
}

/// A root DLL the probe checked but that is gone is an I/O failure.
#[test]
fn an_unreadable_root_dll_is_an_environment_failure() {
    let fx = Fixture::new("native_windows_gone");
    std::fs::remove_file(fx.layout.prefix.join("python3.dll")).expect("remove");
    assert!(fx.refused().starts_with("could not read"));
}
