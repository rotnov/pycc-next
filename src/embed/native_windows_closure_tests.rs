//! The Windows closure-image classification on the Windows-shaped fake
//! layout, with its system directory injected: every rule for a closure
//! image and a native, every screen row, every collision, and the pure
//! listing map and classifier with synthetic names. Runs on every host.

use super::super::fake_layout::{FakeLayout, fake_windows_env, fake_windows_layout};
use super::super::pe::fixture::PeSpec;
use super::*;
use crate::lock::build::ClosureFile;
use pycc_scratch::ScratchDir;

const CRT: &str = "api-ms-win-crt-runtime-l1-1-0.dll";

struct Fixture {
    _dir: ScratchDir,
    layout: FakeLayout,
    env: WindowsEnv,
    site: PathBuf,
    files: Vec<ClosureFile>,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir = ScratchDir::new(tag).expect("scratch");
        let layout = fake_windows_layout(&dir);
        let env = fake_windows_env(&layout);
        let site = layout.prefix.parent().expect("root").join("site");
        Self {
            _dir: dir,
            layout,
            env,
            site,
            files: Vec::new(),
        }
    }

    /// Writes `bytes` at `rel` under the site, unlocked; returns the path.
    fn put(&self, rel: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.site.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, bytes).expect("write");
        path
    }

    /// Writes `bytes` at `rel` and locks it for distribution `package`.
    fn lock(&mut self, package: &str, rel: &str, bytes: impl AsRef<[u8]>) {
        let source = self.put(rel, bytes);
        self.files.push(ClosureFile {
            rel: rel.to_string(),
            source,
            digest: String::new(),
            package: package.to_string(),
        });
    }

    fn derive(&self) -> Result<Vec<DerivedNative>, String> {
        let closure = LockedClosure::of_files(self.files.clone());
        derive(&self.layout.probe, &closure, &self.env)
    }

    fn natives(&self) -> Vec<(String, Vec<String>)> {
        let natives = self.derive().expect("derived");
        natives
            .into_iter()
            .map(|n| (n.locked.name, n.locked.required_by))
            .collect()
    }

    fn refused(&self) -> String {
        self.derive().expect_err("refused")
    }
}

fn dll(imports: &[&str]) -> Vec<u8> {
    PeSpec::dll(imports).bytes()
}

fn owned(names: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
    names
        .iter()
        .map(|(name, owners)| {
            let owners = owners.iter().map(|o| o.to_string()).collect();
            (name.to_string(), owners)
        })
        .collect()
}

#[test]
fn a_closure_without_a_pe_image_derives_nothing() {
    let mut fx = Fixture::new("nwc_no_image");
    fx.lock("pkg", "pkg/__init__.py", "X = 1\n");
    fx.lock("pkg", "pkg/tool.exe", b"MZ launcher");
    fx.lock("pkg", "pkg/data.bin", b"not an image");
    assert_eq!(fx.derive(), Ok(Vec::new()));
}

#[test]
fn a_closure_file_that_cannot_be_read_is_refused_by_path() {
    let mut fx = Fixture::new("nwc_missing");
    fx.lock("pkg", "pkg/gone.py", "");
    let missing = fx.files[0].source.clone();
    std::fs::remove_file(&missing).expect("remove");
    let err = fx.refused();
    assert!(err.contains(&missing.display().to_string()), "{err}");
}

#[test]
fn imports_of_api_sets_root_dlls_and_system_dlls_resolve() {
    let mut fx = Fixture::new("nwc_r1_r2_r6");
    let imports = [CRT, "python314.dll", "VCRUNTIME140.dll", "kernel32.DLL"];
    fx.lock("pkg", "pkg/_x.pyd", dll(&imports));
    assert_eq!(fx.natives(), owned(&[]));
}

#[test]
fn every_screen_row_refuses_a_closure_image() {
    let rows: [(&str, Vec<u8>, &str); 6] = [
        ("_a.pyd", b"MZ".to_vec(), "cannot scan"),
        ("_b.pyd", b"plain text".to_vec(), "is not a PE image"),
        (
            "_c.pyd",
            PeSpec::dll(&[]).pe32().bytes(),
            "is not an x86-64 PE32+ image",
        ),
        (
            "_d.pyd",
            PeSpec::dll(&[]).machine(0x14c).bytes(),
            "is not an x86-64 PE32+ image",
        ),
        ("_e.pyd", PeSpec::dll(&[]).not_dll().bytes(), "is not a DLL"),
        (
            "_f.pyd",
            PeSpec::dll(&[]).forwards(&["nodot"]).bytes(),
            "names no module",
        ),
    ];
    for (name, bytes, why) in rows {
        let mut fx = Fixture::new("nwc_screen");
        fx.lock("pkg", &format!("pkg/sub/{name}"), bytes);
        let err = fx.refused();
        let shown = format!("`closure\\pkg\\sub\\{name}` (distribution `pkg`)");
        assert!(err.contains(&shown), "{err}");
        assert!(err.contains(why), "{err}");
        assert!(err.ends_with(" -- use `pycc build --ext`"), "{err}");
    }
}

