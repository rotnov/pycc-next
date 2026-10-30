//! v0.10 accept 5: the names CPython's builtin exception classes bind at
//! class level, which an exception class's native layout must leave alone
//! (`exc_layout`): `self.errno = code` in an `OSError` subclass stores
//! through `OSError.errno`'s descriptor, which `str(e)` reads, so it stays a
//! store on the CPython object.

/// The non-dunder names `BaseException` binds (`vars(BaseException)`).
const BASE: [&str; 3] = ["add_note", "args", "with_traceback"];

/// The non-dunder names bound in the class dictionaries along builtin
/// exception class `class`'s MRO, from CPython 3.13 (`winerror` only on
/// Windows). Dunder names are never layout attributes, so they are left
/// out. Checked against the running CPython by this module's test.
pub(super) fn builtin_exception_names(class: &str) -> impl Iterator<Item = &'static str> {
    let own: &'static [&'static str] = match class {
        "OSError"
        | "BlockingIOError"
        | "ChildProcessError"
        | "ConnectionError"
        | "FileExistsError"
        | "FileNotFoundError"
        | "InterruptedError"
        | "IsADirectoryError"
        | "NotADirectoryError"
        | "PermissionError"
        | "ProcessLookupError"
        | "TimeoutError"
        | "BrokenPipeError"
        | "ConnectionAbortedError"
        | "ConnectionRefusedError"
        | "ConnectionResetError" => &[
            "characters_written",
            "errno",
            "filename",
            "filename2",
            "strerror",
            "winerror",
        ],
        "BaseExceptionGroup" | "ExceptionGroup" => {
            &["derive", "exceptions", "message", "split", "subgroup"]
        }
        "ImportError" | "ModuleNotFoundError" => &["msg", "name", "name_from", "path"],
        "AttributeError" => &["name", "obj"],
        "NameError" => &["name"],
        "StopIteration" => &["value"],
        _ => &[],
    };
    BASE.into_iter().chain(own.iter().copied())
}

/// Whether builtin exception class `class`, or a class along its MRO short
/// of `BaseException`, defines its own `__reduce__`, `__reduce_ex__`,
/// `__getstate__` or `__setstate__` (CPython 3.13): the template's
/// `__reduce__` only ever extends `BaseException`'s, so a layout over one
/// is not made. Checked against the running CPython by this module's test.
pub(super) fn builtin_reduces_itself(class: &str) -> bool {
    matches!(
        class,
        "OSError"
            | "BlockingIOError"
            | "ChildProcessError"
            | "ConnectionError"
            | "FileExistsError"
            | "FileNotFoundError"
            | "InterruptedError"
            | "IsADirectoryError"
            | "NotADirectoryError"
            | "PermissionError"
            | "ProcessLookupError"
            | "TimeoutError"
            | "BrokenPipeError"
            | "ConnectionAbortedError"
            | "ConnectionRefusedError"
            | "ConnectionResetError"
            | "ImportError"
            | "ModuleNotFoundError"
            | "AttributeError"
    )
}

