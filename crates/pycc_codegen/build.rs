//! Builds `pycc_rt`'s static library as part of building `pycc_codegen`.
//!
//! `pycc build` links every compiled module against `pycc_rt`, but Cargo
//! never produces that archive on its own: it does not uplift a
//! `staticlib` reached through an ordinary `[dependencies]` edge (only
//! `target/deps/lib<name>-<hash>.a` exists, under a name nothing can
//! predict), and `cargo test` does not build the `staticlib` crate type at
//! all. Artifact dependencies would express this directly but require
//! `-Z bindeps`, which this project's pinned `rust-version` cannot use.
//! So the archive is produced here, by a nested `cargo build -p pycc_rt`,
//! and installed at the profile directory the rest of the workspace already
//! looks in.
//!
//! See `docs/decisions/D-184-build-pycc-rt-from-pycc-codegen-s-build.md`
//! for the alternatives weighed and the risks accepted.

use pycc_artifact_layout::{
    NESTED_BUILD_SENTINEL, anchor_target_root_for_build_script, cargo_target_dir_from_env,
    pycc_rt_lib_filename, resolve_cargo_target_root,
};
use std::path::{Path, PathBuf};

/// The modification time stamped on every installed archive.
///
/// The install path is also a `rerun-if-changed` input, so a freshly
/// written file would look newer than the build script's own output and
/// ask Cargo to run this script again on the very next build. A constant,
/// far-past timestamp makes the installed file's mtime a function of
/// nothing, which ends that loop. 2020-01-01T00:00:00Z.
const INSTALLED_ARCHIVE_MTIME_SECS: u64 = 1_577_836_800;

/// Both host profiles are built unconditionally.
///
/// Which one a later `pycc build` links against is chosen at *its* run
/// time (`pycc build --release`), long after this script has finished, and
/// the build script has no way to learn that choice: `PROFILE` describes
/// how `pycc_codegen` itself is being compiled, not how the compiler it
/// becomes will later be asked to link. Building only the matching profile
/// would leave `pycc build --release` failing after a plain `cargo build`,
/// which is exactly the clean-checkout breakage this exists to fix.
const PROFILES: [&str; 2] = ["debug", "release"];

fn main() {
    // Cargo hashes a build script's stdout into its dependents'
    // fingerprints, so every directive below must be emitted on every run,
    // in the same order, before any early return.
    let manifest_dir = PathBuf::from(env_var("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| {
            panic!(
                "cannot locate the workspace root above {}",
                manifest_dir.display()
            )
        })
        .to_path_buf();
    let rt_dir = workspace_root.join("crates").join("pycc_rt");

    println!("cargo::rerun-if-changed={}", rt_dir.join("src").display());
    println!(
        "cargo::rerun-if-changed={}",
        rt_dir.join("Cargo.toml").display()
    );
    println!(
        "cargo::rerun-if-changed={}",
        workspace_root.join("Cargo.toml").display()
    );
    println!("cargo::rerun-if-env-changed=CARGO_TARGET_DIR");
    println!("cargo::rerun-if-env-changed=CARGO_BUILD_TARGET_DIR");
    println!("cargo::rerun-if-env-changed={NESTED_BUILD_SENTINEL}");

    let out_dir = PathBuf::from(env_var("OUT_DIR"));
    let host = env_var("HOST");
    let target = env_var("TARGET");
    let cross = target != host;
    let archive_name = pycc_rt_lib_filename(Some(&target));

    let anchored = anchor_target_root_for_build_script(
        &resolve_cargo_target_root(&workspace_root, cargo_target_dir_from_env),
        &workspace_root,
        &out_dir,
    );
    if anchored.diverged {
        println!(
            "cargo::warning=pycc_rt archives are being installed under {} because a relative \
             Cargo target directory was anchored on the workspace root, but this build's OUT_DIR \
             ({}) is not inside it. Cargo resolved that relative value against a different \
             working directory; invoke cargo from the workspace root, or set an absolute \
             CARGO_TARGET_DIR, or `cargo build -p pycc_rt` yourself.",
            anchored.root.display(),
            out_dir.display()
        );
    }

    // Every path is computed, and every directive emitted, before the
    // recursion guard below can return: the stdout contract is
    // unconditional, so no directive may sit behind an early return or
    // inside the work loop.
    let triple = cross.then_some(&target);
    let installs: Vec<(&str, PathBuf, PathBuf, PathBuf)> = PROFILES
        .iter()
        .map(|profile| {
            let nested_root = out_dir.join(format!("pycc_rt-nested-{profile}"));
            let built = profile_dir(&nested_root, profile, triple).join(archive_name);
            let installed = profile_dir(&anchored.root, profile, triple).join(archive_name);
            (*profile, nested_root, built, installed)
        })
        .collect();
    for (_, _, _, installed) in &installs {
        println!("cargo::rerun-if-changed={}", installed.display());
    }

    // Re-entered from inside the nested build: doing anything further here
    // would recurse.
    if std::env::var_os(NESTED_BUILD_SENTINEL).is_some() {
        return;
    }

    for (profile, nested_root, built, installed) in &installs {
        run_nested_cargo(&workspace_root, nested_root, profile, triple);
        install(built, installed);
    }
}

