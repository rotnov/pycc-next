//! The CPython code an `--ext` module runs, compiled at build time
//! (docs/design/compile-little.md, section 2.4).
//!
//! A module with selected functions runs its whole source as CPython
//! bytecode, and a published signature (#1194) runs a small wrapper source;
//! each is a [`CodeChunk`]. The build compiles every chunk once with the
//! target interpreter (`compile(source, filename, "exec", dont_inherit=True,
//! optimize=0)`, in a fresh `-I` process) and embeds the module's code
//! objects as one `marshal`led tuple with the interpreter's bytecode magic
//! number, so an import unmarshals them as CPython loads a `.pyc`. The
//! module runs chunk `k` through `__pycc_run__(k)` (`module_code.c`), bound
//! in its namespace only while its body runs, as `exec` of its code object
//! would. The hook unmarshals the tuple on its first call when the running
//! interpreter has the same magic number and runs unoptimized (`-O` strips
//! `assert`s at compile time), and otherwise compiles the embedded source
//! exactly as the build did: same file name, so `co_filename`, line numbers
//! and tracebacks are the same either way. A build that cannot run the
//! interpreter (`PYCC_PYTHON_INCLUDE` without one) embeds the source only.
//!
//! `marshal` writes a `frozenset`'s elements sorted, so a `frozenset`
//! constant (a display of three or more constants, or `x in {...}`) loaded
//! from it would iterate in that order, not in a fresh compile's -- the
//! order of `print({1, 8, 0})` run from source, which is what pycc matches.
//! The build therefore also records, for each such constant of numbers
//! (and tuples of them) whose order `marshal` changes, the source display
//! it was folded from (its elements in source order, checked to rebuild the
//! compiled table), and the hook rebuilds those constants from it after
//! unmarshalling. The build checks the result, fail-closed: a module whose
//! rebuilt bytecode would not iterate every such constant as a fresh compile
//! does is compiled from its source at import instead. A constant with a
//! `str` or `bytes` keeps `marshal`'s
//! order (docs/DEVIATIONS.md): its fresh-compile order depends on the
//! process's hash key and on what it interned before.

use crate::fallback::CodeChunk;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::io::Write as _;
use std::process::Stdio;

/// What the target interpreter compiled: its magic number, the `marshal`led
/// pair of every chunk's code object (`None` for a chunk it refused to
/// compile, which then fails at import exactly as `compile()` does, or whose
/// `frozenset` constants its bytecode would not reproduce, which is then
/// compiled at import) and the fix-ups, and which chunks are precompiled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Precompiled {
    pub(crate) magic: u32,
    pub(crate) table: Vec<u8>,
    pub(crate) present: Vec<bool>,
}

/// Compiles the chunks of its stdin (`marshal`'s form of `((source,
/// filename), ...)`) and writes the bytecode magic number (four bytes), one
/// byte per chunk (1 when it compiled) and the `marshal`led pair of their
/// code objects and the `frozenset` constants to rebuild after loading:
/// `(chunk, path, index, elements)`, the constant at `co_consts[index]` of
/// the code object reached from the chunk's by the `co_consts` indexes in
/// `path`, rebuilt as `frozenset(elements)`.
///
/// A constant gets a fix-up when its hashes are `fixed` (no `str` or
/// `bytes`, whose hashes are randomized per process) and `marshal` changes
/// its table. Its elements are those of a set display of the chunk's source,
/// in source order: the display at the source position of the first
/// instruction that loads the constant (`loaded_at`), else (a constant
/// merged with an equal one folded elsewhere) another display that rebuilds
/// it.
///
/// The gate is the result, not the matching. Each chunk is unmarshalled
/// again with its fix-ups applied (`refold`, as the hook does), and every
/// `frozenset` constant of numbers in every code object of it must iterate
/// as the fresh compile's (`agrees`, through `same`: element by element by
/// type and `repr`, recursively through tuples, also once copied into a set
/// and grown, as `SET_UPDATE` and later `add`s would). A chunk for which
/// that fails (a display `ast.literal_eval` cannot read, as `{1 + 0, 8, 0}`)
/// or raises is not precompiled: the hook compiles its source at import, as
/// for a chunk the interpreter refused to compile.
///
/// `marshal` data rather than JSON: importing `json` doubled the
/// interpreter's start-up, and the script runs once per built module; `ast`
/// and `dis` are imported only for a chunk with such a constant.
const COMPILE_SCRIPT: &str = "\
import importlib.util, marshal, sys
def key(value):
    if isinstance(value, tuple):
        return type(value), tuple(key(item) for item in value)
    return type(value), repr(value)
