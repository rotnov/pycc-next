use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, PartialEq)]
pub struct PyccToml {
    pub project: ProjectSection,
    #[serde(default)]
    pub build: BuildSection,
    /// The raw `[interop]` table (D-128, #1224), kept unvalidated here on
    /// purpose. `parse` is shared with the module loader's project-root
    /// discovery, which also runs for `pycc build --ext`, and D-244 rule 3
    /// has `--ext` ignore the table: validating it here would let a bad
    /// `[interop]` break an `--ext` build. `crate::interop_policy` validates
    /// it only on the paths the policy governs. A `toml::Value` rather than
    /// a table type, so a stray top-level `interop = "deny"` still parses
    /// here and is refused by that validator instead.
    #[serde(default)]
    pub interop: Option<toml::Value>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct ProjectSection {
    pub name: String,
    pub entry: String,
    pub python: String,
}

#[derive(Debug, Deserialize, PartialEq, Default)]
pub struct BuildSection {
    pub opt: Option<String>,
    pub targets: Option<Vec<String>>,
    /// `static = true` links an embedded build's libpython statically, as
    /// `pycc build --static-libpython` does (D-251); read only by `pycc
    /// build`, and ignored by `--ext` and `pycc run`.
    #[serde(rename = "static")]
    pub static_: Option<bool>,
}

/// v1 accepts exactly Python 3.14 (D-012) -- a `pycc.toml` naming any other
/// version is a validation error, not a silent accept of an unsupported
/// language level.
pub fn parse(contents: &str) -> Result<PyccToml, String> {
    let config: PyccToml = toml::from_str(contents).map_err(|e| e.to_string())?;
    if config.project.python != "3.14" {
        return Err(format!(
            "pycc.toml: unsupported python version `{}` -- v1 accepts exactly \"3.14\" (D-012)",
            config.project.python
        ));
    }
    Ok(config)
}

/// Serialization-only wrapper matching `pycc.toml`'s `[project]` table
/// shape. `scaffold` writes only this section (never `[build]`, which
/// `PyccToml`'s own `BuildSection` -- all `Option` fields -- would need
/// `#[serde(skip_serializing_if = "Option::is_none")]` to serialize at
/// all, since the `toml` crate has no representation for TOML-absent
/// `None` values). Reusing `ProjectSection` directly (rather than
/// hand-formatting a string) is what makes `project_name` safe to
/// serialize regardless of its content -- see `scaffold`'s doc comment.
#[derive(Serialize)]
struct ScaffoldToml<'a> {
    project: &'a ProjectSection,
}