#[test]
fn an_mz_file_under_any_other_suffix_is_screened() {
    let mut fx = Fixture::new("nwc_mz_suffix");
    fx.lock("pkg", "pkg/blob.bin", b"MZ");
    let err = fx.refused();
    assert!(
        err.contains("cannot scan `closure\\pkg\\blob.bin`"),
        "{err}"
    );
}

#[test]
fn the_program_dll_name_is_refused() {
    let mut fx = Fixture::new("nwc_r2b");
    fx.lock("pkg", "pkg/_x.pyd", dll(&["PYCC_PROGRAM.dll"]));
    let err = fx.refused();
    assert!(
        err.contains("imports `PYCC_PROGRAM.dll`, which is the name the program DLL"),
        "{err}"
    );
}

#[test]
fn a_locked_image_beside_the_importer_is_payload() {
    let mut fx = Fixture::new("nwc_r3");
    fx.lock("pkg", "pkg/_x.pyd", dll(&["Payload.dll"]));
    fx.lock("pkg", "pkg/payload.dll", dll(&["KERNEL32.dll"]));
    assert_eq!(fx.natives(), owned(&[]));
}

#[test]
fn an_unlocked_file_beside_the_importer_is_a_native_by_its_on_disk_name() {
    let mut fx = Fixture::new("nwc_r4");
    fx.lock("pkg", "pkg/_x.pyd", dll(&["HELPER.DLL"]));
    let helper = fx.put("pkg/helper.dll", dll(&["KERNEL32.dll", CRT]));
    let natives = fx.derive().expect("derived");
    assert_eq!(natives.len(), 1);
    assert_eq!(natives[0].source, helper);
    assert_eq!(natives[0].locked.name, "helper.dll");
    assert_eq!(natives[0].locked.required_by, ["pkg"]);
    let digest = crate::embed::sha256::sha256_file(&helper).expect("digest");
    assert_eq!(natives[0].locked.sha256, digest);
}

#[test]
fn natives_chain_and_are_shared_by_their_owners() {
    let mut fx = Fixture::new("nwc_chain");
    fx.lock("pkg", "pkg/_x.pyd", dll(&["a.dll"]));
    fx.lock("dep", "pkg/_y.pyd", dll(&["a.dll"]));
    fx.put("pkg/a.dll", dll(&["b.dll"]));
    fx.put("pkg/b.dll", dll(&["python314.dll"]));
    assert_eq!(
        fx.natives(),
        owned(&[("a.dll", &["dep", "pkg"]), ("b.dll", &["dep", "pkg"])])
    );
}

#[test]
fn a_native_import_cycle_is_scanned_once_per_owner() {
    let mut fx = Fixture::new("nwc_cycle");
    fx.lock("pkg", "pkg/_x.pyd", dll(&["a.dll", "c.dll"]));
    fx.put("pkg/a.dll", dll(&["b.dll", "c.dll"]));
    fx.put("pkg/b.dll", dll(&["a.dll"]));
    fx.put("pkg/c.dll", dll(&[]));
    assert_eq!(
        fx.natives(),
        owned(&[
            ("a.dll", &["pkg"]),
            ("b.dll", &["pkg"]),
            ("c.dll", &["pkg"])
        ])
    );
}

#[test]
fn the_one_locked_image_of_a_name_anywhere_is_payload() {
    let mut fx = Fixture::new("nwc_r5");
    fx.lock("pkg", "pkg/core/_x.pyd", dll(&["shared.dll"]));
    fx.lock("pkg", "pkg.libs/shared.dll", dll(&[]));
    assert_eq!(fx.natives(), owned(&[]));
}

