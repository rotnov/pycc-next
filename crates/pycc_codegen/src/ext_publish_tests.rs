//! Unit tests for `ext_publish`: the pure publication plan, and the IR the
//! emitter leaves in the module entry point (#1199).

use super::*;
use crate::{CompileOptions, EXT_MODULE_EXEC_SYMBOL, compile_to_object_with_observer};
use inkwell::values::AnyValue;
use pycc_mir::{HirClassDef, Ty};

fn class(name: &str, mro: &[&str]) -> (String, HirClassDef) {
    let def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: name.to_string(),
        bases: Vec::new(),
        mro: mro.iter().map(|m| (*m).to_string()).collect(),
        attrs: Vec::new(),
        methods: Vec::new(),
        type_param: None,
        properties: Vec::new(),
        static_methods: Vec::new(),
        class_methods: Vec::new(),
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: Vec::new(),
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    };
    (name.to_string(), def)
}

/// A method-bearing class: `class()` with `methods` recorded as its own,
/// which the copy pass needs to recognise an inherited copy.
fn class_with(name: &str, mro: &[&str], methods: &[&str]) -> (String, HirClassDef) {
    let (name, mut def) = class(name, mro);
    def.methods = methods
        .iter()
        .map(|m| ((*m).to_string(), format!("{name}.{m}")))
        .collect();
    (name, def)
}

fn function(name: &str) -> MirItem {
    MirItem::Function {
        name: name.to_string(),
        params: Vec::new(),
        return_ty: Ty::None,
        body: Vec::new(),
    }
}

fn plan(items: Vec<MirItem>, class_defs: Vec<(String, HirClassDef)>, ext: bool) -> PublishPlan {
    let mir = MirModule { items, class_defs };
    let copies = CopySlots::new(&mir);
    PublishPlan::new(&mir, &copies, ext)
}

fn names(plan: &PublishPlan, ordinal: usize) -> Vec<&str> {
    plan.names_at(ordinal).iter().map(String::as_str).collect()
}

/// A function is published at its own ordinal, and a redefined one only at
/// its last: every definition shares the last one's wrapper metadata, so an
/// earlier one must never be callable from the host.
#[test]
fn a_redefined_function_is_published_only_at_its_last_definition() {
    let p = plan(
        vec![function("f"), function("g"), function("f"), function("f")],
        Vec::new(),
        true,
    );
    assert!(names(&p, 0).is_empty());
    assert_eq!(names(&p, 1), ["g"]);
    assert!(names(&p, 2).is_empty());
    assert_eq!(names(&p, 3), ["f"]);
    assert!(names(&p, 4).is_empty());
}

/// Generated specializations, private-by-convention names the export
/// filter refuses, and the PEP 562 hooks the shim adds after the body are
/// never published per definition.
#[test]
fn hooks_and_unexportable_names_are_not_published() {
    let p = plan(
        vec![
            function("__getattr__"),
            function("__dir__"),
            function("0gen_f_int"),
        ],
        Vec::new(),
        true,
    );
    for ordinal in 0..3 {
        assert!(names(&p, ordinal).is_empty(), "ordinal {ordinal}");
    }
}

/// A class is published after the last of its own items (the end of its
/// class statement), not after the first.
#[test]
fn a_class_is_published_after_its_last_own_item() {
    let p = plan(
        vec![
            function("helper"),
            function("C.__init__"),
            function("C.m"),
            function("after"),
        ],
        vec![class("C", &["C"])],
        true,
    );
    assert!(names(&p, 1).is_empty());
    assert_eq!(names(&p, 2), ["C"]);
    assert_eq!(names(&p, 3), ["after"]);
}

/// `class E(Base): pass` owns no item; its only copy (`E.get`) is bound
/// with `Base.get`, so `E` is published there: the maximum over its MRO,
/// never at the copy's own appended ordinal. A seeded class with no item
/// anywhere in its MRO is not published at all.
#[test]
fn a_class_without_own_items_is_published_with_its_bases_methods() {
    let p = plan(
        vec![
            function("Base.get"),
            function("Base.__init__"),
            function("make"),
            function("E.get"),
        ],
        vec![
            class_with("Base", &["Base"], &["get", "__init__"]),
            class("E", &["E", "Base"]),
            class("ValueError", &["ValueError", "Exception"]),
        ],
        true,
    );
    assert_eq!(names(&p, 1), ["Base", "E"]);
    assert_eq!(names(&p, 2), ["make"]);
    assert!(names(&p, 3).is_empty(), "a copy publishes nothing");
}