/// `pycc init [NAME]`: scaffolds a starter `pycc.toml` + `src/main.py` in
/// `dir`. `name` defaults to `dir`'s own file-name component when omitted,
/// matching how `cargo init`/`npm init` derive a project name from the
/// target directory.
///
/// Refusal contract (#237): before writing anything, every destination is
/// inspected, and the scaffold returns an `ErrorKind::AlreadyExists` error
/// -- "`pycc.toml` already exists", "`src` exists and is not a directory",
/// or "`src/main.py` already exists" -- leaving all existing paths
/// byte-for-byte unchanged. An existing `src` *directory* is not itself a
/// conflict (only its entry type and `main.py`'s presence are checked). A
/// missing `dir` is created. `pycc.toml` is written last, with
/// `create_new`, so a late failure never leaves it behind and a dangling
/// symlink at either file destination fails cleanly instead of writing
/// through to the symlink's target.
pub fn scaffold(name: Option<&str>, dir: &std::path::Path) -> std::io::Result<()> {
    let project_name = name
        .map(str::to_string)
        .or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "myapp".to_string());

    let project = ProjectSection {
        name: project_name,
        entry: "src/main.py".to_string(),
        python: "3.14".to_string(),
    };
    // Serializing through `toml::to_string` (rather than hand-formatting
    // `name = "{project_name}"` into a string) is what makes this safe
    // for *any* `project_name`, including one containing a `"` or `\`
    // (from an explicit `pycc init 'my"app'` or an oddly-named target
    // directory) -- the `toml` crate picks a literal (single-quoted)
    // string or escapes as needed, verified empirically (`name =
    // 'my"app\weird'` round-trips through `parse` unchanged). A hand-
    // formatted string would instead emit syntactically invalid TOML for
    // such a name, and the self-check below would panic instead of
    // returning a clean error.
    let toml_contents = toml::to_string(&ScaffoldToml { project: &project })
        .expect("ProjectSection must always serialize to valid TOML");
    // Self-check: the scaffolded file must itself satisfy `parse`'s own
    // validation (including D-012's exact-\"3.14\" check). This is a
    // genuine invariant of this function's own construction -- `entry`
    // and `python` are hardcoded literals, and `name` is safely escaped
    // above regardless of its content -- so `.expect()` documents a real
    // assertion rather than threading a `Result` no actual input can
    // produce (see docs/TESTING.md's `.expect()`-for-genuine-invariants
    // convention). It also gives `parse` a real production call site:
    // `pycc build`'s own use of a neighboring `pycc.toml`'s `build.opt`
    // as a default profile is a separate, larger consumption point that
    // lands together with the `--release` flag it depends on (this PR's
    // Task 3).
    parse(&toml_contents).expect("scaffold's own generated pycc.toml must parse and validate");

    // #237: inspect every destination before writing anything, so an
    // existing project is never silently replaced (the pre-check phase) and
    // a later failure can never leave a half-written scaffold over user
    // files (the write order below). Checks use follow-symlink semantics
    // deliberately: a symlink to a real file is user content and must
    // refuse, and a symlink to a directory is a usable `src`. A *dangling*
    // symlink passes these checks (its target does not exist), which is
    // why both file writes below use `create_new`: plain `fs::write` would
    // follow such a symlink and could silently create the file *outside*
    // the target directory when the symlink's target parent exists -- on
    // POSIX platforms `create_new` refuses the existing symlink entry
    // instead (O_EXCL never resolves symlinks; the Windows reparse-point
    // path shares W3's accepted cfg(unix) test gap). The checks
    // are a single-process guarantee, not an atomic one: nothing locks the
    // directory between check and write, the same non-atomicity
    // `cargo init`-family tools accept; `create_new` narrows that window
    // for the two files.
    let toml_path = dir.join("pycc.toml");
    let src_dir = dir.join("src");
    let main_py_path = src_dir.join("main.py");
    if toml_path.try_exists()? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "`pycc.toml` already exists",
        ));
    }
    // #256: captured (not discarded) so a later rollback knows whether
    // *this invocation* created `src/`, and so never removes a directory
    // that pre-existed it.
    let src_dir_created = match std::fs::metadata(&src_dir) {
        Ok(meta) if !meta.is_dir() => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "`src` exists and is not a directory",
            ));
        }
        // An existing `src` *directory* is not by itself a conflict:
        // `create_dir_all` below tolerates it, and an empty `src/` next to
        // no `pycc.toml`/`main.py` carries no user content to destroy.
        Ok(_) => false,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => return Err(error),
    };
    if main_py_path.try_exists()? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "`src/main.py` already exists",
        ));
    }

    // Write order (#237): `pycc.toml` goes LAST, so any late failure in
    // the `src` steps leaves it uncreated -- the strongest form of the
    // issue's "a late write failure leaves `pycc.toml` unchanged"
    // regression. Only a failure in the *final* `pycc.toml` write below
    // triggers rollback (#256): if `create_dir_all` or `main.py`'s own
    // write fails here instead, the `?` propagates directly and an
    // invocation-created, still-empty `src/` can still be left behind --
    // the same accepted residue `cargo init`-family tools leave, and not a
    // conflict on the next `pycc init` (an empty `src/` was already a
    // non-conflict before this issue).
    std::fs::create_dir_all(&src_dir)?;
    let main_py = "def main() -> None:\n    print(\"hello from pycc\")\n";
    write_new(&main_py_path, main_py.as_bytes())?;
    if let Err(toml_error) = write_new(&toml_path, toml_contents.as_bytes()) {
        return Err(roll_back_after_toml_failure(
            toml_error,
            &main_py_path,
            &src_dir,
            src_dir_created,
        ));
    }
    Ok(())
}

/// #256: `pycc.toml` is the last mutation (see the write order above). If
/// it fails after `main.py` was already created by *this* invocation, that
/// `main.py` is residue nothing else will ever clean up -- the next `pycc
/// init`, even after the underlying cause is fixed, would refuse it as a
/// pre-existing user file. This function only ever removes `main_py_path`
/// and, conditionally, `src_dir` -- both unconditionally owned by this
/// invocation per the caller's pre-check (a pre-existing `src/` directory,
/// and anything already in it, is never removed, matching #237's "never
/// overwrite or destroy pre-existing user content" contract). It does not
/// remove `dir` itself even when `create_dir_all` also created `dir` (not
/// just `src/`): `scaffold`'s own doc comment already scopes that case out,
/// since the public `pycc init` surface always passes an existing current
/// directory, so an invocation-created `dir` is a library-only corner case
/// left as accepted residue, the same as an earlier `create_dir_all`/
/// `main.py`-write failure already leaves behind above.
fn roll_back_after_toml_failure(
    toml_error: std::io::Error,
    main_py_path: &std::path::Path,
    src_dir: &std::path::Path,
    src_dir_created: bool,
) -> std::io::Error {
    if let Err(rollback_error) = std::fs::remove_file(main_py_path) {
        // Unlike `write_new_in`'s own best-effort cleanup of the file whose
        // *own* write just failed, a failure here is reported rather than
        // swallowed: this removes an *earlier sibling's* already-successful
        // write, and silently leaving that unreported is exactly what
        // produces the residue this issue is about.
        return std::io::Error::new(
            toml_error.kind(),
            format!(
                "{toml_error}; additionally, could not remove the `src/main.py` \
                 this invocation created ({rollback_error}) -- remove it manually \
                 before retrying"
            ),
        );
    }
    if src_dir_created {
        // `remove_dir` (not `remove_dir_all`) refuses a non-empty directory
        // by construction, so this can never remove anything beyond what
        // `main.py`'s own removal above just emptied -- no separate
        // emptiness check that could race. A refusal here (something else
        // is unexpectedly inside) is not this invocation's own residue by
        // definition, so it is left alone without being reported, the same
        // way an empty pre-existing `src/` was already an accepted
        // non-conflict before this issue.
        let _ = std::fs::remove_dir(src_dir);
    }
    toml_error
}

