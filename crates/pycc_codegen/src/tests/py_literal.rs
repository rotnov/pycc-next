//! v0.12: a `str` literal only ever packed for CPython is made as a CPython
//! `str` directly (`crate::py_literal`), pinned on the LLVM IR of an `ext`
//! object.

use super::*;

fn store(attr: &str, value: MirExpr) -> MirStmt {
    MirStmt::ObjAttrSet {
        base: numpy_pi(),
        attr: attr.to_string(),
        value,
    }
}

fn literal(text: &str) -> MirExpr {
    MirExpr::StringLiteral(text.to_string())
}

fn calls(ir: &str, callee: &str) -> usize {
    ir.lines()
        .filter(|line| line.contains(&format!("call ptr @{callee}(")))
        .count()
}

/// Each packed literal becomes one `pycc_ext_obj_str_literal` call, the
/// sites of one text share one slot, the interning flag is decided at
/// build time, and no native object is built for any of them.
#[test]
fn a_packed_literal_is_a_cpython_str_made_directly() {
    compile_ext_items_checking_ir(
        "py_literal_packed",
        with_foreign_numpy(
            [
                store("s", literal("k")),
                store("t", literal("k")),
                store("u", literal("a b")),
            ]
            .into_iter()
            .map(MirItem::TopLevelStmt)
            .collect(),
        ),
        |ir| {
            let symbol = crate::py_literal::EXT_OBJ_STR_LITERAL_SYMBOL;
            assert_eq!(calls(ir, symbol), 3, "{ir}");
            assert_eq!(calls(ir, crate::ext::EXT_OBJ_PACK_STR_SYMBOL), 0, "{ir}");
            assert_eq!(calls(ir, "pycc_rt_str_from_literal_cached"), 0, "{ir}");
            let slots = ir
                .lines()
                .filter(|line| line.starts_with("@py_lit_cache"))
                .count();
            assert_eq!(slots, 2, "{ir}");
            let flags: Vec<&str> = ir
                .lines()
                .filter(|line| line.contains(&format!("call ptr @{symbol}(")))
                .map(|line| line.rsplit("i32 ").next().unwrap_or_default())
                .collect();
            assert_eq!(flags, ["1)", "1)", "0)"], "{ir}");
            assert!(!ir.contains("str_lit_build"), "{ir}");
        },
    );
}

/// A literal whose value is also used natively -- bound to a local and
/// packed from there -- keeps its native object and its pack.
#[test]
fn a_literal_used_natively_keeps_its_native_object() {
    compile_ext_items_checking_ir(
        "py_literal_native",
        with_foreign_numpy(vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: literal("z"),
            }),
            MirItem::TopLevelStmt(store(
                "v",
                MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Str,
                },
            )),
        ]),
        |ir| {
            assert_eq!(calls(ir, crate::py_literal::EXT_OBJ_STR_LITERAL_SYMBOL), 0, "{ir}");
            assert_eq!(calls(ir, crate::ext::EXT_OBJ_PACK_STR_SYMBOL), 1, "{ir}");
            assert_eq!(calls(ir, "pycc_rt_str_from_literal_cached"), 1, "{ir}");
        },
    );
}

#[test]
fn interning_follows_cpythons_code_constants() {
    use crate::py_literal::interned_as_constant;
    assert!(interned_as_constant(b""));
    assert!(interned_as_constant(b" "));
    assert!(!interned_as_constant("é".as_bytes()));
    assert!(interned_as_constant(b"_a1Z"));
    assert!(!interned_as_constant(b"a b"));
    assert!(!interned_as_constant(b"a-b"));
}
