//! The compile unit of a non-strict `--ext` module that selects functions
//! (docs/design/compile-little.md, section 2.4, `crate::select`).
//!
//! The module stays CPython bytecode: its whole source is one chunk
//! (`render::CodeChunk`), compiled at build time, which the extension's
//! body runs in the module's namespace as CPython's own import would. Only
//! the selected functions are compiled, each as the lift of its `def`
//! (`lift`: every parameter and the return an object, every module global
//! read from the namespace, `super()` given the class). Once the chunk has
//! run, `__pycc_patch__` (`src/ext/module/patch.c`) points each function
//! CPython made of a selected `def` at its compiled body; the function
//! object, its `__code__`, defaults and decorators stay CPython's.
//!
//! The unit the frontend compiles is the selected lifts, each at its own
//! lines, then the trailer (`trailer`). A lift the compiler refuses is
//! dropped and the unit compiled again: a selected function compiles whole
//! or stays bytecode. A builtin call pycc does not compile for its
//! arguments goes through CPython's `builtins` instead (`builtins.rs`), and
//! an annotation it cannot lower under `from __future__ import annotations`
//! is erased (`annotations`), as in the fallback loop.

use super::render::{NOWHERE, Out};
use super::*;

/// The hook the trailer calls once the module's code has run (`patch.c`).
const PATCH_HOOK: &str = "__pycc_patch__";

/// The names every module binds before its code runs.
const MODULE_DUNDERS: &[&str] = &[
    "__name__",
    "__doc__",
    "__file__",
    "__spec__",
    "__loader__",
    "__package__",
    "__path__",
    "__builtins__",
    "__cached__",
];

/// `with_fallback` for a module whose functions `selected` names (qualified
/// name, first line): compiles the selected ones that compile, the module
/// itself as bytecode.
pub(super) fn with_selection<T, E>(
    path: &Path,
    display: &str,
    publishes: Publishes,
    selected: &BTreeSet<crate::select::Key>,
    attempt: impl Fn() -> Result<T, E>,
    split: impl Fn(&E) -> Option<Vec<Failure>>,
) -> Result<T, E> {
    set_override(None);
    pycc_hir::set_external_attribute_reads(BTreeSet::new());
    let Some(mut plan) =
        driver::read_entry(path).and_then(|source| Plan::new(&source, display, publishes))
    else {
        // A syntax error: the frontend reports it.
        return attempt();
    };
    let mut units: Vec<usize> = Vec::new();
    let mut dropped: Vec<(String, String)> = Vec::new();
    for (at, lift) in plan.lifts.iter().enumerate() {
        let unit = plan.top + at;
        let key = (lift.qualname.clone(), line_of(&plan.source, lift.range.start) as u32);
        if !selected.contains(&key) {
            continue;
        }
        if plan.by_string.contains(&unit) {
            dropped.push((
                lift.qualname.clone(),
                "it reaches the module's globals by a string".to_string(),
            ));
            continue;
        }
        units.push(unit);
    }
    // A selected function pycc does not lift (an `async def`, a body that is
    // only a docstring, ...) stays bytecode.
    for (qualname, line) in selected {
        let lifted = plan.lifts.iter().any(|lift| {
            &lift.qualname == qualname && line_of(&plan.source, lift.range.start) as u32 == *line
        });
        if !lifted {
            dropped.push((qualname.clone(), "pycc does not compile a function of its kind".to_string()));
        }
    }
    let value = loop {
        let rendered = plan.render_selected(&units);
        if std::env::var_os("PYCC_FALLBACK_DUMP").is_some() {
            eprintln!("--- selected functions ---\n{}", rendered.text);
        }
        install(&rendered);
        let failure = match attempt() {
            Ok(value) => break Ok(value),
            Err(failure) => failure,
        };
        let Some(failures) = split(&failure) else {
            // Not a failure of the compile unit (an input error).
            set_override(None);
            break Err(failure);
        };
        let mut progress = false;
        let mut refused: BTreeMap<usize, String> = BTreeMap::new();
        for failure in &failures {
            if plan.rewrite_builtin_use(Some(&rendered), failure)
                || plan.erase_annotation(Some(&rendered), failure)
            {
                progress = true;
                continue;
            }
            let at = failure
                .diagnostic
                .span
                .filter(|s| (s.start, s.end) != (0, 0))
                .map(|s| s.start)
                .or(failure.origin);
            let owner = at.and_then(|at| {
                units
                    .iter()
                    .copied()
                    .find(|unit| rendered.ranges[*unit].contains(&at))
            });
            match owner {
                Some(unit) => {
                    refused.entry(unit).or_insert_with(|| describe(&failure.diagnostic));
                }
                // A failure no selected function owns: none of them is
                // compiled.
                None => {
                    for unit in &units {
                        refused
                            .entry(*unit)
                            .or_insert_with(|| describe(&failure.diagnostic));
                    }
                }
            }
        }
        if refused.is_empty() && !progress {
            set_override(None);
            break Err(failure);
        }
        units.retain(|unit| !refused.contains_key(unit));
        for (unit, reason) in refused {
            let lift = plan.lift(unit).expect("a lift");
            dropped.push((lift.qualname.clone(), reason));
        }
    };
    if reporting() {
        // The name `--compile` and the profile take.
        let module = crate::select::import_name(path);
        for unit in &units {
            let lift = plan.lift(*unit).expect("a lift");
            eprintln!("compiled: {module}:{}", lift.qualname);
        }
        for (qualname, reason) in &dropped {
            eprintln!("not compiled: {module}:{qualname}: {reason}");
        }
    }
    value
}

