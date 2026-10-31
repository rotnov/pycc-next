//! Shipping an `--ext` module's CPython source pre-compiled (v0.8).
//!
//! The fallback rewrite (`crate::fallback`) runs each piece of source pycc
//! does not compile -- a class statement CPython runs, the methods that
//! fall back, the copies Python callers read -- with `exec` while the
//! module body runs. Compiling that source with `compile()` on every
//! import cost pydantic's import more than all of CPython's own work, which
//! a `.pyc` cache saves CPython. Instead, the build compiles every chunk
//! (`crate::fallback::CodeChunk`) once with the target interpreter and
//! embeds the module's code objects as one `marshal`led table, with the
//! interpreter's bytecode magic number (one table, so `marshal` shares the
//! names and constants the chunks repeat and each is read and interned once);
//! the rewrite reads each code object through the module's `__pycc_code__`
//! hook (`PRECOMPILED_C`), present in the namespace only while the body runs.
//! The hook unmarshals the table on its first call, when the running
//! interpreter has the same magic number and runs unoptimized
//! (`-O` strips `assert`s at compile time), and otherwise compiles the
//! embedded source exactly as the rewrite did before: same file name, same
//! `from __future__ import annotations` flag, so `co_filename`, line
//! numbers and tracebacks are the same either way.
//!
//! The compile happens in a fresh `-I` interpreter process with
//! `dont_inherit=True` and `optimize=0`. A build that cannot run the
//! interpreter (`PYCC_PYTHON_INCLUDE` without one) embeds the source only.

use crate::fallback::CodeChunk;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::io::Write as _;
use std::process::Stdio;

/// What the target interpreter compiled: its magic number, the
/// `marshal`led table of every chunk's entry -- its code object and, when
/// its code holds `frozenset` constants, their elements in source order (a
/// tuple of tuples, for [`ORDERS_FIX`]), or `None` for a chunk it refused to
/// compile, which then fails at import exactly as `compile()` did -- followed
/// by [`ORDERS_FIX`]'s code object, and which chunks have an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Precompiled {
    pub(crate) magic: u32,
    pub(crate) table: Vec<u8>,
    pub(crate) present: Vec<bool>,
}

/// The script compiling the chunks of its stdin to marshalled code.
///
/// Its input (pycc's own) and its answer are `marshal` data and raw bytes
/// rather than JSON: the script runs for every module an `--ext` build
/// compiles (each member of a package), and importing `json`, and `ast`
/// up front where only a chunk holding a set display needs it, doubled the
/// interpreter's start-up (about 21 ms instead of 10).
/// The input is `marshal`'s form of `(((source, filename, future), ...),
/// fix)` ([`marshal_input`]); the answer is the bytecode magic number
/// (`importlib.util.MAGIC_NUMBER`, four bytes), one byte per chunk (1 when
/// it compiled) and the `marshal`led table ([`parse`]).
///
/// `marshal` rebuilds a `frozenset` in its iteration order, so a set display
/// the compiler folds (`{1, 8, 0}`) would iterate in another order than a
/// fresh compile's, as from a `.pyc`. For a chunk holding such constants the
/// script also writes each one's elements in source order -- the order the
/// compiler built it from, found as the one set display of the chunk with
/// those elements (`ast.literal_eval` of each element) -- in the order a
/// depth-first walk of the constants meets them; the import rebuilds each
/// from them (`ORDERS_FIX`). A chunk where some constant has no such
/// display, or several in different orders, is left to `compile()` at
/// import, as before.
const COMPILE_SCRIPT: &str = "import sys,marshal,importlib.util,__future__\n\
    def walk(co,acc):\n\
    \x20 for k in co.co_consts:\n\
    \x20   if type(k) is frozenset and len(k)>1: acc.append(k)\n\
    \x20   elif hasattr(k,'co_consts'): walk(k,acc)\n\
    def displays(src):\n\
    \x20 import ast\n\
    \x20 out=[]\n\
    \x20 for n in ast.walk(ast.parse(src)):\n\
    \x20   if isinstance(n,ast.Set):\n\
    \x20     try: out.append(tuple(ast.literal_eval(e) for e in n.elts))\n\
    \x20     except Exception: pass\n\
    \x20 return out\n\
    def orders(src,co):\n\
    \x20 sets=[]\n\
    \x20 walk(co,sets)\n\
    \x20 if not sets: return None\n\
    \x20 shown=displays(src)\n\
    \x20 out=[]\n\
    \x20 for k in sets:\n\
    \x20   found={t for t in shown if frozenset(t)==k}\n\
    \x20   if len(found)!=1: raise ValueError\n\
    \x20   out.append(found.pop())\n\
    \x20 return tuple(out)\n\
    chunks,fix=marshal.loads(sys.stdin.buffer.read())\n\
    table=[]\n\
    for source,filename,future in chunks:\n\
    \x20 try:\n\
    \x20   f=__future__.annotations.compiler_flag if future else 0\n\
    \x20   co=compile(source,filename,'exec',f,True,0)\n\
    \x20   table.append((co,orders(source,co)))\n\
    \x20 except Exception:\n\
    \x20   table.append(None)\n\
    present=bytes(t is not None for t in table)\n\
    table.append(compile(fix,'<pycc precompiled>','exec',0,True,0))\n\
    sys.stdout.buffer.write(importlib.util.MAGIC_NUMBER+present+marshal.dumps(tuple(table)))\n";

