//! The whole-package build's driver and link (v0.8, `crate::package_build`):
//! one member build per module, in parallel, then the runtime unit and the
//! package unit, then one link into `PKG/__init__.abi3.so`.

use super::ext_plan::write_ext_source;
use super::package_schedule;
use super::{
    add_linux_system_libs, add_windows_crt_math, add_windows_system_libs, c_objects,
    effective_link_target, find_pycc_rt_lib_dir, linker_command, linker_spawn_error,
};
use crate::package_build::{self, MEMBER_MANIFEST_NAME, PackageModule};
use crate::{ext_build, ext_output};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// How a package build compiles its members: the flags of the `pycc build`
/// it was given that every member build takes too.
pub(crate) struct PackageBuild<'a> {
    pub(crate) path: &'a Path,
    pub(crate) out: &'a Path,
    pub(crate) target: Option<&'a str>,
    pub(crate) release: bool,
    pub(crate) excludes: &'a [String],
    /// `--non-strict`, `--strict`, `--report-fallbacks` as given.
    pub(crate) mode_args: Vec<OsString>,
}

/// Writes a member's interface (`crate::package_build::Interface`): its C
/// header beside the member's module unit, then the interface file into
/// `out`, renamed into place so a reader never sees half of it.
pub(super) fn write_interface(
    out: &Path,
    obj_path: &Path,
    member: &package_build::Member,
    mir: &pycc_mir::MirModule,
    published: &std::collections::BTreeSet<String>,
) -> Result<(), ExitCode> {
    let interface =
        package_build::Interface::collect(&member.dotted, &member.prefix, mir, published);
    write_ext_source(
        &obj_path.with_file_name(package_build::INTERFACE_HEADER_NAME),
        &interface.c_header(),
    )?;
    let path = out.join(package_build::INTERFACE_NAME);
    let partial = out.join(format!("{}.partial", package_build::INTERFACE_NAME));
    write_ext_source(&partial, &interface.to_text())?;
    std::fs::rename(&partial, &path).map_err(|e| {
        eprintln!("error: could not write `{}`: {e}", path.display());
        ExitCode::from(2)
    })
}

/// Records a member build's objects and the highest exception tag its
/// classes took, in `out` (its output directory), for the package build.
pub(super) fn write_member_manifest(
    out: &Path,
    objects: &[&PathBuf],
    typed_hir: &pycc_hir::HirModule,
) -> Result<(), ExitCode> {
    let max_tag = typed_hir
        .class_defs
        .iter()
        .filter_map(|(_, def)| def.exception_type_tag)
        .filter(|tag| *tag != pycc_hir::FOREIGN_BASE_EXCEPTION_TYPE_TAG)
        .max()
        .unwrap_or(0);
    let mut manifest = String::new();
    for object in objects {
        manifest.push_str(&format!("{}\n", object.display()));
    }
    manifest.push_str(&format!("max_tag {max_tag}\n"));
    // The interfaces this member compiled against, checked before the link.
    for producer in package_build::member().map_or(&[][..], |member| &member.producers) {
        manifest.push_str(&format!(
            "consumed {} {}\n",
            producer.interface.module, producer.hash
        ));
    }
    write_ext_source(&out.join(MEMBER_MANIFEST_NAME), &manifest)
}

/// What a finished member build left: its objects, its highest tag and
/// the interfaces it compiled against (module, content hash).
type MemberManifest = (Vec<PathBuf>, u16, Vec<(String, u64)>);

fn read_member_manifest(dir: &Path) -> Result<MemberManifest, String> {
    let text = std::fs::read_to_string(dir.join(MEMBER_MANIFEST_NAME)).map_err(|e| {
        format!(
            "a member build left no manifest in `{}`: {e}",
            dir.display()
        )
    })?;
    let mut objects = Vec::new();
    let mut max_tag = 0;
    let mut consumed = Vec::new();
    for line in text.lines() {
        if let Some(tag) = line.strip_prefix("max_tag ") {
            max_tag = tag.parse().unwrap_or(u16::MAX);
        } else if let Some(rest) = line.strip_prefix("consumed ") {
            let (module, hash) = rest.split_once(' ').unwrap_or((rest, ""));
            consumed.push((module.to_string(), hash.parse().unwrap_or(0)));
        } else {
            objects.push(PathBuf::from(line));
        }
    }
    Ok((objects, max_tag, consumed))
}