impl Plan {
    /// The module-level names a selected function reads from the
    /// namespace: every name the module binds or declares `global`, every
    /// name a module has before its code runs, and every name that is not
    /// a builtin. With a wildcard import any name may be the module's.
    fn selected_globals(&self, units: &[usize]) -> BTreeSet<String> {
        let mut bound: BTreeSet<String> = MODULE_DUNDERS.iter().map(|n| n.to_string()).collect();
        for stmt in &self.stmts[..self.top] {
            bound.extend(stmt.bound.iter().cloned());
            bound.extend(stmt.declares.iter().cloned());
        }
        let wildcard = bound.contains("*");
        let mut out = BTreeSet::new();
        for unit in units {
            for name in &self.stmts[*unit].mentions {
                if wildcard || bound.contains(name) || !is_builtin(name) {
                    out.insert(name.clone());
                }
            }
            out.extend(self.stmts[*unit].declares.iter().cloned());
        }
        out
    }

    /// The compile unit of [`with_selection`]: the lifts `units`, each at
    /// its own lines, then the trailer.
    pub(super) fn render_selected(&self, units: &[usize]) -> Rendered {
        let globals = self.selected_globals(units);
        let native = BTreeSet::new();
        let src = &self.source;
        let mut out = Out {
            text: String::with_capacity(src.len()),
            ranges: vec![NOWHERE; self.stmts.len()],
            copied: Vec::new(),
            mirror_spans: Vec::new(),
            edited: Vec::new(),
            chunks: Vec::new(),
        };
        let mut lines = 0usize;
        let mut placed: Vec<usize> = Vec::new();
        let mut order: Vec<usize> = units.to_vec();
        order.sort_by_key(|unit| self.lift(*unit).map_or(0, |lift| lift.keyword));
        for unit in order {
            let lift = self.lift(unit).expect("a lift");
            let line = src[..lift.keyword as usize].matches('\n').count();
            if line < lines {
                // Another lift ends on this one's first line.
                continue;
            }
            out.text.push_str(&"\n".repeat(line - lines));
            let at = out.text.len() as u32;
            self.emit_compiled(&mut out, unit, &globals, &native);
            out.ranges[unit] = at..out.text.len() as u32;
            out.text.push('\n');
            lines = out.text.matches('\n').count();
            placed.push(unit);
        }
        let start = out.text.len() as u32;
        let module = self.exec_text_as(&mut out.chunks, src, &self.display, false);
        let file = self.exec_text_as(&mut out.chunks, FILE_FIXUP, "<pycc>", false);
        out.text.push_str(&format!(
            "import sys as {SYS}; import builtins as {BUILTINS}; {NS} = {SYS}.modules[__name__].__dict__\n\
             {file}\n{module}\n"
        ));
        let patch = self.patch_call(&placed);
        if !patch.is_empty() {
            out.text.push_str(&patch);
            out.text.push('\n');
        }
        Rendered {
            text: out.text.clone(),
            ranges: out.ranges,
            preamble: start..out.text.len() as u32,
            copied: out.copied,
            mirror_spans: out.mirror_spans,
            edited: out.edited,
            twinned: BTreeSet::new(),
            attached: Vec::new(),
            chunks: out.chunks,
        }
    }