def same(a, b):
    if [key(v) for v in a] != [key(v) for v in b]:
        return False
    for extra in range(-8, 40):
        x = set(a); y = set(b); x.add(extra); y.add(extra)
        if [key(v) for v in x] != [key(v) for v in y]:
            return False
    return True
def fixed(value):
    if isinstance(value, (tuple, frozenset)):
        return all(fixed(item) for item in value)
    return value is None or isinstance(value, (int, float, complex))
def rebuilds(order, const):
    try:
        built = frozenset(order)
    except TypeError:
        return False
    return same(built, const)
def frozensets(code, path):
    for index, const in enumerate(code.co_consts):
        if isinstance(const, frozenset):
            yield code, path, index, const
        elif hasattr(const, 'co_consts'):
            yield from frozensets(const, path + (index,))
def displays(source):
    import ast
    out = {}
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.Set):
            try:
                order = tuple(ast.literal_eval(e) for e in node.elts)
            except (ValueError, TypeError, SyntaxError, MemoryError, RecursionError):
                continue
            out[(node.lineno, node.col_offset, node.end_lineno, node.end_col_offset)] = order
    return out
def loaded_at(code, index):
    import dis
    for ins in dis.get_instructions(code):
        if ins.opname == 'LOAD_CONST' and ins.arg == index and ins.positions is not None:
            p = ins.positions
            return p.lineno, p.col_offset, p.end_lineno, p.end_col_offset
    return None
def refold(code, path, index, order):
    consts = list(code.co_consts)
    if path:
        consts[path[0]] = refold(consts[path[0]], path[1:], index, order)
    else:
        consts[index] = frozenset(order)
    return code.replace(co_consts=tuple(consts))
def agrees(loaded, compiled):
    for x, y in zip(loaded.co_consts, compiled.co_consts, strict=True):
        if isinstance(y, frozenset):
            if fixed(y) and not same(x, y):
                return False
        elif hasattr(y, 'co_consts') and not agrees(x, y):
            return False
    return True
chunks = marshal.loads(sys.stdin.buffer.read())
codes = []
fixups = []
for k, (source, filename) in enumerate(chunks):
    try:
        code = compile(source, filename, 'exec', dont_inherit=True, optimize=0)
    except (SyntaxError, ValueError):
        code = None
    orders = None
    mine = []
    for owner, path, index, const in (() if code is None else frozensets(code, ())):
        if not fixed(const) or same(marshal.loads(marshal.dumps(const)), const):
            continue
        if orders is None:
            orders = displays(source)
        first = orders.get(loaded_at(owner, index))
        for order in ([first] if first is not None else []) + list(orders.values()):
            if rebuilds(order, const):
                mine.append((k, path, index, order))
                break
    if code is not None:
        try:
            loaded = marshal.loads(marshal.dumps(code))
            for _, path, index, order in mine:
                loaded = refold(loaded, path, index, order)
            ok = agrees(loaded, code)
        except Exception:
            ok = False
        if not ok:
            code, mine = None, []
    codes.append(code)
    fixups.extend(mine)