/// The `fix(code, orders)` an import runs on a chunk holding `frozenset`
/// constants: rebuilds each from its elements in source order
/// (`COMPILE_SCRIPT`'s `orders`, met in the same depth-first walk of the
/// constants) as CPython 3.13's compiler builds one -- a frozenset of the
/// display's elements, rebuilt once more from its own iteration when the
/// code's constants are merged -- so it iterates as a fresh compile's does;
/// any mismatch raises, and the chunk is compiled instead. Compiled with the
/// chunks, its code object is the table's last entry.
const ORDERS_FIX: &str = "def fix(co, orders):
    it = iter(orders)
    def walk(c):
        consts = list(c.co_consts)
        changed = False
        for i, k in enumerate(consts):
            if type(k) is frozenset and len(k) > 1:
                s = frozenset(tuple(frozenset(next(it))))
                if s != k:
                    raise ValueError('pycc: a frozenset constant does not match')
                consts[i] = s
                changed = True
            elif type(k) is type(c):
                n = walk(k)
                if n is not k:
                    consts[i] = n
                    changed = True
        return c.replace(co_consts=tuple(consts)) if changed else c
    out = walk(co)
    if next(it, None) is not None:
        raise ValueError('pycc: frozenset orders left over')
    return out
";

/// Compiles `chunks` with `interpreter`; `None` when it cannot run or
/// answers something this build cannot read.
pub(crate) fn precompile(interpreter: &OsStr, chunks: &[CodeChunk]) -> Option<Precompiled> {
    let input = marshal_input(chunks)?;
    let mut child = super::probe_command(interpreter, COMPILE_SCRIPT)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output().ok()?;
    writer.join().ok()?.ok()?;
    if !output.status.success() {
        return None;
    }
    parse(&output.stdout, chunks.len())
}

/// The compile script's input: `marshal`'s form of `(((source, filename,
/// future), ...), ORDERS_FIX)`, built from its tuple (`(`), string (`u`,
/// UTF-8) and boolean (`T`, `F`) codes, each length a 4-byte little-endian
/// count -- a format every `marshal` version reads. `None` for a string or
/// tuple too long for one.
fn marshal_input(chunks: &[CodeChunk]) -> Option<Vec<u8>> {
    fn sized(out: &mut Vec<u8>, code: u8, len: usize) -> Option<()> {
        out.push(code);
        out.extend_from_slice(&u32::try_from(len).ok()?.to_le_bytes());
        Some(())
    }
    fn text(out: &mut Vec<u8>, text: &str) -> Option<()> {
        sized(out, b'u', text.len())?;
        out.extend_from_slice(text.as_bytes());
        Some(())
    }
    let mut out = Vec::new();
    sized(&mut out, b'(', 2)?;
    sized(&mut out, b'(', chunks.len())?;
    for chunk in chunks {
        sized(&mut out, b'(', 3)?;
        text(&mut out, &chunk.source)?;
        text(&mut out, &chunk.filename)?;
        out.push(if chunk.future_annotations { b'T' } else { b'F' });
    }
    text(&mut out, ORDERS_FIX)?;
    Some(out)
}

