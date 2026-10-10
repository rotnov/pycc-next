//! Strict versus non-strict mode (#3).
//!
//! Strict is the default: a public function needs annotations (`T0001`) and
//! `Any` is refused (`T0002`). Non-strict lowers what cannot be proven typed
//! to the opaque CPython object, run through the C API with CPython's own
//! semantics. It is selected by `--non-strict` on `build`/`run`/`check`/
//! `lock`, or by the project file next to the entry module:
//!
//! - `pyproject.toml`: `[tool.pycc] strict = false`
//! - `pycc.toml`: `[build] strict = false`
//!
//! The flag wins; a file that is missing, unreadable or malformed, or whose
//! `strict` is anything but the boolean `false`, keeps strict mode -- the
//! same forgiving read `[build] opt` gets. The mode applies to every module
//! of the program.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// The `--non-strict` flag of the running command. Set once by `main()`
/// before any frontend work; unit tests never set it, so they stay strict.
static CLI_NON_STRICT: AtomicBool = AtomicBool::new(false);

/// Records the command line's `--non-strict`.
pub(crate) fn set_cli_non_strict(non_strict: bool) {
    CLI_NON_STRICT.store(non_strict, Ordering::Relaxed);
}

/// Whether the program whose entry module is `entry` compiles non-strict.
pub(crate) fn non_strict_for(entry: &Path) -> bool {
    CLI_NON_STRICT.load(Ordering::Relaxed) || project_says_non_strict(entry)
}

fn project_says_non_strict(entry: &Path) -> bool {
    let Some(dir) = entry.parent() else {
        return false;
    };
    let read = |name: &str| -> Option<toml::Table> {
        std::fs::read_to_string(dir.join(name))
            .ok()?
            .parse::<toml::Table>()
            .ok()
    };
    let is_false =
        |value: Option<&toml::Value>| value.and_then(toml::Value::as_bool) == Some(false);
    let pyproject = read("pyproject.toml");
    let tool_pycc = pyproject
        .as_ref()
        .and_then(|table| table.get("tool"))
        .and_then(|tool| tool.get("pycc"));
    if is_false(tool_pycc.and_then(|pycc| pycc.get("strict"))) {
        return true;
    }
    let pycc = read("pycc.toml");
    is_false(
        pycc.as_ref()
            .and_then(|table| table.get("build"))
            .and_then(|build| build.get("strict")),
    )
}

#[cfg(test)]
mod tests {
    use super::project_says_non_strict;
    use pycc_scratch::ScratchDir;

    fn entry_with(file: &str, contents: &str) -> (ScratchDir, std::path::PathBuf) {
        let dir = ScratchDir::new("mode").expect("scratch");
        std::fs::write(dir.join(file), contents).expect("write project file");
        let entry = dir.join("m.py");
        (dir, entry)
    }

    #[test]
    fn pyproject_tool_pycc_strict_false_selects_non_strict() {
        let (_dir, entry) = entry_with("pyproject.toml", "[tool.pycc]\nstrict = false\n");
        assert!(project_says_non_strict(&entry));
    }

    #[test]
    fn pycc_toml_build_strict_false_selects_non_strict() {
        let (_dir, entry) = entry_with("pycc.toml", "[build]\nstrict = false\n");
        assert!(project_says_non_strict(&entry));
    }

    #[test]
    fn anything_else_keeps_strict() {
        for (file, contents) in [
            ("pyproject.toml", "[tool.pycc]\nstrict = true\n"),
            ("pyproject.toml", "[tool.pycc]\nstrict = \"false\"\n"),
            ("pyproject.toml", "[tool.other]\nstrict = false\n"),
            ("pyproject.toml", "not toml ["),
            ("pycc.toml", "[project]\nstrict = false\n"),
        ] {
            let (_dir, entry) = entry_with(file, contents);
            assert!(!project_says_non_strict(&entry), "{file}: {contents}");
        }
        let dir = ScratchDir::new("mode_none").expect("scratch");
        assert!(!project_says_non_strict(&dir.join("m.py")));
    }
}