out = sys.stdout.buffer
out.write(importlib.util.MAGIC_NUMBER)
out.write(bytes(c is not None for c in codes))
out.write(marshal.dumps((tuple(codes), tuple(fixups))))
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

/// The compile script's input: `marshal`'s form of `((source, filename),
/// ...)`, built from its tuple (`(`) and string (`u`, UTF-8) codes, each
/// length a 4-byte little-endian count -- a format every `marshal` version
/// reads. `None` for a string or tuple too long for one.
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
    sized(&mut out, b'(', chunks.len())?;
    for chunk in chunks {
        sized(&mut out, b'(', 2)?;
        text(&mut out, &chunk.source)?;
        text(&mut out, &chunk.filename)?;
    }
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
    let mut head = String::from("#define PYCC_EXT_HAS_MODULE_CODE 1\n\n");
    let _ = writeln!(
        head,
        "/* The bytecode magic number the embedded code objects were compiled for\n \
         * (0: none embedded). */\nstatic const long pycc_ext_chunk_magic = {}L;\n",
        compiled.map_or(0, |compiled| compiled.magic)
    );
    let table = compiled.map_or(&[][..], |compiled| compiled.table.as_slice());
    let _ = writeln!(
        head,
        "/* The marshalled tuple of the chunks' code objects. */\n\
         static const char pycc_ext_chunk_table_data[] =\n    {};\n\
         static const Py_ssize_t pycc_ext_chunk_table_len = {};\n",
        c_bytes(table),
        table.len()
    );
    // Each chunk's source and file name are offsets into one string,
    // `pycc_ext_chunk_text`, instead of pointers: a pointer needs a
    // load-time relocation, which dirties the table's pages in every
    // process that loads the artifact. A file name is stored once, with its
    // NUL, however many chunks name it.
    let mut text: Vec<u8> = Vec::new();
    let mut files: Vec<(&str, usize)> = Vec::new();
    let mut rows = String::new();
    for (index, chunk) in chunks.iter().enumerate() {
        let present = compiled.is_some_and(|compiled| compiled.present.get(index) == Some(&true));
        let source = text.len();
        text.extend_from_slice(chunk.source.as_bytes());
        let filename = match files.iter().find(|(name, _)| *name == chunk.filename) {
            Some(&(_, at)) => at,
            None => {
                let at = text.len();
                text.extend_from_slice(chunk.filename.as_bytes());
                text.push(0);
                files.push((&chunk.filename, at));
                at
            }
        };
        let _ = writeln!(
            rows,
            "    {{{source}, {}, {filename}, {}}},",
            chunk.source.len(),
            i32::from(present),
        );
    }
    let _ = writeln!(
        head,
        "/* The chunks' sources and file names, which `pycc_ext_chunks` indexes. */\n\
         static const char pycc_ext_chunk_text[] =\n    {};\n",
        c_bytes(&text)
    );
    head.push_str(
        "static const struct pycc_ext_chunk {\n    Py_ssize_t source;\n    \
         Py_ssize_t source_len;\n    Py_ssize_t filename;\n    int precompiled;\n} \
         pycc_ext_chunks[] = {\n",
    );
    head.push_str(&rows);
    let _ = writeln!(
        head,
        "}};\nstatic const Py_ssize_t pycc_ext_chunk_count = {};\n",
        chunks.len()
    );
    head.push_str(MODULE_CODE_C);
    let marker = "#define PYCC_EXT_MODULE_NAME_STR";
    let at = inc
        .find(marker)
        .and_then(|start| inc[start..].find("\n\n").map(|end| start + end + 2))
        .expect("the generated companion defines the module name first");
    format!("{}{head}{}", &inc[..at], &inc[at..])
}

/// The hook and its install/remove pair (`module_init.c` calls these
/// around the body), and `pycc_ext_chunk_run_in` (`signatures.rs`).
const MODULE_CODE_C: &str = include_str!("module_code.c");

#[cfg(test)]
mod tests;
