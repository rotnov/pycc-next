//! Circular imports (`super::super::cycle`): between dependencies they
//! load; through the entry module they are `E0108`.

use super::*;

#[test]
fn an_import_cycle_through_the_entry_is_reported_with_its_chain() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(
        &scratch,
        "a.py",
        "from main import first\n\n\ndef second(x: int) -> int:\n    return first(x)\n",
    );
    let entry = write(
        &scratch,
        "main.py",
        "from a import second\n\n\ndef first(x: int) -> int:\n    return x\n",
    );
    let (path, code, message) = first_diagnostic(&entry);
    assert!(
        path.ends_with("a.py"),
        "reported in the importing file: {path}"
    );
    assert_eq!(code, "E0108");
    assert!(
        message.starts_with("import cycle through the entry module: ")
            && message.matches("main.py").count() == 2,
        "unexpected message: {message}"
    );
}

#[test]
fn a_cycle_between_dependencies_loads_both() {
    let scratch = ScratchDir::new("modules_tests").expect("scratch");
    write(
        &scratch,
        "a.py",
        "from b import second\n\n\ndef first(x: int) -> int:\n    return second(x)\n",
    );
    write(
        &scratch,
        "b.py",
        "import a\n\n\ndef second(x: int) -> int:\n    return x if x < 1 else a.first(x - 1)\n",
    );
    let entry = write(
        &scratch,
        "main.py",
        "from a import first\n\nprint(first(3))\n",
    );
    let Ok(program) = load(&entry, None) else {
        panic!("a cycle between dependencies loads");
    };
    assert_eq!(program.modules.len(), 3);
}