#[test]
fn two_locked_images_of_a_name_are_ambiguous() {
    let mut fx = Fixture::new("nwc_r5_ambiguous");
    fx.lock("pkg", "pkg/core/_x.pyd", dll(&["shared.dll"]));
    fx.lock("pkg", "pkg/a/shared.dll", dll(&[]));
    fx.lock("dep", "dep/b/Shared.dll", dll(&[]));
    let err = fx.refused();
    assert!(
        err.contains(
            "matches more than one locked image of the closure (`closure\\pkg\\a\\shared.dll` \
             and `closure\\dep\\b\\Shared.dll`)"
        ),
        "{err}"
    );
}

#[test]
fn an_unplaced_closure_import_is_refused_naming_image_distribution_and_import() {
    let mut fx = Fixture::new("nwc_closure_refused");
    fx.lock("tinyext", "tinyext/_bad.pyd", dll(&["pycc1306missing.dll"]));
    assert_eq!(
        fx.refused(),
        "`closure\\tinyext\\_bad.pyd` (distribution `tinyext`) imports \
         `pycc1306missing.dll`, which is neither an API set, a DLL the sidecar bundles, a \
         locked file of the closure, a file beside it, nor a system DLL -- use `pycc build \
         --ext`"
    );
}

#[test]
fn a_name_only_in_the_interpreter_dlls_is_refused() {
    let mut fx = Fixture::new("nwc_dlls_only");
    fx.lock("pkg", "pkg/_x.pyd", dll(&["libssl-3.dll"]));
    let err = fx.refused();
    assert!(
        err.contains("imports `libssl-3.dll`, which is neither"),
        "{err}"
    );
}

#[test]
fn an_unplaced_native_import_is_refused_naming_the_native() {
    let mut fx = Fixture::new("nwc_native_refused");
    fx.lock("tinyext", "tinyext/_bad.pyd", dll(&["pycc1306helper.dll"]));
    fx.put("tinyext/pycc1306helper.dll", dll(&["pycc1306missing.dll"]));
    assert_eq!(
        fx.refused(),
        "the native `natives\\pycc1306helper.dll` (copied for distribution `tinyext`, needed \
         by `closure\\tinyext\\_bad.pyd`) imports `pycc1306missing.dll`, which is neither an \
         API set, a DLL the sidecar bundles, a file beside it, nor a system DLL -- use `pycc \
         build --ext`"
    );
}

#[test]
fn a_native_does_not_reach_closure_images_or_the_program_dll() {
    let mut fx = Fixture::new("nwc_native_rules");
    fx.lock("pkg", "pkg/core/_x.pyd", dll(&["helper.dll"]));
    fx.put("pkg/core/helper.dll", dll(&["shared.dll"]));
    fx.lock("pkg", "pkg.libs/shared.dll", dll(&[]));
    let err = fx.refused();
    assert!(err.contains("the native `natives\\helper.dll`"), "{err}");
    assert!(
        err.contains("imports `shared.dll`, which is neither"),
        "{err}"
    );

    let mut fx = Fixture::new("nwc_native_r3");
    fx.lock("pkg", "pkg/_x.pyd", dll(&["helper.dll", "payload.dll"]));
    fx.lock("pkg", "pkg/payload.dll", dll(&[]));
    fx.put("pkg/helper.dll", dll(&["payload.dll"]));
    let err = fx.refused();
    assert!(
        err.contains("imports `payload.dll`, which is neither"),
        "{err}"
    );

    let mut fx = Fixture::new("nwc_native_r2b");
    fx.lock("pkg", "pkg/_x.pyd", dll(&["helper.dll"]));
    fx.put("pkg/helper.dll", dll(&["pycc_program.dll"]));
    let err = fx.refused();
    assert!(err.contains("which is the name the program DLL"), "{err}");
}

#[test]
fn every_screen_row_refuses_a_native() {
    let rows: [(Vec<u8>, &str); 3] = [
        (b"not an image".to_vec(), "is not a PE image"),
        (PeSpec::dll(&[]).not_dll().bytes(), "is not a DLL"),
        (
            PeSpec::dll(&[]).pe32().bytes(),
            "is not an x86-64 PE32+ image",
        ),
    ];
    for (bytes, why) in rows {
        let mut fx = Fixture::new("nwc_native_screen");
        fx.lock("pkg", "pkg/_x.pyd", dll(&["helper.dll"]));
        fx.put("pkg/helper.dll", bytes);
        let err = fx.refused();
        assert!(err.starts_with("the native `natives\\helper.dll`"), "{err}");
        assert!(err.contains(why), "{err}");
    }
}

