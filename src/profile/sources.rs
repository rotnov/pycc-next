//! The package's current sources: its functions, as the driver indexes
//! them, with their source hashes.

use super::*;

/// One function of the package's current sources (the driver's `index`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct Function {
    pub(crate) module: String,
    pub(crate) qualname: String,
    pub(crate) firstlineno: u32,
    pub(crate) sha256: String,
    /// Its source's length in lines, decorators included.
    #[serde(default)]
    pub(crate) lines: u32,
    /// Module-level functions and methods of module-level classes that are
    /// not generators or coroutines (section 2.1).
    pub(crate) eligible: bool,
    /// Its module is kept in Python (`--exclude`): profiled, never selected.
    #[serde(default)]
    pub(crate) excluded: bool,
}

impl Function {
    /// `entry` (this function's profile entry) as [`select`] decides it.
    pub(crate) fn candidate<'p>(&self, entry: &'p Entry) -> Candidate<'p> {
        Candidate {
            entry,
            eligible: self.eligible,
            excluded: self.excluded,
            lines: self.lines,
            winning: Winning::No,
        }
    }
}

/// The package's current functions, from `python` (the driver's `index`).
pub(crate) fn index(python: &OsStr, pkg: &Path) -> Result<Vec<Function>, String> {
    index_excluding(python, pkg, &[])
}

/// [`index`], with the functions of each module `excludes` names (as a
/// package build's `--exclude`: a module or subpackage, relative to the
/// package or with its name first) marked kept in Python.
pub(crate) fn index_excluding(python: &OsStr, pkg: &Path, excludes: &[String]) -> Result<Vec<Function>, String> {
    let mut functions = index_all(python, pkg)?;
    if !excludes.is_empty() && pkg.is_dir() {
        let package = pkg.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // The functions' modules are dotted from the package's import root.
        let prefix = functions
            .iter()
            .find_map(|f| {
                let at = f.module.split('.').position(|part| part == package)?;
                Some(f.module.split('.').take(at + 1).collect::<Vec<_>>().join("."))
            })
            .unwrap_or_else(|| package.clone());
        for f in &mut functions {
            let relative = if f.module == prefix {
                ""
            } else {
                f.module.strip_prefix(&prefix).and_then(|rest| rest.strip_prefix('.')).unwrap_or(&f.module)
            };
            f.excluded = crate::package_build::excluded(&package, relative, excludes);
        }
    }
    Ok(functions)
}

/// `python`'s version (`platform.python_version()`).
pub(crate) fn python_version(python: &OsStr) -> Result<String, String> {
    let output = std::process::Command::new(python)
        .args(["-I", "-c", "import platform; print(platform.python_version())"])
        .output()
        .map_err(|e| format!("cannot run {}: {e}", python.to_string_lossy()))?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn index_all(python: &OsStr, pkg: &Path) -> Result<Vec<Function>, String> {
    let scratch = pycc_scratch::ScratchDir::new("profile").map_err(|e| e.to_string())?;
    let script = scratch.join("driver.py");
    std::fs::write(&script, DRIVER.replace("\r\n", "\n")).map_err(|e| e.to_string())?;
    let driver = Driver {
        python,
        script: &script,
        pkg,
        origin: None,
    };
    let out = scratch.join("index.json");
    driver.run("index", &out, &[], &[])?;
    Driver::json(&out)
}