/// `fs::write` with `create_new`: fails if the path already names any
/// entry, including a dangling symlink, instead of following it (#237).
/// When the write itself fails after `create_new` succeeded, the freshly
/// created file is removed, so a quota or I/O failure cannot leave behind
/// an empty/partial `main.py` that makes the next `pycc init` refuse our
/// own residue, or a corrupt partial `pycc.toml` -- `create_new` guarantees
/// the entry is ours to delete (PR #253's review). That removal is no
/// longer silently best-effort (#256's review): if it too fails, the
/// cleanup error is folded into the returned error rather than discarded,
/// since a caller told only "the write failed" would otherwise have no way
/// to learn a partially written file is still sitting at `path`.
fn write_new(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    write_new_in(path, contents, |file, contents| {
        use std::io::Write;
        file.write_all(contents)
    })
}

/// Testable core of [`write_new`], following the same dependency-injection
/// split as `find_pycc_rt_lib_dir_in` in
/// `pycc_codegen::artifact_layout`: the injected writer lets a test exercise the
/// cleanup-on-write-failure path deterministically, which no portable
/// filesystem setup can trigger for a real `write_all`. A plain `fn`
/// pointer (not `impl FnOnce`) on purpose: a generic parameter would
/// monomorphize one copy per closure, and the production copy's
/// cleanup branch would count as uncovered under D-014's region gate even
/// though the test copy exercises the same source.
fn write_new_in(
    path: &std::path::Path,
    contents: &[u8],
    write: fn(&mut std::fs::File, &[u8]) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let write_error = match write(&mut file, contents) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    // Close the handle before removing -- Windows cannot delete an open
    // file.
    drop(file);
    if let Err(cleanup_error) = std::fs::remove_file(path) {
        // #256's review: unlike a bare best-effort cleanup, silently
        // discarding this would leave the partially written entry at
        // `path` on disk with no caller ever told it exists. For the
        // `pycc.toml` write specifically, `roll_back_after_toml_failure`
        // above only ever sees the error this function returns, so a
        // swallowed cleanup failure here would leave a partial `pycc.toml`
        // behind while every reported message still claimed only `main.py`
        // needed manual removal -- the same residue this whole issue is
        // about, one layer earlier and unreported. Report it for every
        // caller of `write_new`, not just the `pycc.toml` one, since the
        // failure mode is identical either way.
        return Err(std::io::Error::new(
            write_error.kind(),
            format!(
                "{write_error}; additionally, could not remove the \
                 partially written file at {path:?} ({cleanup_error}) \
                 -- remove it manually before retrying"
            ),
        ));
    }
    Err(write_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pycc_scratch::ScratchDir;

    #[test]
    fn parses_a_minimal_valid_pycc_toml() {
        let toml = r#"
[project]
name = "myapp"
entry = "src/main.py"
python = "3.14"

[build]
opt = "release"
targets = ["x86_64-unknown-linux-gnu"]
static = true
"#;
        let config = parse(toml).expect("valid pycc.toml should parse");
        assert_eq!(config.project.name, "myapp");
        assert_eq!(config.project.entry, "src/main.py");
        assert_eq!(config.project.python, "3.14");
        assert_eq!(config.build.opt.as_deref(), Some("release"));
        assert_eq!(
            config.build.targets.as_deref(),
            Some(&["x86_64-unknown-linux-gnu".to_string()][..])
        );
        assert_eq!(config.build.static_, Some(true));
    }

    #[test]
    fn rejects_an_unsupported_python_version() {
        let toml = r#"
[project]
name = "myapp"
entry = "src/main.py"
python = "3.15"
"#;
        let err = parse(toml).expect_err("python != 3.14 must be rejected in v1");
        assert!(
            err.contains("3.14"),
            "error should mention the only supported version: {err}"
        );
    }

    #[test]
    fn accepts_a_file_with_not_yet_implemented_sections() {
        // [test] is documented in docs/CLI_SPEC.md for a later milestone, and
        // [interop] is kept raw here (#1224): `parse` still accepts this
        // shape, while `crate::interop_policy`'s validator -- which runs only
        // for a program with a CPython import -- rejects a non-empty `allow`
        // without `policy = "allowlist"`.
        let toml = r#"
[project]
name = "myapp"
entry = "src/main.py"
python = "3.14"

[interop]
allow = ["numpy"]

[test]
paths = ["tests/"]
"#;
        parse(toml).expect("documented-but-not-yet-consumed sections must not fail parsing");
    }

    #[test]
    fn rejects_malformed_toml_syntax() {
        let err = parse("this is not [valid toml").expect_err("malformed TOML must be rejected");
        assert!(!err.is_empty());
    }

    #[test]
    fn scaffold_writes_a_valid_pycc_toml_and_main_py() {
        let dir = ScratchDir::new("init_test").expect("failed to create scratch dir");

        scaffold(Some("scaffoldtest"), &dir).expect("scaffold should succeed");

        let toml_contents = std::fs::read_to_string(dir.join("pycc.toml")).unwrap();
        let config = parse(&toml_contents).expect("scaffolded pycc.toml must itself parse");
        assert_eq!(config.project.name, "scaffoldtest");

        let main_py = std::fs::read_to_string(dir.join("src").join("main.py")).unwrap();
        assert!(main_py.contains("def main"));
    }

    #[test]
    fn scaffold_derives_the_project_name_from_the_directory_when_none_is_given() {
        // `name: None` with a directory that does have a `file_name()`
        // component -- this exercises `scaffold`'s middle name-resolution
        // branch (derive from the target directory), distinct from both
        // the explicit-name test above and the no-file-name fallback test
        // below.
        let dir = ScratchDir::new("derived_name").expect("failed to create scratch dir");

        scaffold(None, &dir).expect("scaffold should succeed");

        let toml_contents = std::fs::read_to_string(dir.join("pycc.toml")).unwrap();
        let config = parse(&toml_contents).expect("scaffolded pycc.toml must itself parse");
        let expected_name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(config.project.name, expected_name);
    }

    #[test]
    fn scaffold_falls_back_to_myapp_when_the_directory_has_no_file_name() {
        // `existing_subdir.join("..")` has no `file_name()` component at
        // all (it ends in ".."), exercising `scaffold`'s final
        // `unwrap_or_else("myapp")` fallback -- but unlike a bare
        // nonexistent-parent path, it resolves (on every Tier-1 platform)
        // to `existing_subdir`'s own real, already-existing, writable
        // parent directory, since both a lexical `..`-collapsing resolver
        // (Windows) and a component-by-component one (POSIX) agree on the
        // answer once every intermediate component genuinely exists
        // (verified empirically: writing there succeeds and lands in the
        // parent). This keeps the test independent of the filesystem-
        // mutation injection scenarios covered further below.
        let dir = ScratchDir::new("myapp_fallback").expect("failed to create scratch dir");
        let existing_subdir = dir.join("existing_subdir");
        std::fs::create_dir_all(&existing_subdir).unwrap();
        let target = existing_subdir.join("..");
        assert!(target.file_name().is_none());

        scaffold(None, &target).expect("scaffold should succeed in the existing parent");

        let toml_contents = std::fs::read_to_string(dir.join("pycc.toml")).unwrap();
        let config = parse(&toml_contents).expect("scaffolded pycc.toml must itself parse");
        assert_eq!(config.project.name, "myapp");
    }

    #[test]
    fn scaffold_creates_a_missing_target_directory() {
        // Behavior change under #237's inverted write order: the first
        // mutation is now `create_dir_all(dir/src)`, which creates the
        // missing target directory and its parents, so scaffolding into a
        // nonexistent directory succeeds (matching `cargo init`-family
        // behavior). The public `pycc init` surface is unaffected -- it
        // always passes the existing current directory. This test used to
        // pin the old order's `NotFound` from writing `pycc.toml` first;
        // the Err arms are now covered by the injection trio below.
        // `ScratchDir::new` unconditionally creates its own directory, so
        // the deliberately-nonexistent target this test needs is a child of
        // a real scratch parent rather than the scratch dir itself -- the
        // parent's Drop still removes `dir` recursively once it exists.
        let parent =
            ScratchDir::new("nonexistent_target_dir").expect("failed to create scratch dir");
        let dir = parent.join("nonexistent");
        assert!(!dir.exists());

        scaffold(Some("x"), &dir).expect("scaffold should create the missing directory");
        assert!(dir.join("pycc.toml").is_file());
        assert!(dir.join("src").join("main.py").is_file());
    }

    #[test]
    fn scaffold_handles_a_project_name_containing_toml_special_characters() {
        // A name with a `"` and a `\` would produce syntactically invalid
        // TOML if hand-formatted into a string literal -- this is exactly
        // the case the `toml::to_string`-based serialization in `scaffold`
        // (rather than `format!`) exists to handle safely: the round trip
        // through `parse` must recover the exact original name, and
        // `scaffold` must not panic.
        let dir = ScratchDir::new("special_name").expect("failed to create scratch dir");
        let tricky_name = "my\"app\\weird";

        scaffold(Some(tricky_name), &dir).expect("scaffold must not panic on a special-char name");

        let toml_contents = std::fs::read_to_string(dir.join("pycc.toml")).unwrap();
        let config = parse(&toml_contents).expect("scaffolded pycc.toml must itself parse");
        assert_eq!(config.project.name, tricky_name);
    }

    #[test]
    fn scaffold_refuses_when_src_exists_as_a_plain_file() {
        // #237: `dir/src` existing as a regular file is caught by the
        // pre-check phase (it used to surface later, as `create_dir_all`'s
        // own error after `pycc.toml` had already been replaced). Nothing
        // is written on refusal.
        let dir = ScratchDir::new("src_conflict").expect("failed to create scratch dir");
        std::fs::write(dir.join("src"), "not a directory").unwrap();

        let err = scaffold(Some("x"), &dir).expect_err("scaffold must refuse a non-directory src");
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(err.to_string(), "`src` exists and is not a directory");
        assert!(!dir.join("pycc.toml").exists());
    }

    #[test]
    fn scaffold_refuses_when_main_py_already_exists() {
        // #237: `dir/src/main.py` already existing (here as a directory,
        // any entry type counts) is caught by the pre-check phase, and
        // nothing is written -- it used to surface as the final write's
        // own error after `pycc.toml` had already been replaced.
        let dir = ScratchDir::new("main_py_conflict").expect("failed to create scratch dir");
        std::fs::create_dir_all(dir.join("src").join("main.py")).unwrap();

        let err = scaffold(Some("x"), &dir).expect_err("scaffold must refuse an existing main.py");
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(err.to_string(), "`src/main.py` already exists");
        assert!(!dir.join("pycc.toml").exists());
    }

    #[test]
    fn scaffold_refuses_when_pycc_toml_already_exists() {
        // #237's headline defect: an existing `pycc.toml` was silently
        // replaced. Refusal must leave it byte-for-byte unchanged.
        let dir = ScratchDir::new("toml_conflict").expect("failed to create scratch dir");
        std::fs::write(dir.join("pycc.toml"), "user content").unwrap();

        let err =
            scaffold(Some("x"), &dir).expect_err("scaffold must refuse an existing pycc.toml");
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(err.to_string(), "`pycc.toml` already exists");
        assert_eq!(
            std::fs::read_to_string(dir.join("pycc.toml")).unwrap(),
            "user content"
        );
        assert!(!dir.join("src").exists());
    }

    #[test]
    fn scaffold_accepts_a_preexisting_empty_src_directory() {
        // #237's deliberate non-conflict: an existing `src/` directory is
        // not itself a conflict -- only its entry type and `main.py`'s
        // presence are checked.
        let dir = ScratchDir::new("empty_src").expect("failed to create scratch dir");
        std::fs::create_dir_all(dir.join("src")).unwrap();

        scaffold(Some("x"), &dir).expect("an empty src directory is not a conflict");
        assert!(dir.join("pycc.toml").is_file());
        assert!(dir.join("src").join("main.py").is_file());
    }

    #[test]
    fn scaffold_accepts_a_populated_src_directory_without_main_py() {
        // The acceptance condition is deliberately "any directory without
        // a `main.py`", not emptiness: existing sibling files are left
        // untouched and scaffolded alongside.
        let dir = ScratchDir::new("populated_src").expect("failed to create scratch dir");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src").join("lib.py"), "user lib").unwrap();

        scaffold(Some("x"), &dir).expect("a src directory without main.py is not a conflict");
        assert!(dir.join("pycc.toml").is_file());
        assert!(dir.join("src").join("main.py").is_file());
        assert_eq!(
            std::fs::read_to_string(dir.join("src").join("lib.py")).unwrap(),
            "user lib"
        );
    }

    #[test]
    fn write_new_removes_its_partial_file_when_the_write_fails() {
        // PR #253's review: a write failure after `create_new` succeeded
        // must not leave the freshly created entry behind. The injected
        // failing writer is the only portable way to trigger this path.
        let dir = ScratchDir::new("write_new_fail").expect("failed to create scratch dir");
        let path = dir.join("partial.txt");

        let err = write_new_in(&path, b"contents", |_, _| {
            Err(std::io::Error::other("injected write failure"))
        })
        .expect_err("the injected write failure must propagate");
        assert_eq!(err.to_string(), "injected write failure");
        assert!(!path.exists(), "the partial file must be cleaned up");
    }

    /// The scratch directory that `write_new_in_reports_a_cleanup_failure_honestly`
    /// makes read-only, published for `chmod_containing_dir_readonly_then_fail_write`
    /// below. A `OnceLock` because `write_new_in` requires a capture-free
    /// `fn` pointer (see its own doc comment), which can neither close over
    /// the test's `ScratchDir` nor recompute its name -- `ScratchDir::new`
    /// embeds a per-call nanosecond timestamp and atomic sequence number
    /// precisely so that no two calls agree on a path. Set exactly once, by
    /// that one test, before it injects the writer.
    #[cfg(unix)]
    static CLEANUP_FAIL_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

    #[cfg(unix)]
    fn chmod_containing_dir_readonly_then_fail_write(
        _file: &mut std::fs::File,
        _contents: &[u8],
    ) -> std::io::Result<()> {
        // A plain top-level `fn`, not a closure: `write_new_in` requires a
        // capture-free `fn` pointer (see its own doc comment), so the
        // directory to make read-only arrives through `CLEANUP_FAIL_DIR`
        // above rather than through a capture.
        use std::os::unix::fs::PermissionsExt;
        let dir = CLEANUP_FAIL_DIR
            .get()
            .expect("the owning test publishes the scratch directory before injecting this writer");
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        Err(std::io::Error::other("injected write failure"))
    }

    #[cfg(unix)]
    #[test]
    fn write_new_in_reports_a_cleanup_failure_honestly() {
        // #256's review: if the write fails *and* the best-effort cleanup
        // of the partially-created file also fails, the cleanup failure
        // must not be silently discarded -- a caller told only "the write
        // failed" would have no way to learn a partial file is still on
        // disk. The injected writer chmods the containing directory
        // read-only after `create_new` already succeeded, so the later
        // `remove_file` in `write_new_in` fails deterministically.
        // (Root ignores 0o555, so this would spuriously succeed under a
        // root test runner; hosted CI and the coverage sandbox's `nobody`
        // user are both non-root.)
        use std::os::unix::fs::PermissionsExt;
        let dir = ScratchDir::new("write_new_cleanup_fail").expect("failed to create scratch dir");
        CLEANUP_FAIL_DIR
            .set(dir.to_path_buf())
            .expect("only this test sets CLEANUP_FAIL_DIR");
        let path = dir.join("partial.txt");

        let result = write_new_in(
            &path,
            b"contents",
            chmod_containing_dir_readonly_then_fail_write,
        );
        // Restore write permission before any assertion can panic, so the
        // scratch dir's Drop can remove it even when this test fails.
        std::fs::set_permissions(&*dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        let err = result.expect_err("the injected write failure must propagate");
        let message = err.to_string();
        assert!(message.contains("injected write failure"));
        assert!(message.contains("could not remove"));
        assert!(message.contains("remove it manually"));
    }

    #[test]
    fn write_new_succeeds_and_keeps_the_file_on_a_clean_write() {
        let dir = ScratchDir::new("write_new_ok").expect("failed to create scratch dir");
        let path = dir.join("whole.txt");

        write_new(&path, b"contents").expect("a clean write must succeed");
        assert_eq!(std::fs::read(&path).unwrap(), b"contents");
    }

    #[cfg(unix)]
    #[test]
    fn scaffold_propagates_a_pycc_toml_existence_check_error() {
        // A self-referential `pycc.toml` symlink makes `try_exists` itself
        // fail (`ELOOP`), covering the first check's `?` propagation --
        // distinct from a dangling symlink, which resolves to Ok(false).
        let dir = ScratchDir::new("toml_eloop").expect("failed to create scratch dir");
        std::os::unix::fs::symlink(dir.join("pycc.toml"), dir.join("pycc.toml")).unwrap();

        let err = scaffold(Some("x"), &dir).expect_err("the existence check must propagate ELOOP");
        assert!(!err.to_string().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn scaffold_propagates_a_src_metadata_error_other_than_not_found() {
        // A self-referential `src` symlink makes `metadata` fail with
        // `ELOOP` -- neither a usable directory nor `NotFound` -- covering
        // the check's error-propagation arm.
        let dir = ScratchDir::new("src_eloop").expect("failed to create scratch dir");
        std::os::unix::fs::symlink(dir.join("src"), dir.join("src")).unwrap();

        let err = scaffold(Some("x"), &dir).expect_err("the src metadata check must propagate");
        assert_ne!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(!dir.join("pycc.toml").exists());
    }

    #[cfg(unix)]
    #[test]
    fn scaffold_propagates_a_main_py_existence_check_error() {
        // Same `ELOOP` shape for the third check: `src` is a real
        // directory, `src/main.py` is a self-referential symlink.
        let dir = ScratchDir::new("py_eloop").expect("failed to create scratch dir");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::os::unix::fs::symlink(
            dir.join("src").join("main.py"),
            dir.join("src").join("main.py"),
        )
        .unwrap();

        let err = scaffold(Some("x"), &dir).expect_err("the main.py check must propagate ELOOP");
        assert!(!err.to_string().is_empty());
        assert!(!dir.join("pycc.toml").exists());
    }

    #[cfg(unix)]
    #[test]
    fn scaffold_fails_cleanly_when_src_is_a_dangling_symlink() {
        // Injection 1 of #237's late-failure trio: a dangling `src`
        // symlink passes the follow-symlink pre-check (its target does not
        // exist), then `create_dir_all` fails on the symlink itself.
        // `pycc.toml` must not be created.
        let dir = ScratchDir::new("src_dangling").expect("failed to create scratch dir");
        std::os::unix::fs::symlink(dir.join("nonexistent_target"), dir.join("src")).unwrap();

        let err = scaffold(Some("x"), &dir).expect_err("create_dir_all must fail on the symlink");
        assert!(!err.to_string().is_empty());
        assert!(!dir.join("pycc.toml").exists());
    }

    #[cfg(unix)]
    #[test]
    fn scaffold_fails_cleanly_when_src_is_read_only() {
        // Injection 2: an existing empty `src/` passes the checks, the
        // directory-creation step is a no-op, and the `main.py` write
        // fails with a permission error. `pycc.toml` must not be created.
        // (Root ignores 0o555, so this would spuriously succeed under a
        // root test runner; hosted CI and the coverage sandbox's `nobody`
        // user are both non-root.)
        use std::os::unix::fs::PermissionsExt;
        let dir = ScratchDir::new("src_readonly").expect("failed to create scratch dir");
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::set_permissions(&src, std::fs::Permissions::from_mode(0o555)).unwrap();

        let result = scaffold(Some("x"), &dir);
        // Restore write permission before any assertion can panic, so the
        // scratch dir's Drop can remove `src` even when this test fails.
        std::fs::set_permissions(&src, std::fs::Permissions::from_mode(0o755)).ok();

        let err = result.expect_err("the main.py write must fail");
        assert!(!err.to_string().is_empty());
        assert!(!dir.join("pycc.toml").exists());
    }

    #[cfg(unix)]
    #[test]
    fn scaffold_rolls_back_main_py_when_pycc_toml_fails_last() {
        // #256, inverting the pre-#256 version of this test: a dangling
        // `pycc.toml` symlink passes the follow-symlink pre-check (its
        // target does not exist), the `src` steps succeed, and the final
        // `write_new` fails immediately with `EEXIST` on the symlink entry
        // itself -- `create_new` never follows it (POSIX O_EXCL-on-symlink).
        // `main.py`, already created by this same invocation, is now
        // residue nothing would otherwise clean up, so it must be rolled
        // back rather than left behind (issue #256's item 1 / regression 4
        // "at the last possible moment").
        let dir = ScratchDir::new("toml_dangling").expect("failed to create scratch dir");
        std::os::unix::fs::symlink(
            dir.join("missing_dir").join("pycc.toml"),
            dir.join("pycc.toml"),
        )
        .unwrap();

        let err = scaffold(Some("x"), &dir).expect_err("the final pycc.toml write must fail");
        let message = err.to_string();
        assert!(!message.is_empty());
        // Positive proof that main.py genuinely existed and was cleanly
        // removed, not that it never existed: `roll_back_after_toml_failure`
        // only appends "could not remove"/"manually" when its own
        // `remove_file` call fails, which it would if main.py were absent
        // (a `NotFound` error). Its absence here proves the "src` steps
        // succeeded, then rolled back" path ran, not a regression that
        // skipped or reordered those steps before reaching this branch.
        assert!(!message.contains("could not remove"));
        assert!(!message.contains("remove it manually"));
        // The invocation-owned main.py is rolled back...
        assert!(!dir.join("src").join("main.py").exists());
        // ...the src/ directory this same invocation created is now empty,
        // so it is rolled back too...
        assert!(!dir.join("src").exists());
        // ...and pycc.toml still does not resolve to any file.
        assert!(!dir.join("pycc.toml").try_exists().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn scaffold_succeeds_on_retry_after_the_underlying_cause_is_removed() {
        // #256 item 2: the whole point of rolling back is that a retry
        // after fixing the cause is not blocked by scaffold's own residue.
        // A dangling `pycc.toml` symlink (not a pre-flight-refused conflict
        // like a plain existing directory) is deliberate here: it passes
        // the follow-symlink pre-check, so the first attempt genuinely
        // reaches and exercises the rollback path -- a directory conflict
        // would instead be refused at the pre-check, before `src`/`main.py`
        // are ever created, and would not discriminate this fix at all.
        let dir = ScratchDir::new("retry_ok").expect("failed to create scratch dir");
        std::os::unix::fs::symlink(
            dir.join("missing_dir").join("pycc.toml"),
            dir.join("pycc.toml"),
        )
        .unwrap();

        scaffold(Some("x"), &dir).expect_err("first attempt must fail on the dangling symlink");
        assert!(
            !dir.join("src").join("main.py").exists(),
            "the rollback must have already run before the retry"
        );
        std::fs::remove_file(dir.join("pycc.toml")).unwrap(); // the fix

        scaffold(Some("x"), &dir).expect("retry after removing the cause must succeed");
        assert!(dir.join("pycc.toml").is_file());
        assert!(dir.join("src").join("main.py").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn scaffold_preserves_a_preexisting_populated_src_while_rolling_back_main_py() {
        // #256 item 3: a pre-existing `src/` (with its own user file, no
        // main.py -- otherwise the pre-check phase would already refuse)
        // must retain every user file; only the invocation-owned main.py
        // is rolled back, and src/ itself -- not created by this
        // invocation -- is never removed.
        let dir = ScratchDir::new("preserve_src").expect("failed to create scratch dir");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src").join("lib.py"), "user lib").unwrap();
        std::os::unix::fs::symlink(
            dir.join("missing_dir").join("pycc.toml"),
            dir.join("pycc.toml"),
        )
        .unwrap();

        scaffold(Some("x"), &dir).expect_err("the final pycc.toml write must fail");
        assert_eq!(
            std::fs::read_to_string(dir.join("src").join("lib.py")).unwrap(),
            "user lib"
        );
        assert!(!dir.join("src").join("main.py").exists());
        assert!(dir.join("src").is_dir(), "pre-existing src/ must survive");
    }

    #[test]
    fn roll_back_after_toml_failure_removes_the_owned_main_py() {
        let dir = ScratchDir::new("rollback_helper").expect("failed to create scratch dir");
        let main_py = dir.join("main.py");
        std::fs::write(&main_py, "content").unwrap();
        let toml_error = std::io::Error::other("original toml failure");

        let returned = roll_back_after_toml_failure(toml_error, &main_py, &dir, false);
        assert_eq!(returned.to_string(), "original toml failure");
        assert!(!main_py.exists());
    }

    #[test]
    fn roll_back_after_toml_failure_removes_an_invocation_created_empty_src_dir() {
        let parent = ScratchDir::new("rollback_srcdir").expect("failed to create scratch dir");
        let src_dir = parent.join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let main_py = src_dir.join("main.py");
        std::fs::write(&main_py, "content").unwrap();
        let toml_error = std::io::Error::other("original toml failure");

        roll_back_after_toml_failure(toml_error, &main_py, &src_dir, true);
        assert!(
            !src_dir.exists(),
            "an invocation-created, now-empty src/ must be removed"
        );
    }

    #[test]
    fn roll_back_after_toml_failure_never_removes_a_preexisting_src_dir() {
        let parent = ScratchDir::new("rollback_preexisting").expect("failed to create scratch dir");
        let src_dir = parent.join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(src_dir.join("sibling.py"), "user file").unwrap();
        let main_py = src_dir.join("main.py");
        std::fs::write(&main_py, "content").unwrap();
        let toml_error = std::io::Error::other("original toml failure");

        // src_dir_created = false: this invocation did not create src/.
        roll_back_after_toml_failure(toml_error, &main_py, &src_dir, false);
        assert!(
            src_dir.is_dir(),
            "a pre-existing src/ must never be removed"
        );
        assert_eq!(
            std::fs::read_to_string(src_dir.join("sibling.py")).unwrap(),
            "user file"
        );
    }

    #[cfg(unix)]
    #[test]
    fn roll_back_after_toml_failure_reports_a_rollback_failure_honestly() {
        // #256 item 5: unlike write_new_in's own best-effort cleanup of the
        // file whose *own* write just failed, a failure to remove an
        // *earlier sibling's* successful write must be reported, not
        // silently swallowed -- otherwise this is the exact same residue
        // the issue is about, just relocated to a rarer trigger.
        // (Root ignores 0o555, so this would spuriously succeed under a
        // root test runner; hosted CI and the coverage sandbox's `nobody`
        // user are both non-root.)
        use std::os::unix::fs::PermissionsExt;
        let dir = ScratchDir::new("rollback_fail").expect("failed to create scratch dir");
        let main_py = dir.join("main.py");
        std::fs::write(&main_py, "content").unwrap();
        // The file already exists; making its parent read-only now blocks
        // only the later `unlink`, not the write that already happened.
        std::fs::set_permissions(&*dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let toml_error = std::io::Error::other("original toml failure");

        let returned = roll_back_after_toml_failure(toml_error, &main_py, &dir, false);
        // Restore write permission before any assertion can panic, so the
        // scratch dir's Drop can remove it even when this test fails.
        std::fs::set_permissions(&*dir, std::fs::Permissions::from_mode(0o755)).ok();

        let message = returned.to_string();
        assert!(message.contains("original toml failure"));
        assert!(message.contains("could not remove"));
        assert!(message.contains("remove it manually"));
    }
}
