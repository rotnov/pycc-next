//! The C sources of an `--ext` or embedded build (the boundary shim, the
//! launcher), compiled to objects while LLVM compiles the program
//! (v0.5 accept 4).
//!
//! The link used to compile them itself, after codegen: the shim alone is
//! some 6,000 lines of C at `-O2`, about a second of every `--ext` build,
//! spent while nothing else ran. Each source now gets its own `cc -c`
//! child as soon as the plan has written it, before codegen starts; the
//! link waits for them and takes their objects instead. The compile flags
//! and the sources are exactly the plan's `compile_args`, so what is
//! compiled, and how, is unchanged.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode};

/// The object file extension the platform's linker expects.
const OBJECT_EXTENSION: &str = if cfg!(windows) { "obj" } else { "o" };

/// One `cc -c` per C source of a plan, running.
pub(super) struct CObjects {
    children: Vec<(Child, PathBuf)>,
}

/// Splits a plan's `compile_args` into flags and C sources.
fn split(args: &[OsString]) -> (Vec<&OsString>, Vec<&OsString>) {
    args.iter()
        .partition(|arg| !Path::new(arg).extension().is_some_and(|ext| ext == "c"))
}

impl CObjects {
    /// Starts compiling every C source among `compile_args` into `dir`,
    /// each with every flag among them. `command` makes the driver (the
    /// linker's own, with its `-target`).
    pub(super) fn spawn(
        compile_args: &[OsString],
        dir: &Path,
        command: impl Fn() -> Command,
    ) -> Result<Self, ExitCode> {
        let (flags, sources) = split(compile_args);
        let mut children = Vec::with_capacity(sources.len());
        for source in sources {
            let stem = Path::new(source)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let object = dir.join(format!("{stem}.{OBJECT_EXTENSION}"));
            let mut cmd = command();
            cmd.arg("-c")
                .args(&flags)
                .arg(source)
                .arg("-o")
                .arg(&object);
            let child = cmd.spawn().map_err(|e| {
                eprintln!("error: {}", super::linker_spawn_error(&cmd, &e));
                ExitCode::from(2)
            })?;
            children.push((child, object));
        }
        Ok(Self { children })
    }

    /// Waits for every compile; their objects, in source order, once all
    /// succeeded. A failed compile has printed its own diagnostics.
    pub(super) fn wait(mut self) -> Result<Vec<PathBuf>, ExitCode> {
        let children = std::mem::take(&mut self.children);
        let mut objects = Vec::with_capacity(children.len());
        let mut failed = false;
        for (mut child, object) in children {
            match child.wait() {
                Ok(status) if status.success() => objects.push(object),
                Ok(_) => failed = true,
                Err(e) => {
                    eprintln!("error: waiting for the C compiler failed: {e}");
                    failed = true;
                }
            }
        }
        if failed {
            Err(ExitCode::from(1))
        } else {
            Ok(objects)
        }
    }
}

/// A build that stops before the link (a codegen failure) stops its
/// compiles too, so none writes into the removed scratch directory or
/// reports on stderr after `pycc` has exited.
impl Drop for CObjects {
    fn drop(&mut self) {
        for (child, _) in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flags_and_the_sources_are_told_apart_by_the_c_extension() {
        let args: Vec<OsString> = ["-O2", "-I", "/inc", "-fPIC", "/s/shim.c", "/s/launcher.c"]
            .into_iter()
            .map(OsString::from)
            .collect();
        let (flags, sources) = split(&args);
        assert_eq!(flags, ["-O2", "-I", "/inc", "-fPIC"]);
        assert_eq!(sources, ["/s/shim.c", "/s/launcher.c"]);
    }

    #[cfg(unix)]
    #[test]
    fn each_source_becomes_an_object_beside_the_others() {
        let scratch = pycc_scratch::ScratchDir::new("c-objects").unwrap();
        let source = scratch.join("unit.c");
        std::fs::write(&source, "int pycc_c_objects_test(void) { return 1; }\n").unwrap();
        let args = vec![OsString::from("-O2"), source.into_os_string()];
        let objects = CObjects::spawn(&args, &scratch, || Command::new("cc"))
            .unwrap()
            .wait()
            .unwrap();
        assert_eq!(objects, [scratch.join(format!("unit.{OBJECT_EXTENSION}"))]);
        assert!(objects[0].is_file());
    }

    #[cfg(unix)]
    #[test]
    fn a_source_that_does_not_compile_fails_the_wait() {
        let scratch = pycc_scratch::ScratchDir::new("c-objects-bad").unwrap();
        let source = scratch.join("bad.c");
        std::fs::write(&source, "this is not C\n").unwrap();
        let args = vec![source.into_os_string()];
        let compiles = CObjects::spawn(&args, &scratch, || Command::new("cc")).unwrap();
        assert!(compiles.wait().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn dropping_unwaited_compiles_stops_them() {
        let scratch = pycc_scratch::ScratchDir::new("c-objects-drop").unwrap();
        let args = vec![OsString::from("x.c")];
        let compiles = CObjects::spawn(&args, &scratch, || {
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg("sleep 30").arg("sh");
            cmd
        })
        .unwrap();
        let started = std::time::Instant::now();
        drop(compiles);
        assert!(started.elapsed() < std::time::Duration::from_secs(20));
    }

    #[test]
    fn a_driver_that_cannot_start_is_an_environment_failure() {
        let args = vec![OsString::from("x.c")];
        let result = CObjects::spawn(&args, Path::new("."), || {
            Command::new("/nonexistent/pycc-test-cc")
        });
        assert!(result.is_err());
    }
}