/// Parses the compile script's answer for `count` chunks: the magic number
/// (four bytes, little-endian), one byte (0 or 1) per chunk, the table.
fn parse(stdout: &[u8], count: usize) -> Option<Precompiled> {
    let (magic, rest) = stdout.split_first_chunk::<4>()?;
    let magic = u32::from_le_bytes(*magic);
    let (present, table) = rest.split_at_checked(count)?;
    let present = present
        .iter()
        .map(|byte| match byte {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    (magic != 0 && !table.is_empty()).then(|| Precompiled {
        magic,
        table: table.to_vec(),
        present,
    })
}

/// `bytes` as a C string literal, split over lines: printable ASCII as
/// itself, everything else (and `"`, `\`, `?`) as a three-digit octal
/// escape, which no following character can extend.
fn c_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() + bytes.len() / 2 + 8);
    out.push('"');
    let mut width = 0;
    for &byte in bytes {
        if width >= 120 {
            out.push_str("\"\n    \"");
            width = 0;
        }
        if (0x20..0x7f).contains(&byte) && !matches!(byte, b'"' | b'\\' | b'?') {
            out.push(char::from(byte));
            width += 1;
        } else {
            let _ = write!(out, "\\{byte:03o}");
            width += 4;
        }
    }
    out.push('"');
    out
}

/// The generated companion `inc` with `chunks` (and their `compiled`
/// code) embedded and the hook defined, after the module-name macros.
/// `inc` itself when there is no chunk.
pub(crate) fn install(inc: String, chunks: &[CodeChunk], compiled: Option<&Precompiled>) -> String {
    if chunks.is_empty() {
        return inc;
    }
    let mut head = String::from("#define PYCC_EXT_HAS_PRECOMPILED 1\n\n");
    let _ = writeln!(
        head,
        "/* The bytecode magic number the embedded code objects were compiled for\n \
         * (0: none embedded). */\nstatic const long pycc_ext_chunk_magic = {}L;\n",
        compiled.map_or(0, |compiled| compiled.magic)
    );
    let table = compiled.map_or(&[][..], |compiled| compiled.table.as_slice());
    let _ = writeln!(
        head,
        "/* The marshalled table of the chunks' code objects. */\n\
         static const char pycc_ext_chunk_table_data[] =\n    {};\n\
         static const Py_ssize_t pycc_ext_chunk_table_len = {};\n",
        c_bytes(table),
        table.len()
    );
    head.push_str(
        "static const struct pycc_ext_chunk {\n    const char *source;\n    \
         Py_ssize_t source_len;\n    const char *filename;\n    int future_annotations;\n    \
         int precompiled;\n} pycc_ext_chunks[] = {\n",
    );
    for (index, chunk) in chunks.iter().enumerate() {
        let present = compiled.is_some_and(|compiled| compiled.present.get(index) == Some(&true));
        let _ = writeln!(
            head,
            "    {{{},\n     {}, {}, {}, {}}},",
            c_bytes(chunk.source.as_bytes()),
            chunk.source.len(),
            c_bytes(chunk.filename.as_bytes()),
            i32::from(chunk.future_annotations),
            i32::from(present),
        );
    }
    let _ = writeln!(
        head,
        "}};\nstatic const Py_ssize_t pycc_ext_chunk_count = {};\n",
        chunks.len()
    );
    head.push_str(PRECOMPILED_C);
    let marker = "#define PYCC_EXT_MODULE_NAME_STR";
    let at = inc
        .find(marker)
        .and_then(|start| inc[start..].find("\n\n").map(|end| start + end + 2))
        .expect("the generated companion defines the module name first");
    format!("{}{head}{}", &inc[..at], &inc[at..])
}

/// The hook and its install/remove pair (`module_init.c` calls these
/// around the body).
const PRECOMPILED_C: &str = r#"/*
 * 1 when the embedded code objects are this interpreter's
 * (`pycc_ext_marshal_loads`). Decided once per process; `marshal.loads`
 * (borrowed from the runtime unit, which keeps it) is kept with it.
 */
static int pycc_ext_chunk_usable = -1;
static PyObject *pycc_ext_chunk_loads = NULL;
/* The unmarshalled table (`pycc_ext_chunk_table_data`), from the hook's
 * first call until the body has run. */
