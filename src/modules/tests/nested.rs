//! The loader's answer to a `from ... import` nested in a module-level
//! `if`/`try` body (#1383): [`Resolution::Foreign`] when the top-level form
//! would be foreign, [`Resolution::Unanswered`] for everything else, and
//! never a load or a cycle check.

use super::*;

const BLOCK_TEXT: &str = "an `import` inside a block body";

/// The foreign `(module_path, name)` from-import bindings of the loaded
/// entry, nested or not.
fn foreign_from_names(program: &LoadedProgram) -> Vec<(String, String)> {
    entry_foreign_from_imports(program)
        .into_iter()
        .map(|(module, name, _)| (module, name))
        .collect()
}

#[test]
fn a_nested_from_import_of_a_foreign_module_is_answered_foreign() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    let entry = write(
        &scratch,
        "main.py",
        "c = True\nif c:\n    from nowhere import thing\ntry:\n    \
         from no.where import other\nexcept ImportError:\n    pass\n",
    );
    let program =
        load(&entry, None).unwrap_or_else(|failure| panic!("must load: {}", describe(&failure)));
    assert_eq!(program.modules.len(), 1, "nothing but the entry loads");
    assert_eq!(
        foreign_from_names(&program),
        vec![
            ("nowhere".to_string(), "thing".to_string()),
            ("no.where".to_string(), "other".to_string()),
        ]
    );
}

/// A present project module is neither loaded nor cycle-checked: a helper
/// that does not even parse, and one that imports the entry back, both
/// leave the entry's own block-body `C0001` as the only diagnostic.
#[test]
fn a_nested_from_import_of_a_project_module_is_left_unanswered() {
    for helper in ["def broken(:\n", "from main import c\n"] {
        let scratch = ScratchDir::new("modules_tests").expect("scratch");
        write(&scratch, "helper.py", helper);
        let entry = write(
            &scratch,
            "main.py",
            "c = True\nif c:\n    from helper import g\n",
        );
        let (path, code, message) = first_diagnostic(&entry);
        assert!(path.ends_with("main.py"), "{path}");
        assert_eq!(code, "C0001", "{helper:?}");
        assert!(message.contains(BLOCK_TEXT), "{message}");
    }
}

/// A dotted name under a project root, a relative miss, a relative hit
/// and a namespace package all keep the block-body `C0001` rather than the
/// diagnostic the top-level form reports.
#[test]
fn every_non_foreign_nested_from_import_keeps_the_block_diagnostic() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "pkg/__init__.py", "");
    write(&scratch, "pkg/sib.py", "x = 1\n");
    std::fs::create_dir_all(scratch.join("pkg/ns")).expect("namespace dir");
    write(&scratch, "helper.py", "x = 1\n");
    for statement in [
        "from helper.sub import y",
        "from .nowhere import y",
        "from .sib import x",
        "from .ns import y",
    ] {
        let entry = write(
            &scratch,
            "pkg/main.py",
            &format!("c = True\nif c:\n    {statement}\n"),
        );
        let (_, code, message) = first_diagnostic(&entry);
        assert_eq!(code, "C0001", "{statement}");
        assert!(message.contains(BLOCK_TEXT), "{statement}: {message}");
    }
}

/// Under `pycc build --ext --foreign-relative-imports` (#1366) the entry's
/// nested relative from-import is foreign like its top-level one.
#[test]
fn a_nested_relative_from_import_is_foreign_under_foreign_from_entry() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(&scratch, "sib.py", "x = 1\n");
    let entry = write(
        &scratch,
        "main.py",
        "try:\n    from .sib import x\nexcept ImportError:\n    pass\n",
    );
    let program =
        load_foreign(&entry).unwrap_or_else(|failure| panic!("must load: {}", describe(&failure)));
    assert_eq!(program.modules.len(), 1, "no sibling file is loaded");
    assert_eq!(
        entry_foreign_from_imports(&program),
        vec![("sib".to_string(), "x".to_string(), 1)]
    );
}