    /// `__pycc_patch__(...)` for the compiled lifts `units`: each function's
    /// name (after its owner path when that changes, as
    /// `Plan::forward_call`'s), its lift's name and its first line, times
    /// two, plus one when the lift takes the class (`__class__`).
    fn patch_call(&self, units: &[usize]) -> String {
        if units.is_empty() {
            return String::new();
        }
        let mut args: Vec<String> = Vec::new();
        let mut current = "";
        let mut sorted: Vec<&lift::Lift> = units.iter().filter_map(|unit| self.lift(*unit)).collect();
        sorted.sort_by(|a, b| a.qualname.rsplit_once('.').map(|p| p.0).cmp(&b.qualname.rsplit_once('.').map(|p| p.0)));
        for lift in sorted {
            let (owner, member) = lift.qualname.rsplit_once('.').unwrap_or(("", &lift.qualname));
            if owner != current || args.is_empty() {
                let parts: Vec<String> = owner
                    .split('.')
                    .filter(|part| !part.is_empty())
                    .map(|part| format!("\"{part}\""))
                    .collect();
                args.push(match parts.len() {
                    0 => "()".to_string(),
                    1 => format!("({},)", parts[0]),
                    _ => format!("({})", parts.join(", ")),
                });
                current = owner;
            }
            let line = line_of(&self.source, lift.range.start) as u32;
            args.push(format!(
                "\"{member}\", \"{}\", {}",
                lift.name,
                line * 2 + u32::from(lift.uses_class)
            ));
        }
        format!("{NS}[\"{PATCH_HOOK}\"]({})", args.join(", "))
    }
}

/// Run before the module's code: a module served from an extension names
/// the `.py` file beside it, as CPython's own would, when that file exists
/// (its `__file__` and `__spec__.origin`).
const FILE_FIXUP: &str = "\
import os as __pycc_os
__pycc_f = globals().get('__file__')
if __pycc_f and not __pycc_f.endswith('.py'):
    __pycc_d, __pycc_b = __pycc_os.path.split(__pycc_f)
    __pycc_p = __pycc_os.path.join(__pycc_d, ('__init__' if __pycc_b.startswith('__init__.') else __name__.rpartition('.')[2]) + '.py')
    if __pycc_os.path.isfile(__pycc_p):
        __file__ = __pycc_p
        if globals().get('__spec__') is not None:
            __spec__.origin = __pycc_p
    del __pycc_d, __pycc_b, __pycc_p
del __pycc_os, __pycc_f
";

/// Whether `name` is one of CPython's builtins.
fn is_builtin(name: &str) -> bool {
    BUILTIN_NAMES.binary_search(&name).is_ok()
}

/// `dir(builtins)` of CPython 3.15 without the dunders, sorted.
const BUILTIN_NAMES: &[&str] = &[
    "ArithmeticError", "AssertionError", "AttributeError", "BaseException", "BaseExceptionGroup",
    "BlockingIOError", "BrokenPipeError", "BufferError", "BytesWarning", "ChildProcessError",
    "ConnectionAbortedError", "ConnectionError", "ConnectionRefusedError", "ConnectionResetError",
    "DeprecationWarning", "EOFError", "Ellipsis", "EncodingWarning", "EnvironmentError",
    "Exception", "ExceptionGroup", "False", "FileExistsError", "FileNotFoundError",
    "FloatingPointError", "FutureWarning", "GeneratorExit", "IOError", "ImportError",
    "ImportWarning", "IndentationError", "IndexError", "InterruptedError", "IsADirectoryError",
    "KeyError", "KeyboardInterrupt", "LookupError", "MemoryError", "ModuleNotFoundError",
    "NameError", "None", "NotADirectoryError", "NotImplemented", "NotImplementedError",
    "OSError", "OverflowError", "PendingDeprecationWarning", "PermissionError",
    "ProcessLookupError", "PythonFinalizationError", "RecursionError", "ReferenceError",
    "ResourceWarning", "RuntimeError", "RuntimeWarning", "StopAsyncIteration", "StopIteration",
    "SyntaxError", "SyntaxWarning", "SystemError", "SystemExit", "TabError", "TimeoutError",
    "True", "TypeError", "UnboundLocalError", "UnicodeDecodeError", "UnicodeEncodeError",
    "UnicodeError", "UnicodeTranslateError", "UnicodeWarning", "UserWarning", "ValueError",
    "Warning", "ZeroDivisionError", "abs", "aiter", "all", "anext", "any", "ascii", "bin",
    "bool", "breakpoint", "bytearray", "bytes", "callable", "chr", "classmethod", "compile",
    "complex", "copyright", "credits", "delattr", "dict", "dir", "divmod", "enumerate", "eval",
    "exec", "exit", "filter", "float", "format", "frozendict", "frozenset", "getattr", "globals",
    "hasattr", "hash", "help", "hex", "id", "input", "int", "isinstance", "issubclass", "iter",
    "len", "license", "list", "locals", "map", "max", "memoryview", "min", "next", "object",
    "oct", "open", "ord", "pow", "print", "property", "quit", "range", "repr", "reversed",
    "round", "set", "setattr", "slice", "sorted", "staticmethod", "str", "sum", "super", "tuple",
    "type", "vars", "zip",
];

#[cfg(test)]
mod tests {
    use super::BUILTIN_NAMES;

    #[test]
    fn the_builtin_names_are_sorted() {
        assert!(BUILTIN_NAMES.windows(2).all(|w| w[0] < w[1]));
    }
}