/// A required build-script environment variable, or a panic naming it.
fn env_var(key: &str) -> String {
    std::env::var(key)
        .unwrap_or_else(|error| panic!("build script requires {key} to be set: {error}"))
}

/// `<root>/<triple>/<profile>` when cross-compiling, `<root>/<profile>`
/// otherwise -- the same shape Cargo itself uses, and the shape
/// `pycc_artifact_layout::find_pycc_rt_lib_dir_in` later looks in.
fn profile_dir(root: &Path, profile: &str, triple: Option<&String>) -> PathBuf {
    match triple {
        Some(triple) => root.join(triple).join(profile),
        None => root.join(profile),
    }
}

/// Runs `cargo build -p pycc_rt` into a private nested build directory.
///
/// The nested directory lives under `OUT_DIR`, never under the directory
/// the outer build is using: a build script that invokes cargo at the same
/// build directory blocks on Cargo's own build-directory lock, which the
/// outer invocation is already holding, and the build hangs indefinitely
/// rather than failing. `pycc_rt` declares no dependencies at all, so the
/// nested invocation needs no registry access and never contends for
/// `$CARGO_HOME/.package-cache` either.
///
/// The environment is scrubbed of everything describing the outer build
/// (see `pycc_artifact_layout::should_scrub_for_nested_cargo`) by removal
/// rather than by an allowlist, so platform toolchain variables survive on
/// every host.
fn run_nested_cargo(
    workspace_root: &Path,
    nested_root: &Path,
    profile: &str,
    triple: Option<&String>,
) {
    let cargo = env_var("CARGO");
    let mut command = std::process::Command::new(&cargo);
    command
        .current_dir(workspace_root)
        .arg("build")
        .arg("--locked")
        .arg("-p")
        .arg("pycc_rt")
        .arg("--target-dir")
        .arg(nested_root);
    if profile == "release" {
        command.arg("--release");
    }
    if let Some(triple) = triple {
        command.arg("--target").arg(triple);
    }
    for (name, _) in std::env::vars_os() {
        let name = name.to_string_lossy().into_owned();
        if pycc_artifact_layout::should_scrub_for_nested_cargo(&name) {
            command.env_remove(&name);
        }
    }
    command.env(NESTED_BUILD_SENTINEL, "1");

    let described = format!(
        "{} {:?} (cwd {})",
        cargo,
        command.get_args().collect::<Vec<_>>(),
        workspace_root.display()
    );
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("failed to spawn nested build: {described}: {error}"));
    assert!(
        output.status.success(),
        "nested build failed ({}): {described}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Copies `built` to `installed`, idempotently and atomically.
///
/// Writing in place would leave a torn archive visible to a concurrent
/// reader, so the bytes go to a uniquely named temporary *inside the
/// destination directory* -- same filesystem, so the rename is atomic --
/// and are renamed over the destination. When the destination already
/// holds the same bytes nothing is written at all, which keeps repeated
/// builds from touching a file the outer build may have hardlinked.
fn install(built: &Path, installed: &Path) {
    let bytes = std::fs::read(built).unwrap_or_else(|error| {
        panic!("nested build did not produce {}: {error}", built.display())
    });
    let dir = installed.parent().unwrap_or_else(|| {
        panic!(
            "install path has no parent directory: {}",
            installed.display()
        )
    });
    std::fs::create_dir_all(dir)
        .unwrap_or_else(|error| panic!("cannot create {}: {error}", dir.display()));
    if std::fs::read(installed).is_ok_and(|existing| existing == bytes) {
        return;
    }
    let unique = format!(
        ".pycc_rt-install-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or_default()
    );
    let temporary = dir.join(unique);
    std::fs::write(&temporary, &bytes)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", temporary.display()));
    std::fs::rename(&temporary, installed).unwrap_or_else(|error| {
        panic!(
            "cannot install {} as {}: {error}",
            temporary.display(),
            installed.display()
        )
    });
    // Backdate only the file just renamed into place -- never a
    // pre-existing destination, which may be a hardlink into Cargo's own
    // artifact directory.
    let times = std::fs::FileTimes::new().set_modified(
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(INSTALLED_ARCHIVE_MTIME_SECS),
    );
    let handle = std::fs::File::options()
        .write(true)
        .open(installed)
        .unwrap_or_else(|error| panic!("cannot reopen {}: {error}", installed.display()));
    handle
        .set_times(times)
        .unwrap_or_else(|error| panic!("cannot backdate {}: {error}", installed.display()));
}