static PyObject *pycc_ext_chunk_table = NULL;
/* `fix(code, orders)` (`ORDERS_FIX` in `src/ext_build/precompiled.rs`),
 * made once from the table's last entry; NULL when unavailable. */
static PyObject *pycc_ext_chunk_fix = NULL;

/* `pycc_ext_chunk_fix`, made by running the table's last entry; NULL (no
 * exception left set) when it cannot be. */
static PyObject *pycc_ext_chunk_fixer(void)
{
    PyObject *code, *ns;

    if (pycc_ext_chunk_fix != NULL) {
        return pycc_ext_chunk_fix;
    }
    code = PyTuple_GetItem(pycc_ext_chunk_table, pycc_ext_chunk_count);
    ns = code == NULL ? NULL : PyDict_New();
    if (ns != NULL && PyDict_SetItemString(ns, "__builtins__", PyEval_GetBuiltins()) == 0) {
        PyObject *done = PyEval_EvalCode(code, ns, ns);

        Py_XDECREF(done);
        if (done != NULL) {
            pycc_ext_chunk_fix = PyDict_GetItemString(ns, "fix");
            Py_XINCREF(pycc_ext_chunk_fix);
        }
    }
    Py_XDECREF(ns);
    PyErr_Clear();
    return pycc_ext_chunk_fix;
}

static int pycc_ext_chunks_usable(void)
{
    if (pycc_ext_chunk_usable >= 0) {
        return pycc_ext_chunk_usable;
    }
    pycc_ext_chunk_loads = pycc_ext_chunk_magic == 0 ? NULL : pycc_ext_marshal_loads(pycc_ext_chunk_magic);
    pycc_ext_chunk_usable = pycc_ext_chunk_loads != NULL;
    return pycc_ext_chunk_usable;
}

/* `compile()` of chunk `chunk`'s embedded source, as the rewrite did. */
static PyObject *pycc_ext_chunk_compile(const struct pycc_ext_chunk *chunk)
{
    PyObject *compile;
    PyObject *source;
    PyObject *flags;
    PyObject *result;

    compile = PyDict_GetItemString(PyEval_GetBuiltins(), "compile");
    if (compile == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc: builtins.compile is missing");
        return NULL;
    }
    if (chunk->future_annotations) {
        PyObject *future = PyImport_ImportModule("__future__");
        PyObject *feature;
        if (future == NULL) {
            return NULL;
        }
        feature = PyObject_GetAttrString(future, "annotations");
        Py_DECREF(future);
        if (feature == NULL) {
            return NULL;
        }
        flags = PyObject_GetAttrString(feature, "compiler_flag");
        Py_DECREF(feature);
    } else {
        flags = PyLong_FromLong(0);
    }
    if (flags == NULL) {
        return NULL;
    }
    source = PyUnicode_DecodeUTF8(chunk->source, chunk->source_len, NULL);
    if (source == NULL) {
        Py_DECREF(flags);
        return NULL;
    }
    result = PyObject_CallFunction(compile, "OssOi", source, chunk->filename, "exec", flags, 1);
    Py_DECREF(source);
    Py_DECREF(flags);
    return result;
}

/* The code object of chunk `index` from the table (unmarshalled on the
 * first call), its `frozenset` constants rebuilt in source order; NULL with
 * no exception set when it cannot be had that way. */
static PyObject *pycc_ext_chunk_precompiled(Py_ssize_t index)
{
    PyObject *entry, *code, *orders, *fix;

    if (pycc_ext_chunk_table == NULL) {
        PyObject *view = PyMemoryView_FromMemory((char *)pycc_ext_chunk_table_data,
                                                 pycc_ext_chunk_table_len, PyBUF_READ);

        if (view != NULL) {
            pycc_ext_chunk_table = PyObject_CallFunctionObjArgs(pycc_ext_chunk_loads, view, NULL);
            Py_DECREF(view);
        }
        if (pycc_ext_chunk_table != NULL
            && (!PyTuple_Check(pycc_ext_chunk_table)
                || PyTuple_Size(pycc_ext_chunk_table) != pycc_ext_chunk_count + 1)) {
            Py_CLEAR(pycc_ext_chunk_table);
        }
        if (pycc_ext_chunk_table == NULL) {
            PyErr_Clear();
            return NULL;
        }
    }
    entry = PyTuple_GetItem(pycc_ext_chunk_table, index);
    if (entry == NULL || !PyTuple_Check(entry) || PyTuple_Size(entry) != 2) {
        PyErr_Clear();
        return NULL;
    }
    code = PyTuple_GetItem(entry, 0);
    orders = PyTuple_GetItem(entry, 1);
    if (orders == Py_None) {
        return Py_NewRef(code);
    }
    fix = pycc_ext_chunk_fixer();
    code = fix == NULL ? NULL : PyObject_CallFunctionObjArgs(fix, code, orders, NULL);
    if (code == NULL) {
        PyErr_Clear();
    }
    return code;
}

