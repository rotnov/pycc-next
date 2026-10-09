//! #1475 (Part 2 of #1387): a native value boxed into an `object` slot
//! (`object_box.rs`), pinned on the LLVM IR of an `ext` object.
//!
//! Each seam calls exactly one packer for the value's own type, checks the
//! packer's result for `NULL` (the #1040 bigint `OverflowError`) and never
//! releases the new reference (#1092's leak-only rule). A `None` value is
//! CPython's own borrowed `Py_None`, packed by nothing.

use super::*;

fn object_fn(name: &str, params: Vec<(String, Ty)>, body: Vec<MirStmt>) -> MirItem {
    MirItem::Function {
        name: name.to_string(),
        params,
        return_ty: Ty::Object,
        body,
    }
}

/// The text of `pyfn_<name>`'s definition in `ir`.
fn body_of<'ir>(ir: &'ir str, name: &str) -> &'ir str {
    let header = format!("@pyfn_{name}(");
    let start = ir
        .lines()
        .position(|line| line.starts_with("define") && line.contains(&header))
        .unwrap_or_else(|| panic!("`pyfn_{name}` should be defined: {ir}"));
    let rest: Vec<&str> = ir.lines().skip(start).collect();
    let end = rest
        .iter()
        .position(|line| *line == "}")
        .expect("a closing brace");
    let offset = ir.find(rest[0]).expect("the header is in the IR");
    let len: usize = rest[..=end].iter().map(|line| line.len() + 1).sum();
    &ir[offset..offset + len - 1]
}

fn items() -> Vec<MirItem> {
    let sink_call = |arg: MirExpr| MirExpr::Call {
        callee: "sink".to_string(),
        args: vec![arg],
        ty: Ty::Object,
    };
    vec![
        object_fn(
            "sink",
            vec![("x".to_string(), Ty::Object)],
            vec![MirStmt::Return(Some(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Object,
            }))],
        ),
        MirItem::Function {
            name: "nothing".to_string(),
            params: Vec::new(),
            return_ty: Ty::None,
            body: vec![MirStmt::Return(None)],
        },
        // A call argument: `sink(3)`.
        object_fn(
            "arg_int",
            Vec::new(),
            vec![MirStmt::Return(Some(sink_call(MirExpr::IntLiteral(3))))],
        ),
        // A call argument that is `None`: `sink(None)`.
        object_fn(
            "arg_none",
            Vec::new(),
            vec![MirStmt::Return(Some(sink_call(MirExpr::NoneLiteral)))],
        ),
        // A returned value: `return True`.
        object_fn(
            "ret_bool",
            Vec::new(),
            vec![MirStmt::Return(Some(MirExpr::BoolLiteral(true)))],
        ),
        // A returned `None`-typed call, evaluated for its effects:
        // `return nothing()`.
        object_fn(
            "ret_none_call",
            Vec::new(),
            vec![MirStmt::Return(Some(MirExpr::Call {
                callee: "nothing".to_string(),
                args: Vec::new(),
                ty: Ty::None,
            }))],
        ),
        // A binding `pycc_mir` boxed: `y: object = "s"; return y`.
        object_fn(
            "bind_str",
            Vec::new(),
            vec![
                MirStmt::Assign {
                    target: "y".to_string(),
                    value: MirExpr::ObjectBox(Box::new(MirExpr::StringLiteral("s".to_string()))),
                },
                MirStmt::Return(Some(MirExpr::Name {
                    name: "y".to_string(),
                    ty: Ty::Object,
                })),
            ],
        ),
    ]
}

#[test]
fn each_seam_packs_its_value_once_and_routes_a_null_to_the_failure_edge() {
    compile_ext_items_checking_ir("object_box_seams", items(), |ir| {
        for (name, packer) in [
            ("arg_int", "pycc_ext_obj_pack_int"),
            ("ret_bool", "pycc_ext_obj_pack_bool"),
            ("bind_str", "pycc_ext_obj_pack_str"),
        ] {
            let body = body_of(ir, name);
            assert_eq!(body.matches(packer).count(), 1, "{name}: {body}");
            assert!(body.contains("object_box_failed"), "{name}: {body}");
            assert!(!body.contains("DecRef"), "{name}: {body}");
        }
    });
}

#[test]
fn a_none_value_is_cpython_s_none_and_a_none_call_still_runs() {
    compile_ext_items_checking_ir("object_box_none", items(), |ir| {
        let arg = body_of(ir, "arg_none");
        assert!(arg.contains("call ptr @pycc_ext_obj_none()"), "{arg}");
        assert!(!arg.contains("pycc_ext_obj_pack_"), "{arg}");
        let ret = body_of(ir, "ret_none_call");
        assert!(ret.contains("@fnptr_nothing"), "{ret}");
        assert!(ret.contains("call ptr @pycc_ext_obj_none()"), "{ret}");
        assert!(!ret.contains("pycc_ext_obj_pack_"), "{ret}");
    });
}

#[test]
fn an_object_value_into_an_object_slot_is_not_boxed() {
    compile_ext_items_checking_ir("object_box_passthrough", items(), |ir| {
        let body = body_of(ir, "sink");
        assert!(!body.contains("pycc_ext_obj_pack_"), "{body}");
        assert!(!body.contains("object_box"), "{body}");
    });
}

#[test]
fn boxes_into_keys_on_an_object_slot_and_a_non_object_value() {
    use crate::object_box::boxes_into;
    let object = MirExpr::Name {
        name: "o".to_string(),
        ty: Ty::Object,
    };
    assert!(boxes_into(&MirExpr::IntLiteral(1), &Ty::Object));
    assert!(boxes_into(&MirExpr::NoneLiteral, &Ty::Object));
    assert!(!boxes_into(&object, &Ty::Object));
    assert!(!boxes_into(&MirExpr::IntLiteral(1), &Ty::Float));
}