/// A native build has no host to publish to.
#[test]
fn a_native_build_has_an_empty_plan() {
    let p = plan(vec![function("f")], vec![class("C", &["C"])], false);
    assert!(names(&p, 0).is_empty());
}

/// The `ext` entry point's LLVM text after compiling `items`, compiled to
/// an object so LLVM's verifier runs over the appended blocks.
fn entry_ir(label: &str, items: Vec<MirItem>) -> String {
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    let mut ir = String::new();
    let mut observer = |module: &inkwell::module::Module<'_>, _: Option<&'static str>| {
        if let Some(entry) = module.get_function(EXT_MODULE_EXEC_SYMBOL) {
            ir = crate::llvm_string_to_owned(entry.print_to_string());
        }
    };
    compile_to_object_with_observer(
        &MirModule {
            items,
            ..Default::default()
        },
        &dir.join(format!("{label}.o")),
        &CompileOptions {
            ext: true,
            ..CompileOptions::default()
        },
        Some(&mut observer),
    )
    .expect("ext codegen should succeed");
    assert!(!ir.is_empty(), "no {EXT_MODULE_EXEC_SYMBOL} was emitted");
    ir
}

/// Each published name is one `pycc_ext_publish` call after the slot
/// store, branching to an `EXT_MODULE_EXEC_FAILED` return on a negative
/// status. Two names exercise both the declaring and the reusing arm of
/// the shim function's declaration.
#[test]
fn each_definition_stores_its_slot_then_publishes_its_name() {
    let ir = entry_ir(
        "ext_publish_calls",
        vec![function("first"), function("second")],
    );
    let store = ir
        .find("store ptr @pyfn_first, ptr @fnptr_first")
        .unwrap_or_else(|| panic!("the slot store is emitted:\n{ir}"));
    let call = ir
        .find(&format!(
            "call i32 @{EXT_PUBLISH_SYMBOL}(ptr @pycc_publish_first)"
        ))
        .expect("the publish call is emitted");
    assert!(store < call, "publication follows the slot store:\n{ir}");
    assert!(
        ir.contains(&format!(
            "call i32 @{EXT_PUBLISH_SYMBOL}(ptr @pycc_publish_second)"
        )),
        "{ir}"
    );
    assert_eq!(ir.matches("icmp slt i32 %publish").count(), 2, "{ir}");
    // One failed return per publication, beside the body's own.
    let failed_returns = ir
        .matches(&format!("ret i64 {EXT_MODULE_EXEC_FAILED}"))
        .count();
    assert!(failed_returns >= 3, "{ir}");
}

/// A three-level chain `A` / `B(A): pass` / `C(B): pass`: the two classes
/// without own items are published with `A`'s last method, and the copy
/// items appended for them publish nothing.
#[test]
fn an_item_less_chain_is_published_with_its_root_class() {
    let p = plan(
        vec![
            function("A.m"),
            function("A.n"),
            function("tail"),
            function("B.m"),
            function("B.n"),
            function("C.m"),
            function("C.n"),
        ],
        vec![
            class_with("A", &["A"], &["m", "n"]),
            class("B", &["B", "A"]),
            class("C", &["C", "B", "A"]),
        ],
        true,
    );
    assert_eq!(names(&p, 1), ["A", "B", "C"]);
    assert_eq!(names(&p, 2), ["tail"]);
    for ordinal in 3..7 {
        assert!(names(&p, ordinal).is_empty(), "ordinal {ordinal}");
    }
}

/// A derived class with its own `__init__` is published at its own class
/// statement, after its base's, even though copies of other classes' items
/// sit between and after them.
#[test]
fn a_class_with_own_items_ignores_interleaved_copies() {
    let p = plan(
        vec![
            function("Base.get"),
            function("D.__init__"),
            function("Other.run"),
            function("D.get"),
        ],
        vec![
            class_with("Base", &["Base"], &["get"]),
            class_with("D", &["D", "Base"], &["__init__"]),
            class_with("Other", &["Other"], &["run"]),
        ],
        true,
    );
    assert_eq!(names(&p, 0), ["Base"]);
    assert_eq!(names(&p, 1), ["D"]);
    assert_eq!(names(&p, 2), ["Other"]);
    assert!(names(&p, 3).is_empty(), "D.get is a copy");
}