/* The code object of chunk `index`; NULL with an exception set. */
static PyObject *pycc_ext_chunk_code(Py_ssize_t index)
{
    const struct pycc_ext_chunk *chunk;

    if (index < 0 || index >= pycc_ext_chunk_count) {
        PyErr_SetString(PyExc_SystemError, "pycc: no such pre-compiled chunk");
        return NULL;
    }
    chunk = &pycc_ext_chunks[index];
    if (chunk->precompiled && pycc_ext_chunks_usable()) {
        PyObject *code = pycc_ext_chunk_precompiled(index);

        if (code != NULL) {
            return code;
        }
        /* Unreadable after all: compile the source instead. */
    }
    return pycc_ext_chunk_compile(chunk);
}

/* `__pycc_code__(k)`: the code object of chunk `k`. */
static PyObject *pycc_ext_code(PyObject *self, PyObject *arg)
{
    Py_ssize_t index = PyLong_AsSsize_t(arg);

    (void)self;
    if (index == -1 && PyErr_Occurred()) {
        return NULL;
    }
    return pycc_ext_chunk_code(index);
}

static PyMethodDef pycc_ext_code_def = {"__pycc_code__", pycc_ext_code, METH_O, NULL};

/* Binds the hook in `module`'s namespace for the body; 0, or -1 with an
 * exception set. */
static int pycc_ext_code_install(PyObject *module)
{
    PyObject *hook = PyCFunction_NewEx(&pycc_ext_code_def, NULL, NULL);
    int status;

    if (hook == NULL) {
        return -1;
    }
    status = PyDict_SetItemString(PyModule_GetDict(module), "__pycc_code__", hook);
    Py_DECREF(hook);
    return status;
}

/* Removes the hook once the body has run, keeping any exception the body
 * raised. */
static void pycc_ext_code_remove(PyObject *module)
{
    PyObject *raised = PyErr_GetRaisedException();
    PyObject *dict = PyModule_GetDict(module);

    if (PyDict_DelItemString(dict, "__pycc_code__") != 0) {
        PyErr_Clear();
    }
    /* No chunk is read after the body: the code objects it made live on in
     * what it defined. */
    Py_CLEAR(pycc_ext_chunk_table);
    PyErr_SetRaisedException(raised);
}

"#;

#[cfg(test)]
mod tests {
    use super::*;

    const INC: &str = "/* Generated */\n#define PYCC_EXT_MODULE_NAME m\n\
                       #define PYCC_EXT_MODULE_NAME_STR \"m\"\n\nBODY\n";

    fn chunk(source: &str) -> CodeChunk {
        CodeChunk {
            source: source.to_string(),
            filename: "/p/m.py".to_string(),
            future_annotations: false,
        }
    }

    #[test]
    fn no_chunk_leaves_the_companion_unchanged() {
        assert_eq!(install(INC.to_string(), &[], None), INC);
    }

    #[test]
    fn chunks_are_embedded_after_the_module_name_with_their_code() {
        let compiled = Precompiled {
            magic: 3571,
            table: vec![0xa9, b'"', b'a'],
            present: vec![true, false],
        };
        let out = install(
            INC.to_string(),
            &[chunk("\n\nx = \"?\\\"\n"), chunk("y = 1\n")],
            Some(&compiled),
        );
        let head = out
            .find("#define PYCC_EXT_HAS_PRECOMPILED")
            .expect("the macro");
        assert!(head > out.find("PYCC_EXT_MODULE_NAME_STR").unwrap());
        assert!(head < out.find("BODY").unwrap());
        assert!(out.contains("pycc_ext_chunk_magic = 3571L;"), "{out}");
        assert!(
            out.contains("pycc_ext_chunk_table_data[] =\n    \"\\251\\042a\";"),
            "{out}"
        );
        assert!(out.contains("pycc_ext_chunk_table_len = 3;"), "{out}");
        assert!(
            out.contains(
                "{\"\\012\\012x = \\042\\077\\134\\042\\012\",\n     11, \"/p/m.py\", 0, 1},"
            ),
            "{out}"
        );
        assert!(
            out.contains("{\"y = 1\\012\",\n     6, \"/p/m.py\", 0, 0},"),
            "{out}"
        );
        assert!(out.contains("pycc_ext_chunk_count = 2;"), "{out}");
    }