#[test]
fn delay_imports_and_forwarders_resolve_only_to_the_system_or_the_interpreter() {
    let mut fx = Fixture::new("nwc_restricted_ok");
    let image = PeSpec::dll(&[])
        .delay(&["WS2_32.dll", "python314.dll", CRT])
        .forwards(&["KERNEL32.HeapAlloc", "PYTHON314.Py_Main"]);
    fx.lock("pkg", "pkg/_x.pyd", image.bytes());
    assert_eq!(fx.natives(), owned(&[]));

    let mut fx = Fixture::new("nwc_delay_refused");
    fx.lock(
        "pkg",
        "pkg/_x.pyd",
        PeSpec::dll(&[]).delay(&["vcruntime140.dll"]).bytes(),
    );
    let err = fx.refused();
    assert_eq!(
        err,
        "`closure\\pkg\\_x.pyd` (distribution `pkg`) delay-loads `vcruntime140.dll`, which is \
         neither an API set, a system DLL, nor `python314.dll` -- use `pycc build --ext`"
    );

    let mut fx = Fixture::new("nwc_forward_refused");
    fx.lock("pkg", "pkg/_x.pyd", dll(&["helper.dll"]));
    fx.put(
        "pkg/helper.dll",
        PeSpec::dll(&[]).forwards(&["other.Fn"]).bytes(),
    );
    let err = fx.refused();
    assert!(
        err.contains("`natives\\helper.dll` (copied for distribution `pkg`"),
        "{err}"
    );
    assert!(
        err.contains("forwards to `other.dll`, which is neither"),
        "{err}"
    );
}

#[test]
fn a_native_named_like_a_system_dll_is_refused_as_shadowing() {
    let mut fx = Fixture::new("nwc_shadow");
    fx.lock("pkg", "pkg/_x.pyd", dll(&["ws2_32.dll"]));
    fx.put("pkg/WS2_32.dll", dll(&[]));
    let err = fx.refused();
    assert!(err.starts_with("the native `natives\\WS2_32.dll`"), "{err}");
    assert!(err.contains("has the name of a system DLL"), "{err}");
}

#[test]
fn a_native_named_like_a_closure_image_is_refused() {
    let mut fx = Fixture::new("nwc_payload_collision");
    fx.lock("pkg", "pkg/a/_x.pyd", dll(&["helper.dll"]));
    fx.put("pkg/a/helper.dll", dll(&[]));
    fx.lock("pkg", "pkg/b/helper.dll", dll(&[]));
    let err = fx.refused();
    assert!(
        err.contains("has the name of the closure image `closure\\pkg\\b\\helper.dll`"),
        "{err}"
    );
}

#[test]
fn a_native_named_like_a_file_in_the_interpreter_dlls_is_refused() {
    let mut fx = Fixture::new("nwc_dlls_collision");
    fx.lock("pkg", "pkg/_x.pyd", dll(&["TCL86T.dll"]));
    fx.put("pkg/tcl86t.dll", dll(&[]));
    let err = fx.refused();
    assert!(
        err.contains("has the name of the interpreter's `DLLs\\tcl86t.dll`"),
        "{err}"
    );
}

#[test]
fn an_unlistable_interpreter_dlls_is_refused_naming_it() {
    let mut fx = Fixture::new("nwc_dlls_file");
    let dlls = fx.layout.prefix.join("DLLs");
    std::fs::remove_dir_all(&dlls).expect("remove DLLs");
    std::fs::write(&dlls, "not a directory").expect("write");
    fx.lock("pkg", "pkg/_x.pyd", dll(&["helper.dll"]));
    fx.put("pkg/helper.dll", dll(&[]));
    let err = fx.refused();
    assert!(err.contains("could not list"), "{err}");
    assert!(err.contains(&dlls.display().to_string()), "{err}");
}

#[test]
fn two_sources_for_one_native_name_are_refused_in_natives() {
    let mut fx = Fixture::new("nwc_two_sources");
    fx.lock("pkg", "pkg/a/_x.pyd", dll(&["helper.dll"]));
    fx.lock("pkg", "pkg/b/_y.pyd", dll(&["helper.dll"]));
    fx.put("pkg/a/helper.dll", dll(&[]));
    fx.put("pkg/b/helper.dll", dll(&[]));
    let err = fx.refused();
    assert!(
        err.contains("both need the name `helper.dll` in the bundle's `natives\\`"),
        "{err}"
    );
}

