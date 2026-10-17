//! Shipping an `--ext` module's CPython source pre-compiled (v0.8).
//!
//! The fallback rewrite (`crate::fallback`) runs each piece of source pycc
//! does not compile -- a class statement CPython runs, the methods that
//! fall back, the copies Python callers read -- with `exec` while the
//! module body runs. Compiling that source with `compile()` on every
//! import cost pydantic's import more than all of CPython's own work, which
//! a `.pyc` cache saves CPython. Instead, the build compiles every chunk
//! (`crate::fallback::CodeChunk`) once with the target interpreter and
//! embeds the `marshal`led code object, with the interpreter's bytecode
//! magic number; the rewrite reads each code object through the module's
//! `__pycc_code__` hook (`PRECOMPILED_C`), present in the namespace only
//! while the body runs. The hook unmarshals the embedded code when the
//! running interpreter has the same magic number and runs unoptimized
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

/// What the target interpreter compiled: its magic number and each
/// chunk's marshalled code object (`None` for one it refused to compile,
/// which then fails at import exactly as `compile()` did).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Precompiled {
    pub(crate) magic: u32,
    pub(crate) code: Vec<Option<Vec<u8>>>,
}

/// The script compiling the chunks of its stdin (JSON) to marshalled code.
const COMPILE_SCRIPT: &str = "import sys,json,marshal,importlib.util,__future__\n\
    chunks=json.loads(sys.stdin.buffer.read())\n\
    out=[]\n\
    for c in chunks:\n\
    \x20 try:\n\
    \x20   f=__future__.annotations.compiler_flag if c['future'] else 0\n\
    \x20   out.append(marshal.dumps(compile(c['source'],c['filename'],'exec',f,True,0)).hex())\n\
    \x20 except Exception:\n\
    \x20   out.append(None)\n\
    sys.stdout.write(json.dumps({'magic':int.from_bytes(importlib.util.MAGIC_NUMBER,'little'),'code':out}))\n";