/// `pycc build PKG --ext`: every module of the package directory into one
/// extension (`crate::package_build`).
pub(crate) fn build_package(
    build: &PackageBuild<'_>,
    toolchain: &ext_build::ExtToolchain,
    scratch: &Path,
) -> Result<(), ExitCode> {
    let fail = |message: String| {
        eprintln!("error: {message}");
        ExitCode::from(2)
    };
    let modules = package_build::discover(build.path, build.excludes).map_err(fail)?;
    let package = &modules[0].dotted;
    let platform = ext_build::ExtLinkPlatform::resolve(build.target);
    let output = ext_output::resolve(build.out, &platform.suffix_platform())
        .map_err(|e| fail(e.message()))?;
    if &output.module_name != package {
        return Err(fail(format!(
            "a package build's artifact is the package's own `__init__` extension: `-o` must \
             name `{}/__init__`, not `{}`",
            build.path.display(),
            build.out.display()
        )));
    }
    let sources: Vec<String> = modules
        .iter()
        .map(|module| std::fs::read_to_string(&module.path).unwrap_or_default())
        .collect();
    let (bases, end) = package_build::exception_tag_bases(&sources).map_err(fail)?;
    let probe = toolchain.probe().map_err(fail)?;

    // Every member, built as a module of its own but to objects.
    let exe = std::env::current_exe().map_err(|e| fail(format!("cannot find pycc itself: {e}")))?;
    let dirs: Vec<PathBuf> = (0..modules.len())
        .map(|i| scratch.join(format!("m{i}")))
        .collect();
    for dir in &dirs {
        std::fs::create_dir_all(dir)
            .map_err(|e| fail(format!("cannot create `{}`: {e}", dir.display())))?;
    }
    // Which members' interfaces each member reads (cross-module native
    // calls): it starts once they are written.
    // `PYCC_NO_NATIVE_SIBLINGS=1` builds without them, to measure them.
    let producers = match std::env::var_os("PYCC_NO_NATIVE_SIBLINGS") {
        Some(_) => vec![Vec::new(); modules.len()],
        None => package_build::producers(&modules, &sources),
    };
    let command = |member: usize, available: &[usize]| {
        let mut cmd = member_command(&exe, build, &modules[member], bases[member], &dirs[member]);
        for &producer in available {
            let mut entry = OsString::from(format!("{}=", modules[producer].dotted));
            entry.push(dirs[producer].join(package_build::INTERFACE_NAME));
            cmd.arg("--sibling-interface").arg(entry);
        }
        cmd
    };
    let jobs = std::env::var("PYCC_JOBS")
        .ok()
        .and_then(|jobs| jobs.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, usize::from));
    let started = std::time::Instant::now();
    let runs = crate::timing::span("package: member builds", || {
        package_schedule::run_members(&dirs, &producers, jobs, &command)
    });
    let mut objects = Vec::new();
    let mut failure = None;
    for (index, (module, run)) in modules.iter().zip(runs).enumerate() {
        let run = run.map_err(fail)?;
        use std::io::Write;
        let _ = std::io::stderr().write_all(&run.output);
        if !run.ok {
            eprintln!("pycc: {}: failed", module.dotted);
            failure.get_or_insert(run.code);
            continue;
        }
        eprintln!("pycc: {}: built in {:.2}s", module.dotted, run.seconds);
        let (member_objects, max_tag, consumed) =
            read_member_manifest(&dirs[index]).map_err(fail)?;
        check_consumed(&modules, &dirs, &module.dotted, &consumed).map_err(fail)?;
        let next = bases.get(index + 1).map_or(end, |base| u16::from(*base));
        if max_tag != 0 && max_tag >= next {
            return Err(fail(format!(
                "internal: {}'s exception classes took tags up to {max_tag}, past its range \
                 (next member at {next})",
                module.dotted
            )));
        }
        objects.extend(member_objects);
    }
    if let Some(code) = failure {
        return Err(ExitCode::from(code));
    }
    eprintln!(
        "pycc: {} modules of {package} built in {:.2}s",
        modules.len(),
        started.elapsed().as_secs_f64()
    );

    // The runtime unit (from the build cache, after the first build) and
    // the package unit.
    let runtime = scratch.join(ext_build::SHIM_RUNTIME_C_NAME);
    write_ext_source(&runtime, ext_build::runtime_unit_text())?;
    let finder = scratch.join("pycc_pkg_finder.c");
    write_ext_source(&finder, package_build::FINDER_C)?;
    write_ext_source(
        &scratch.join(package_build::MEMBERS_HEADER_NAME),
        &package_build::members_header(package, &modules),
    )?;
    let mut compile_args = ext_build::ext_compile_args(platform, &probe.include, &runtime);
    compile_args.push(finder.into_os_string());
    let c = c_objects::CObjects::spawn(&compile_args, scratch, || {
        let mut cmd = linker_command(build.target);
        if let Some(triple) = effective_link_target(build.target) {
            cmd.arg("-target").arg(triple);
        }
        cmd
    })?
    .wait()?;
    objects.extend(c);

    // One link, one copy of the runtime.
    let rt_lib_dir = find_pycc_rt_lib_dir(build.target, build.release).map_err(fail)?;
    let mut link_args = ext_build::ext_link_args(platform, &probe.libs);
    let map = scratch.join(ext_build::EXPORT_MAP_NAME);
    let export_args = ext_build::ext_export_args(platform, &map);
    if !export_args.is_empty() {
        write_ext_source(&map, &ext_build::export_map())?;
        link_args.extend(export_args);
    }
    let mut cmd = linker_command(build.target);
    if let Some(triple) = effective_link_target(build.target) {
        cmd.arg("-target").arg(triple);
    }
    cmd.args(&objects).args(&link_args);
    cmd.arg("-L").arg(&rt_lib_dir);
    add_windows_crt_math(&mut cmd);
    cmd.arg("-lpycc_rt");
    cmd.arg("-o").arg(&output.artifact);
    add_windows_system_libs(&mut cmd);
    add_linux_system_libs(&mut cmd);
    let status = crate::timing::span("package: link", || cmd.status())
        .map_err(|e| fail(linker_spawn_error(&cmd, &e)))?;
    if status.success() {
        Ok(())
    } else {
        Err(ExitCode::from(1))
    }
}