/// The public classes of CPython 3.13's `builtins`: a base named so, with
/// no binding of the name in the module, is that class, which no layout
/// class is. Checked against the running CPython by this module's test.
pub(super) const BUILTIN_CLASSES: [&str; 96] = [
    "ArithmeticError",
    "AssertionError",
    "AttributeError",
    "BaseException",
    "BaseExceptionGroup",
    "BlockingIOError",
    "BrokenPipeError",
    "BufferError",
    "BytesWarning",
    "ChildProcessError",
    "ConnectionAbortedError",
    "ConnectionError",
    "ConnectionRefusedError",
    "ConnectionResetError",
    "DeprecationWarning",
    "EOFError",
    "EncodingWarning",
    "EnvironmentError",
    "Exception",
    "ExceptionGroup",
    "FileExistsError",
    "FileNotFoundError",
    "FloatingPointError",
    "FutureWarning",
    "GeneratorExit",
    "IOError",
    "ImportError",
    "ImportWarning",
    "IndentationError",
    "IndexError",
    "InterruptedError",
    "IsADirectoryError",
    "KeyError",
    "KeyboardInterrupt",
    "LookupError",
    "MemoryError",
    "ModuleNotFoundError",
    "NameError",
    "NotADirectoryError",
    "NotImplementedError",
    "OSError",
    "OverflowError",
    "PendingDeprecationWarning",
    "PermissionError",
    "ProcessLookupError",
    "PythonFinalizationError",
    "RecursionError",
    "ReferenceError",
    "ResourceWarning",
    "RuntimeError",
    "RuntimeWarning",
    "StopAsyncIteration",
    "StopIteration",
    "SyntaxError",
    "SyntaxWarning",
    "SystemError",
    "SystemExit",
    "TabError",
    "TimeoutError",
    "TypeError",
    "UnboundLocalError",
    "UnicodeDecodeError",
    "UnicodeEncodeError",
    "UnicodeError",
    "UnicodeTranslateError",
    "UnicodeWarning",
    "UserWarning",
    "ValueError",
    "Warning",
    "ZeroDivisionError",
    "bool",
    "bytearray",
    "bytes",
    "classmethod",
    "complex",
    "dict",
    "enumerate",
    "filter",
    "float",
    "frozenset",
    "int",
    "list",
    "map",
    "memoryview",
    "object",
    "property",
    "range",
    "reversed",
    "set",
    "slice",
    "staticmethod",
    "str",
    "super",
    "tuple",
    "type",
    "zip",
];

#[cfg(test)]
mod tests {
    use super::{BUILTIN_CLASSES, builtin_exception_names, builtin_reduces_itself};

    /// [`BUILTIN_CLASSES`] is what the running CPython's `builtins` holds.
    #[test]
    #[ignore = "requires a CPython 3.13 on PATH (CI runs it)"]
    fn the_builtin_classes_are_cpython_s() {
        let output = std::process::Command::new(
            std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()),
        )
        .arg("-c")
        .arg(
            "import builtins\n\
             print(*sorted(n for n in dir(builtins)\n\
             \x20   if isinstance(getattr(builtins, n), type) and not n.startswith('_')))",
        )
        .output()
        .expect("python3 runs");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).expect("utf-8");
        let cpython: Vec<&str> = text.split_whitespace().collect();
        assert_eq!(BUILTIN_CLASSES.to_vec(), cpython);
    }
    use std::collections::BTreeSet;

    /// The table matches what the running CPython binds along each builtin
    /// exception class's MRO (`winerror` aside, which only Windows has).
    #[test]
    #[ignore = "requires a CPython 3.13 on PATH (CI runs it)"]
    fn the_table_matches_cpython() {
        let script = "import builtins, sys\n\
             hooks = ('__reduce__', '__reduce_ex__', '__getstate__', '__setstate__')\n\
             for name in sys.argv[1:]:\n\
             \x20   mro = getattr(builtins, name).__mro__\n\
             \x20   names = sorted({n for c in mro[:-1] for n in vars(c)\n\
             \x20                   if not (n.startswith('__') and n.endswith('__'))})\n\
             \x20   own = any(h in vars(c) for c in mro[:-2] for h in hooks)\n\
             \x20   print(name, own, *names)\n";
        let output = std::process::Command::new(
            std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()),
        )
        .arg("-c")
        .arg(script)
        .args(pycc_hir::BUILTIN_EXCEPTION_CLASSES)
        .output()
        .expect("python3 runs");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).expect("utf-8");
        let mut seen = 0;
        for line in text.lines() {
            let mut words = line.split_whitespace();
            let class = words.next().expect("a class");
            let own = words.next().expect("whether it reduces itself") == "True";
            assert_eq!(builtin_reduces_itself(class), own, "{class}");
            let cpython: BTreeSet<&str> = words.filter(|n| *n != "winerror").collect();
            let table: BTreeSet<&str> = builtin_exception_names(class)
                .filter(|n| *n != "winerror")
                .collect();
            assert_eq!(table, cpython, "{class}");
            seen += 1;
        }
        assert_eq!(seen, pycc_hir::BUILTIN_EXCEPTION_CLASSES.len());
    }
}
