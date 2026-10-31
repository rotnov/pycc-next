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
//! the rewrite runs each chunk through the module's `__pycc_run__` hook
//! (`PRECOMPILED_C`), present in the namespace only while the body runs, as
//! `exec` of its code object would. A chunk of only `def` statements with
//! constant defaults and annotations (v0.11) is tabled as the functions'
//! own code objects and what `MAKE_FUNCTION` takes, and the hook makes the
//! functions directly: no wrapper code object to read, no frame to run.
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

use crate::fallback::{CodeChunk, CodeRewrite};
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::io::Write as _;
use std::process::Stdio;

/// What the target interpreter compiled: its magic number, the
/// `marshal`led table of every chunk's entry -- `(code, orders)`, its code
/// object and, when its code holds `frozenset` constants, their elements in
/// source order (a tuple of tuples, for [`ORDERS_FIX`]); for a chunk of only
/// constant `def`s, `((spec, ...),)`, what makes each function directly
/// (`precompile.py`'s `defs`, each spec with its function's orders); or
/// `None` for a chunk it refused to compile, which then fails at import
/// exactly as `compile()` did -- followed by [`ORDERS_FIX`]'s code object,
/// and which chunks have an entry.
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
/// The input is `marshal`'s form of `(((source, filename, future,
/// rewrites), ...), fix)` ([`marshal_input`]), each rewrite `(kind, path, a,
/// b)` -- `'q'` with `old`, `new` for [`CodeRewrite::Requal`], `'t'` with
/// the qualified name and the flags for [`CodeRewrite::Retag`] -- made on
/// the chunk's code before it is tabled; the answer is the bytecode magic number
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
///
/// A chunk of only `def`s whose defaults and annotations are constants
/// (v0.11) is tabled as each function's code and the constants
/// `MAKE_FUNCTION` takes, found by reading the chunk's bytecode, so the
/// import makes the functions without the chunk's own code object or a
/// frame (`precompiled_hook.c`). Kept in `precompile.py`; the build
/// normalizes its line endings.
const COMPILE_SCRIPT: &str = include_str!("precompile.py");

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
    let mut child = super::probe_command(interpreter, &COMPILE_SCRIPT.replace("\r\n", "\n"))
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
        sized(&mut out, b'(', 4)?;
        text(&mut out, &chunk.source)?;
        text(&mut out, &chunk.filename)?;
        out.push(if chunk.future_annotations { b'T' } else { b'F' });
        sized(&mut out, b'(', chunk.rewrites.len())?;
        for rewrite in &chunk.rewrites {
            let (kind, path, a) = match rewrite {
                CodeRewrite::Requal { path, old, .. } => ("q", path, old),
                CodeRewrite::Retag { path, qualname, .. } => ("t", path, qualname),
            };
            sized(&mut out, b'(', 4)?;
            text(&mut out, kind)?;
            sized(&mut out, b'(', path.len())?;
            for name in path {
                text(&mut out, name)?;
            }
            text(&mut out, a)?;
            match rewrite {
                CodeRewrite::Requal { new, .. } => text(&mut out, new)?,
                CodeRewrite::Retag { flags, .. } => {
                    out.push(b'i');
                    out.extend_from_slice(&i32::try_from(*flags).ok()?.to_le_bytes());
                }
            }
        }
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
/// around the body), and `pycc_ext_chunk_run_in` (`signatures.rs`).
const PRECOMPILED_C: &str = include_str!("precompiled_hook.c");

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
            rewrites: Vec::new(),
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
        let mut second = chunk("é");
        second.rewrites = vec![
            CodeRewrite::Requal {
                path: vec!["mk".to_string(), "f".to_string()],
                old: "mk.<locals>.f".to_string(),
                new: "g".to_string(),
            },
            CodeRewrite::Retag {
                path: vec!["h".to_string()],
                qualname: "C.h".to_string(),
                flags: 0x4000_0000,
            },
        ];
        let input = marshal_input(&[first, second]).expect("small enough");
        let mut want = b"(\x02\x00\x00\x00(\x02\x00\x00\x00".to_vec();
        want.extend_from_slice(b"(\x04\x00\x00\x00u\x06\x00\x00\x00x = 1\n");
        want.extend_from_slice(b"u\x07\x00\x00\x00/p/m.pyT(\x00\x00\x00\x00");
        want.extend_from_slice(b"(\x04\x00\x00\x00u\x02\x00\x00\x00\xc3\xa9");
        want.extend_from_slice(b"u\x07\x00\x00\x00/p/m.pyF(\x02\x00\x00\x00");
        want.extend_from_slice(b"(\x04\x00\x00\x00u\x01\x00\x00\x00q(\x02\x00\x00\x00");
        want.extend_from_slice(b"u\x02\x00\x00\x00mku\x01\x00\x00\x00f");
        want.extend_from_slice(b"u\x0d\x00\x00\x00mk.<locals>.fu\x01\x00\x00\x00g");
        want.extend_from_slice(b"(\x04\x00\x00\x00u\x01\x00\x00\x00t(\x01\x00\x00\x00");
        want.extend_from_slice(b"u\x01\x00\x00\x00hu\x03\x00\x00\x00C.hi\x00\x00\x00\x40");
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
    fn a_real_interpreter_applies_the_rewrites() {
        let mut nested = chunk("def mk():\n def f():\n  def g(): pass\n  return g\n return f\n");
        nested.rewrites = vec![CodeRewrite::Requal {
            path: vec!["mk".to_string(), "f".to_string()],
            old: "mk.<locals>.f".to_string(),
            new: "C.m".to_string(),
        }];
        let mut wrapper = chunk("def _pycc_make(e):\n def m(): pass\n return m\n");
        wrapper.rewrites = vec![CodeRewrite::Retag {
            path: vec!["_pycc_make".to_string(), "m".to_string()],
            qualname: "C.m".to_string(),
            flags: 0x4000_0000,
        }];
        let Some(compiled) = precompile(OsStr::new(super::super::DEFAULT_PYTHON), &[nested, wrapper])
        else {
            return;
        };
        assert_eq!(compiled.present, vec![true, true]);
        // Both chunks are a bare `def`: tabled as the function's code.
        let check = "import marshal,sys,types\n\
            t=marshal.loads(sys.stdin.buffer.read())\n\
            ns={}\n\
            f=types.FunctionType(t[0][0][0][0],ns)()\n\
            print(f.__code__.co_qualname, f().__qualname__)\n\
            m=types.FunctionType(t[1][0][0][0],ns)(0)\n\
            print(m.__code__.co_qualname, hex(m.__code__.co_flags & 0x40000000))\n";
        let mut child = std::process::Command::new(super::super::DEFAULT_PYTHON)
            .args(["-I", "-c", check])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("the interpreter that compiled the table runs");
        child
            .stdin
            .take()
            .expect("piped")
            .write_all(&compiled.table)
            .expect("the table is written");
        let out = child.wait_with_output().expect("it finishes");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
            "C.m C.m.<locals>.g\nC.m 0x40000000\n"
        );
    }

    #[test]
    fn a_real_interpreter_tables_constant_defs_as_their_functions() {
        let chunks = [
            chunk("def f(a, b=(1, 2), *, c=3):\n    return a\ndef g(): pass\n__pycc_forward__('g')\n"),
            chunk("@d\ndef f(): pass\n"),
            chunk("def f(a=X): pass\n"),
            chunk("def f():\n    return 1 in {3, 1, 2}\n"),
            chunk("x = 1\ndef f(): pass\n"),
        ];
        let Some(compiled) = precompile(OsStr::new(super::super::DEFAULT_PYTHON), &chunks) else {
            return;
        };
        assert_eq!(compiled.present, vec![true; 5]);
        // A `def` entry is `((spec, ...),)`: each function's code, defaults,
        // keyword defaults, annotations, name, forwarded names and set
        // orders; anything else `(code, orders)`.
        let check = "import marshal,sys\n\
            t=marshal.loads(sys.stdin.buffer.read())\n\
            print([len(e) for e in t[:-1]])\n\
            (f,g),=t[0]\n\
            print(f[0].co_name, f[1:6], g[0].co_name, g[1:6])\n\
            print(t[3][0][0][6], tuple(len(s) for s in t[0][0]))\n";
        let mut child = std::process::Command::new(super::super::DEFAULT_PYTHON)
            .args(["-I", "-c", check])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("the interpreter that compiled the table runs");
        child
            .stdin
            .take()
            .expect("piped")
            .write_all(&compiled.table)
            .expect("the table is written");
        let out = child.wait_with_output().expect("it finishes");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
            "[1, 2, 2, 1, 2]\n\
             f (((1, 2),), {'c': 3}, None, 'f', None) g (None, None, None, 'g', ('g',))\n\
             ((3, 1, 2),) (7, 7)\n"
        );
    }

    #[test]
    fn an_interpreter_that_cannot_run_compiles_nothing() {
        assert_eq!(
            precompile(OsStr::new("/nonexistent/python3"), &[chunk("x = 1\n")]),
            None
        );
    }
}