/// Checks that every interface member `dotted` compiled against is the one
/// its module's build left (a stale or rewritten interface would mean a
/// call with the wrong signature).
fn check_consumed(
    modules: &[PackageModule],
    dirs: &[PathBuf],
    dotted: &str,
    consumed: &[(String, u64)],
) -> Result<(), String> {
    for (module, hash) in consumed {
        let index = modules
            .iter()
            .position(|candidate| &candidate.dotted == module)
            .ok_or_else(|| {
                format!("internal: {dotted} read the interface of unknown `{module}`")
            })?;
        let text = std::fs::read_to_string(dirs[index].join(package_build::INTERFACE_NAME))
            .unwrap_or_default();
        if package_build::content_hash(&text) != *hash {
            return Err(format!(
                "internal: {dotted} was compiled against another interface of {module} than \
                 {module}'s build left"
            ));
        }
    }
    Ok(())
}

/// The `pycc build` of one member: its source, its dotted name, its first
/// exception tag, its output directory, and the package build's own flags.
fn member_command(
    exe: &Path,
    build: &PackageBuild<'_>,
    module: &PackageModule,
    tag_base: u8,
    dir: &Path,
) -> std::process::Command {
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("build")
        .arg(&module.path)
        .arg("-o")
        .arg(dir)
        .arg("--ext")
        // The imports between the package's modules bind through CPython:
        // each member is a module of its own.
        .arg("--foreign-relative-imports")
        .arg("--package-member")
        .arg(&module.dotted)
        .arg("--exception-tag-base")
        .arg(tag_base.to_string());
    if build.release {
        cmd.arg("--release");
    }
    if let Some(target) = build.target {
        cmd.arg("--target").arg(target);
    }
    cmd.args(&build.mode_args);
    // Each member keeps its scratch inside the package build's.
    cmd.env("TMPDIR", dir);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_member_compiled_against_another_interface_fails_the_build() {
        let scratch = pycc_scratch::ScratchDir::new("package-consumed").unwrap();
        let dirs = vec![scratch.join("m0"), scratch.join("m1")];
        for dir in &dirs {
            std::fs::create_dir_all(dir).unwrap();
        }
        let module = |dotted: &str| PackageModule {
            dotted: dotted.to_string(),
            relative: String::new(),
            path: PathBuf::new(),
            is_package: false,
        };
        let modules = [module("p.a"), module("p.b")];
        let text = "pycc-interface 1\nmodule p.a\nprefix x\n";
        std::fs::write(dirs[0].join(package_build::INTERFACE_NAME), text).unwrap();
        let hash = package_build::content_hash(text);
        assert!(check_consumed(&modules, &dirs, "p.b", &[("p.a".to_string(), hash)]).is_ok());
        let stale = check_consumed(&modules, &dirs, "p.b", &[("p.a".to_string(), hash + 1)]);
        assert!(stale.unwrap_err().contains("another interface of p.a"));
        let unknown = check_consumed(&modules, &dirs, "p.b", &[("p.z".to_string(), hash)]);
        assert!(unknown.unwrap_err().contains("unknown `p.z`"));
    }
}