#[test]
fn a_subdirectory_beside_an_image_is_not_a_native() {
    let mut fx = Fixture::new("nwc_subdir");
    fx.lock("pkg", "pkg/_x.pyd", dll(&["helper.dll"]));
    fx.put("pkg/helper.dll/inner.txt", "a directory, not a file");
    let err = fx.refused();
    assert!(
        err.contains("imports `helper.dll`, which is neither"),
        "{err}"
    );
}

#[test]
fn a_listing_map_records_case_folded_duplicates_and_refuses_only_them() {
    let dir = Path::new("site/pkg");
    let names = ["Foo.dll", "bar.dll", "foo.DLL", "FOO.dll"];
    let map = listing_map(dir, names.iter().map(OsString::from).collect());
    assert_eq!(map.lookup("BAR.DLL"), Ok(Some("bar.dll")));
    assert_eq!(map.lookup("other.dll"), Ok(None));
    let err = map.lookup("foo.dll").expect_err("duplicate");
    assert_eq!(
        err,
        format!(
            "matches both `Foo.dll` and `foo.DLL` in `{}`, which Windows cannot tell apart",
            dir.display()
        )
    );
}

/// A context over synthetic listings: nothing touches the disk.
fn synthetic(dir: &Path, listed: &[&str]) -> Ctx {
    let mut ctx = Ctx {
        ldlibrary: "python314.dll".to_string(),
        ..Ctx::default()
    };
    let names = listed.iter().map(OsString::from).collect();
    ctx.listings
        .insert(dir.to_path_buf(), listing_map(dir, names));
    ctx
}

#[test]
fn the_classifier_refuses_a_duplicate_only_when_it_is_looked_up() {
    let dir = PathBuf::from("site").join("pkg");
    let ctx = synthetic(&dir, &["Helper.dll", "helper.DLL", "other.dll"]);
    let image = ImageCtx {
        source_dir: dir.clone(),
        closure: true,
    };
    let err = classify("HELPER.dll", &image, &ctx).expect_err("duplicate");
    assert!(
        err.contains("matches both `Helper.dll` and `helper.DLL`"),
        "{err}"
    );
    let native = Resolution::Native(dir.join("other.dll"), "other.dll".to_string());
    assert_eq!(classify("OTHER.DLL", &image, &ctx), Ok(native));
}

#[test]
fn the_payload_keys_are_case_folded_full_paths() {
    let dir = PathBuf::from("Site").join("Pkg");
    let mut ctx = synthetic(&dir, &["DATA.dll"]);
    let recorded = PathBuf::from("site/pkg/data.dll");
    ctx.payload.insert(fold_path(&recorded));
    ctx.images.insert(fold_path(&recorded));
    let closure = ImageCtx {
        source_dir: dir.clone(),
        closure: true,
    };
    assert_eq!(
        classify("data.dll", &closure, &ctx),
        Ok(Resolution::Payload)
    );
    let native = ImageCtx {
        source_dir: dir.clone(),
        closure: false,
    };
    assert_eq!(
        classify("data.dll", &native, &ctx),
        Err(NATIVE_WHY.to_string())
    );
    let unlisted = ImageCtx {
        source_dir: PathBuf::from("elsewhere"),
        closure: true,
    };
    assert_eq!(
        classify("data.dll", &unlisted, &ctx),
        Err(CLOSURE_WHY.to_string())
    );
}

#[test]
fn a_duplicate_in_the_interpreter_dlls_is_refused_when_a_native_claims_it() {
    let mut ctx = Ctx::default();
    let dir = Path::new("base/DLLs");
    let names = ["Clash.dll", "clash.dll"]
        .iter()
        .map(OsString::from)
        .collect();
    ctx.dlls = Some(listing_map(dir, names));
    let err = ctx
        .check_native_name("the native `natives\\CLASH.dll`", "CLASH.dll")
        .expect_err("duplicate");
    assert!(
        err.starts_with("the native `natives\\CLASH.dll` matches both `Clash.dll`"),
        "{err}"
    );
    assert!(err.ends_with(" -- use `pycc build --ext`"), "{err}");
    assert_eq!(ctx.check_native_name("s", "fine.dll"), Ok(()));
}

#[test]
fn a_folded_path_ignores_case_and_the_separator_spelling() {
    let joined = Path::new("A").join("b").join("C.dll");
    assert_eq!(fold_path(Path::new("a/B/c.DLL")), fold_path(&joined));
    assert_eq!(folded_name(Path::new("x/Y.DLL")), "y.dll");
    assert_eq!(source_dir(Path::new("")), PathBuf::new());
}
