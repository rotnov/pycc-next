use super::*;

const INC: &str = "/* Generated */\n#define PYCC_EXT_MODULE_NAME m\n\
                   #define PYCC_EXT_MODULE_NAME_STR \"m\"\n\nBODY\n";

fn chunk(source: &str) -> CodeChunk {
    CodeChunk {
        source: source.to_string(),
        filename: "/p/m.py".to_string(),
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
    let head = out.find("#define PYCC_EXT_HAS_MODULE_CODE").expect("the macro");
    assert!(head > out.find("PYCC_EXT_MODULE_NAME_STR").unwrap());
    assert!(head < out.find("BODY").unwrap());
    assert!(out.contains("pycc_ext_chunk_magic = 3571L;"), "{out}");
    assert!(out.contains("pycc_ext_chunk_table_data[] =\n    \"\\251\\042a\";"), "{out}");
    assert!(out.contains("pycc_ext_chunk_table_len = 3;"), "{out}");
    assert!(
        out.contains(
            "pycc_ext_chunk_text[] =\n    \
             \"\\012\\012x = \\042\\077\\134\\042\\012/p/m.py\\000y = 1\\012\";"
        ),
        "{out}"
    );
    // The second chunk's file name is the first's.
    assert!(out.contains("{0, 11, 11, 1},"), "{out}");
    assert!(out.contains("{19, 6, 11, 0},"), "{out}");
    assert!(out.contains("pycc_ext_chunk_count = 2;"), "{out}");
}

#[test]
fn without_compiled_code_only_the_source_is_embedded() {
    let out = install(INC.to_string(), &[chunk("y = 1\n")], None);
    assert!(out.contains("pycc_ext_chunk_magic = 0L;"), "{out}");
    assert!(out.contains("pycc_ext_chunk_table_len = 0;"), "{out}");
    assert!(out.contains("{0, 6, 6, 0},"), "{out}");
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
    let input = marshal_input(&[chunk("x = 1\n"), chunk("é")]).expect("small enough");
    let mut want = b"(\x02\x00\x00\x00".to_vec();
    want.extend_from_slice(b"(\x02\x00\x00\x00u\x06\x00\x00\x00x = 1\n");
    want.extend_from_slice(b"u\x07\x00\x00\x00/p/m.py");
    want.extend_from_slice(b"(\x02\x00\x00\x00u\x02\x00\x00\x00\xc3\xa9");
    want.extend_from_slice(b"u\x07\x00\x00\x00/p/m.py");
    assert_eq!(input, want);
}

#[test]
fn a_real_interpreter_compiles_the_chunks() {
    let Some(compiled) = precompile(
        OsStr::new(super::super::DEFAULT_PYTHON),
        &[chunk("x = 1\n"), chunk("def (:\n"), chunk("a = {1, 2, 3}\n")],
    ) else {
        // No interpreter on this host: the build embeds the source only.
        return;
    };
    assert_ne!(compiled.magic, 0);
    assert!(!compiled.table.is_empty());
    // A syntax error is left to `compile()` at import, which raises it.
    assert_eq!(compiled.present, vec![true, false, true]);
}

/// A frozenset constant `marshal` reorders is precompiled with a fix-up when
/// its display can be read back, and its chunk is left to `compile()` at
/// import when it cannot (`1 + 0` is folded, not a literal): never run with
/// `marshal`'s order.
#[test]
fn a_frozenset_constant_marshal_reorders_is_rebuilt_or_its_chunk_compiles_at_import() {
    let Some(compiled) = precompile(
        OsStr::new(super::super::DEFAULT_PYTHON),
        &[
            chunk("a = {1, 8, 0}\n"),
            chunk("def f():\n    return {1 + 0, 8, 0}\n"),
            chunk("g = lambda: {2 * 4, 1, 0}\n"),
            chunk("b = {'zz', 'q', 'abc'}\n"),
        ],
    ) else {
        return;
    };
    // The `str` display keeps `marshal`'s order (docs/DEVIATIONS.md).
    assert_eq!(compiled.present, vec![true, false, false, true]);
}

#[test]
fn an_interpreter_that_cannot_run_compiles_nothing() {
    assert_eq!(precompile(OsStr::new("/nonexistent/python3"), &[chunk("x = 1\n")]), None);
}
