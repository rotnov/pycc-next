//! #1386 (Part 2 of #1367): a foreign-class annotation crossing the `--ext`
//! boundary. The annotation resolves to `Ty::Object` with the class name
//! erased, so a constructor whose `__init__` takes one is admitted, and its
//! generated `tp_init` hands the host's object through with no class check
//! (D-244's #1386 amendment; `docs/RUNTIME.md`'s `object` admissibility row).

use super::exports::constructible_module;
use super::*;

#[test]
fn an_init_taking_a_foreign_class_object_makes_the_class_constructible() {
    // `class Box: def __init__(self, f: Fraction, w: int)` -- the foreign
    // annotation is already `Ty::Object` by the time it reaches HIR.
    let mut hir = constructible_module("Box");
    hir.items[0] = init_func("Box", &[("f", Ty::Object), ("w", Ty::Int)], Ty::None);
    let exports = collect_exports(&hir).expect("a carriable signature");
    // The class is constructible, so its instance method is published
    // rather than silently omitted with the constructor.
    assert!(
        exports.iter().any(|export| export.name == "Box.area"),
        "{exports:?}"
    );
    let publications = collect_class_publications(&hir, &exports);
    let ctors = collect_constructors(&hir, &publications);
    assert_eq!(ctors.len(), 1);
    assert_eq!(ctors[0].params, vec![Ty::Object, Ty::Int]);
    let inc = generate_exports_inc("m", &exports, &[], &publications, &ctors, &[]);
    let start = inc
        .find("static int pycc_ext_tp_init_Box(")
        .expect("the generated tp_init");
    let tp_init = &inc[start..start + inc[start..].find("\n}\n").expect("its end")];
    // The object slot unpacks through the unchecked object helper: no
    // instance helper, no class name, no `isinstance` in the constructor.
    assert!(
        tp_init.contains(
            "if (pycc_ext_unpack_object(PyTuple_GetItem(args, 0), \"Box.__init__\", 0, &a0) != 0) {"
        ),
        "{tp_init}"
    );
    for absent in ["pycc_ext_unpack_instance", "isinstance", "IsInstance"] {
        assert!(!tp_init.contains(absent), "{absent}: {tp_init}");
    }
    assert!(tp_init.contains("    void * a0;\n"), "{tp_init}");
    assert!(
        tp_init.contains(
            "((void (*)(void *, void *, long long))fnptr_0m3_Box8___init__)(inst, a0, a1);"
        ),
        "{tp_init}"
    );
}