    #[test]
    fn without_compiled_code_only_the_source_is_embedded() {
        let out = install(INC.to_string(), &[chunk("y = 1\n")], None);
        assert!(out.contains("pycc_ext_chunk_magic = 0L;"), "{out}");
        assert!(out.contains("pycc_ext_chunk_table_len = 0;"), "{out}");
        assert!(out.contains("0, 0},"), "{out}");
    }

    #[test]
    fn a_long_literal_is_split_over_lines() {
        let text = "a".repeat(300);
        let literal = c_bytes(text.as_bytes());
        assert_eq!(literal.matches("\"\n    \"").count(), 2);
        assert_eq!(literal.replace("\"\n    \"", "").len(), 302);
    }

    #[test]
    fn the_answer_parses_only_when_complete() {
        assert_eq!(
            parse(b"\xf3\x0d\x0d\x0a\x01\x00\xa9\xff", 2),
            Some(Precompiled {
                magic: 0x0a0d_0df3,
                table: vec![0xa9, 0xff],
                present: vec![true, false],
            })
        );
        // Too short for the magic number, for one byte per chunk, or for a
        // table; a presence byte that is not 0 or 1; no magic number.
        assert_eq!(parse(b"\xf3\x0d", 0), None);
        assert_eq!(parse(b"\xf3\x0d\x0d\x0a\x01", 2), None);
        assert_eq!(parse(b"\xf3\x0d\x0d\x0a\x01", 1), None);
        assert_eq!(parse(b"\xf3\x0d\x0d\x0a\x02\xa9", 1), None);
        assert_eq!(parse(b"\x00\x00\x00\x00\xa9", 0), None);
    }

    #[test]
    fn the_input_is_marshal_data() {
        let mut first = chunk("x = 1\n");
        first.future_annotations = true;
        let input = marshal_input(&[first, chunk("é")]).expect("small enough");
        let mut want = b"(\x02\x00\x00\x00(\x02\x00\x00\x00".to_vec();
        want.extend_from_slice(b"(\x03\x00\x00\x00u\x06\x00\x00\x00x = 1\n");
        want.extend_from_slice(b"u\x07\x00\x00\x00/p/m.pyT");
        want.extend_from_slice(b"(\x03\x00\x00\x00u\x02\x00\x00\x00\xc3\xa9");
        want.extend_from_slice(b"u\x07\x00\x00\x00/p/m.pyF");
        want.push(b'u');
        want.extend_from_slice(&u32::try_from(ORDERS_FIX.len()).unwrap().to_le_bytes());
        want.extend_from_slice(ORDERS_FIX.as_bytes());
        assert_eq!(input, want);
    }

    #[test]
    fn a_real_interpreter_compiles_the_chunks() {
        let Some(compiled) = precompile(
            OsStr::new(super::super::DEFAULT_PYTHON),
            &[
                chunk("x = 1\n"),
                chunk("def (:\n"),
                chunk("def f(k):\n    return k in {3, 1, 2}\n"),
                chunk("a = {1, 2, 3}\nb = {3, 2, 1}\n"),
            ],
        ) else {
            // No interpreter on this host: the build embeds the source only.
            return;
        };
        assert_ne!(compiled.magic, 0);
        assert!(!compiled.table.is_empty());
        // A set display's constant comes with its source order; two
        // displays of the same elements in different orders leave the
        // chunk to `compile()`, as does a syntax error.
        assert_eq!(compiled.present, vec![true, false, true, false]);
    }

    #[test]
    fn an_interpreter_that_cannot_run_compiles_nothing() {
        assert_eq!(
            precompile(OsStr::new("/nonexistent/python3"), &[chunk("x = 1\n")]),
            None
        );
    }
}