/// Compiles `chunks` with `interpreter`; `None` when it cannot run or
/// answers something this build cannot read.
pub(crate) fn precompile(interpreter: &OsStr, chunks: &[CodeChunk]) -> Option<Precompiled> {
    let input = serde_json::to_vec(
        &chunks
            .iter()
            .map(|chunk| {
                serde_json::json!({
                    "source": chunk.source,
                    "filename": chunk.filename,
                    "future": chunk.future_annotations,
                })
            })
            .collect::<Vec<_>>(),
    )
    .ok()?;
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

/// Parses the compile script's answer for `count` chunks.
fn parse(stdout: &[u8], count: usize) -> Option<Precompiled> {
    let value: serde_json::Value = serde_json::from_slice(stdout).ok()?;
    let magic = u32::try_from(value.get("magic")?.as_u64()?).ok()?;
    let code = value
        .get("code")?
        .as_array()?
        .iter()
        .map(|entry| match entry {
            serde_json::Value::Null => Some(None),
            serde_json::Value::String(hex) => decode_hex(hex).map(Some),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    (code.len() == count && magic != 0).then_some(Precompiled { magic, code })
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok())
        .collect()
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
    head.push_str(
        "static const struct pycc_ext_chunk {\n    const char *source;\n    \
         Py_ssize_t source_len;\n    const char *filename;\n    int future_annotations;\n    \
         const char *code;\n    Py_ssize_t code_len;\n} pycc_ext_chunks[] = {\n",
    );
    for (index, chunk) in chunks.iter().enumerate() {
        let code = compiled.and_then(|compiled| compiled.code.get(index)?.as_deref());
        let _ = writeln!(
            head,
            "    {{{},\n     {}, {}, {},\n     {},\n     {}}},",
            c_bytes(chunk.source.as_bytes()),
            chunk.source.len(),
            c_bytes(chunk.filename.as_bytes()),
            i32::from(chunk.future_annotations),
            code.map_or_else(|| "NULL".to_string(), c_bytes),
            code.map_or(0, <[u8]>::len),
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
 * 1 when the embedded code objects are this interpreter's: the same
 * bytecode magic number, and no `-O` (which strips `assert`s at compile
 * time). Decided once per process; `marshal.loads` is kept with it.
 */
static int pycc_ext_chunk_usable = -1;
static PyObject *pycc_ext_chunk_loads = NULL;

static int pycc_ext_chunks_usable(void)
{
    PyObject *flags;
    PyObject *optimize;
    PyObject *marshal;
    long level;

    if (pycc_ext_chunk_usable >= 0) {
        return pycc_ext_chunk_usable;
    }
    pycc_ext_chunk_usable = 0;
    if (pycc_ext_chunk_magic == 0) {
        return 0;
    }
    if (PyImport_GetMagicNumber() != pycc_ext_chunk_magic) {
        PyErr_Clear();
        return 0;
    }
    flags = PySys_GetObject("flags");
    optimize = flags == NULL ? NULL : PyObject_GetAttrString(flags, "optimize");
    if (optimize == NULL) {
        PyErr_Clear();
        return 0;
    }
    level = PyLong_AsLong(optimize);
    Py_DECREF(optimize);
    if (level != 0) {
        PyErr_Clear();
        return 0;
    }
    marshal = PyImport_ImportModule("marshal");
    if (marshal == NULL) {
        PyErr_Clear();
        return 0;
    }
    pycc_ext_chunk_loads = PyObject_GetAttrString(marshal, "loads");
    Py_DECREF(marshal);
    if (pycc_ext_chunk_loads == NULL) {
        PyErr_Clear();
        return 0;
    }
    pycc_ext_chunk_usable = 1;
    return 1;
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

/* `__pycc_code__(k)`: the code object of chunk `k`. */
static PyObject *pycc_ext_code(PyObject *self, PyObject *arg)
{
    const struct pycc_ext_chunk *chunk;
    Py_ssize_t index = PyLong_AsSsize_t(arg);

    (void)self;
    if (index == -1 && PyErr_Occurred()) {
        return NULL;
    }
    if (index < 0 || index >= pycc_ext_chunk_count) {
        PyErr_SetString(PyExc_SystemError, "pycc: no such pre-compiled chunk");
        return NULL;
    }
    chunk = &pycc_ext_chunks[index];
    if (chunk->code != NULL && pycc_ext_chunks_usable()) {
        PyObject *view = PyMemoryView_FromMemory((char *)chunk->code, chunk->code_len, PyBUF_READ);
        PyObject *code;
        if (view == NULL) {
            return NULL;
        }
        code = PyObject_CallFunctionObjArgs(pycc_ext_chunk_loads, view, NULL);
        Py_DECREF(view);
        if (code != NULL) {
            return code;
        }
        /* Unreadable after all: compile the source instead. */
        PyErr_Clear();
    }
    return pycc_ext_chunk_compile(chunk);
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
            code: vec![Some(vec![0xe3, b'"', b'a']), None],
        };
        let out = install(
            INC.to_string(),
            &[chunk("\n\nx = \"?\\\"\n"), chunk("y = 1\n")],
            Some(&compiled),
        );
        let head = out.find("#define PYCC_EXT_HAS_PRECOMPILED").expect("the macro");
        assert!(head > out.find("PYCC_EXT_MODULE_NAME_STR").unwrap());
        assert!(head < out.find("BODY").unwrap());
        assert!(out.contains("pycc_ext_chunk_magic = 3571L;"), "{out}");
        assert!(out.contains("{\"\\012\\012x = \\042\\077\\134\\042\\012\",\n     11, \"/p/m.py\", 0,\n     \"\\343\\042a\",\n     3},"), "{out}");
        assert!(out.contains("{\"y = 1\\012\",\n     6, \"/p/m.py\", 0,\n     NULL,\n     0},"), "{out}");
        assert!(out.contains("pycc_ext_chunk_count = 2;"), "{out}");
    }

    #[test]
    fn without_compiled_code_only_the_source_is_embedded() {
        let out = install(INC.to_string(), &[chunk("y = 1\n")], None);
        assert!(out.contains("pycc_ext_chunk_magic = 0L;"), "{out}");
        assert!(out.contains("NULL,\n     0},"), "{out}");
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
            parse(br#"{"magic": 3571, "code": ["e3ff", null]}"#, 2),
            Some(Precompiled {
                magic: 3571,
                code: vec![Some(vec![0xe3, 0xff]), None]
            })
        );
        assert_eq!(parse(br#"{"magic": 3571, "code": ["e3ff"]}"#, 2), None);
        assert_eq!(parse(br#"{"magic": 3571, "code": ["e3f"]}"#, 1), None);
        assert_eq!(parse(br#"{"magic": 3571, "code": ["zz"]}"#, 1), None);
        assert_eq!(parse(br#"{"magic": 0, "code": []}"#, 0), None);
        assert_eq!(parse(b"not json", 0), None);
        assert_eq!(parse(br#"{"magic": 1, "code": [1]}"#, 1), None);
    }

    #[test]
    fn a_real_interpreter_compiles_the_chunks() {
        let Some(compiled) = precompile(
            OsStr::new(super::super::DEFAULT_PYTHON),
            &[chunk("x = 1\n"), chunk("def (:\n")],
        ) else {
            // No interpreter on this host: the build embeds the source only.
            return;
        };
        assert_ne!(compiled.magic, 0);
        assert!(compiled.code[0].as_ref().is_some_and(|code| !code.is_empty()));
        assert_eq!(compiled.code[1], None);
    }

    #[test]
    fn an_interpreter_that_cannot_run_compiles_nothing() {
        assert_eq!(precompile(OsStr::new("/nonexistent/python3"), &[chunk("x = 1\n")]), None);
    }
}
