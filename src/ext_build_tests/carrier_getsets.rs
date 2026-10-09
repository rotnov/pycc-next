//! #1448: every carrier type carries its field descriptors, not only a
//! constructible published class's type object. A non-constructible
//! published type installs `Py_tp_getset` on its own slot array, and a
//! class that publishes nothing gets a hidden carrier type that carries the
//! table (alongside #1427's comparison slots when it has both), while a
//! class with nothing to describe still crosses on the shim's on-demand
//! type.

use super::*;

/// `source` lowered through the `--ext` frontend.
fn lower(tag: &str, source: &str) -> HirModule {
    let dir = pycc_scratch::ScratchDir::new(tag).expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, source).expect("write source");
    crate::frontend::resolve_frontend_with(
        &src,
        Some("m"),
        crate::modules::RelativeImports::Project,
    )
    .unwrap_or_else(|_| panic!("the fixture must type-check in an ext build: {source}"))
}

/// `module`'s descriptor tables and the companion the `--ext` build renders
/// for it, slots included.
fn tables_and_inc(module: &HirModule) -> (Vec<ExtClassGetsets>, String) {
    let exports = collect_exports(module).expect("the module exports");
    let publications = collect_class_publications(module, &exports);
    let ctors = collect_constructors(module, &publications);
    let carriers = collect_carrier_classes(module);
    let getsets = collect_carrier_getsets(module, &carriers);
    let slots =
        collect_slot_dunders(module, &carriers, SlotArtifact::Ext).expect("installable slots");
    let inc = generate_exports_inc_with_slots(
        "m",
        &exports,
        &[],
        &publications,
        &ctors,
        &carriers,
        &getsets,
        &slots,
    );
    (getsets, inc)
}

fn names(tables: &[ExtClassGetsets], class: &str) -> Vec<String> {
    tables
        .iter()
        .find(|table| table.class == class)
        .unwrap_or_else(|| panic!("`{class}` has a descriptor table"))
        .getsets
        .iter()
        .map(|getset| getset.name().to_string())
        .collect()
}

/// One class of each kind #1448 covers, next to a constructible one whose
/// type is unchanged: `NC` is published -- its method is reachable through
/// the constructible `NCSub` -- but its `dict` parameter keeps it from
/// being constructible, `Hid` publishes nothing (its `tuple`
/// parameter is uncarriable and it has no public method), `_Priv` is
/// private and compares, and `Bare` has no carriable field at all.
const KINDS: &str = "from typing import Any, Dict, List, Tuple\n\
    class Pub:\n\
    \x20   def __init__(self, n: int) -> None:\n\
    \x20       self.n = n\n\
    class NC:\n\
    \x20   def __init__(self, d: Dict[str, int], n: int) -> None:\n\
    \x20       self.n = n\n\
    \x20       self.s = 'nc'\n\
    \x20   def get(self) -> int:\n\
    \x20       return self.n\n\
    \x20   @property\n\
    \x20   def half(self) -> float:\n\
    \x20       return self.n / 2\n\
    class NCSub(NC):\n\
    \x20   def __init__(self, n: int) -> None:\n\
    \x20       self.n = n\n\
    \x20       self.s = 'sub'\n\
    class Hid:\n\
    \x20   x: int\n\
    \x20   p: Pub\n\
    \x20   def __init__(self, t: Tuple[int, int]) -> None:\n\
    \x20       self.x = t[0]\n\
    \x20       self.p = Pub(t[1])\n\
    class _Priv:\n\
    \x20   def __init__(self, n: int) -> None:\n\
    \x20       self.n = n\n\
    \x20   def __eq__(self, other: Any) -> Any:\n\
    \x20       return NotImplemented\n\
    class Bare:\n\
    \x20   xs: List[int]\n\
    \x20   def __init__(self, t: Tuple[int, int]) -> None:\n\
    \x20       self.xs = [t[0]]\n\
    def make_nc(n: int) -> NC:\n\
    \x20   return NC({'k': n}, n)\n\
    def make_hid(a: int, b: int) -> Hid:\n\
    \x20   return Hid((a, b))\n\
    def make_priv(n: int) -> _Priv:\n\
    \x20   return _Priv(n)\n\
    def make_bare(n: int) -> Bare:\n\
    \x20   return Bare((n, n))\n";

#[test]
fn every_carrier_class_with_a_carriable_field_gets_a_table() {
    let module = lower("1448_kinds", KINDS);
    let (tables, _) = tables_and_inc(&module);
    assert_eq!(
        tables
            .iter()
            .map(|table| table.class.as_str())
            .collect::<Vec<_>>(),
        vec!["Pub", "NC", "NCSub", "Hid", "_Priv"],
        "`Bare` has nothing to describe"
    );
    assert_eq!(names(&tables, "NC"), ["n", "s", "half"]);
    assert_eq!(names(&tables, "Hid"), ["x", "p"]);
    assert_eq!(names(&tables, "_Priv"), ["n"]);
}

#[test]
fn a_non_constructible_published_type_installs_its_table() {
    let module = lower("1448_nc_c", KINDS);
    let (_, inc) = tables_and_inc(&module);
    let nc = &inc[inc
        .find("static PyType_Slot pycc_ext_type_slots_NC[]")
        .expect("`NC` is published")..];
    let nc = &nc[..nc.find("};").expect("the slot array ends")];
    assert!(
        nc.contains("{Py_tp_getset, pycc_ext_type_getset_NC}"),
        "{nc}"
    );
    assert!(!nc.contains("Py_tp_init"), "{nc}");
    assert!(inc.contains("static PyGetSetDef pycc_ext_type_getset_NC[]"));
    // Its descriptors are not duplicated onto a hidden type.
    assert!(!inc.contains("pycc_ext_carrier_spec_NC"), "{inc}");
}

#[test]
fn an_unpublished_class_s_table_rides_a_hidden_carrier_type() {
    let module = lower("1448_hidden_c", KINDS);
    let (_, inc) = tables_and_inc(&module);
    let hid = &inc[inc
        .find("static PyType_Slot pycc_ext_carrier_slots_Hid[]")
        .expect("`Hid` gets a hidden type")..];
    let hid = &hid[..hid.find("};").expect("the slot array ends")];
    assert_eq!(
        hid,
        "static PyType_Slot pycc_ext_carrier_slots_Hid[] = {\n    \
         {Py_tp_dealloc, pycc_ext_instance_dealloc},\n    \
         {Py_tp_methods, pycc_ext_carrier_methods_Hid},\n    \
         {Py_tp_getset, pycc_ext_type_getset_Hid},\n    {0, NULL},\n"
    );
    assert!(inc.contains("pycc_ext_carrier_register(\"Hid\", type)"));
    assert!(inc.contains("static PyGetSetDef pycc_ext_type_getset_Hid[]"));
    // A table-only hidden type has no unhashable marker to swap.
    assert!(!inc.contains("pycc_ext_unhashable_slots(pycc_ext_carrier_slots_Hid)"));
}

#[test]
fn a_class_with_both_a_table_and_slots_gets_one_hidden_type_with_both() {
    let module = lower("1448_both_c", KINDS);
    let (_, inc) = tables_and_inc(&module);
    assert_eq!(
        inc.matches("static PyType_Spec pycc_ext_carrier_spec__Priv =")
            .count(),
        1,
        "{inc}"
    );
    assert_eq!(
        inc.matches("pycc_ext_carrier_register(\"_Priv\", type)")
            .count(),
        1
    );
    let slots = &inc[inc
        .find("static PyType_Slot pycc_ext_carrier_slots__Priv[]")
        .expect("`_Priv` gets a hidden type")..];
    let slots = &slots[..slots.find("};").expect("the slot array ends")];
    assert!(
        slots.contains("{Py_tp_getset, pycc_ext_type_getset__Priv},\n"),
        "{slots}"
    );
    assert!(
        slots.contains("{Py_tp_richcompare, pycc_ext_richcompare__Priv},\n"),
        "{slots}"
    );
    // `__eq__` without `__hash__` makes the type unhashable, as in CPython.
    assert!(inc.contains("pycc_ext_unhashable_slots(pycc_ext_carrier_slots__Priv)"));
}

#[test]
fn a_class_with_nothing_to_describe_keeps_the_on_demand_type() {
    let module = lower("1448_bare_c", KINDS);
    let (_, inc) = tables_and_inc(&module);
    assert!(!inc.contains("pycc_ext_carrier_spec_Bare"), "{inc}");
    assert!(!inc.contains("pycc_ext_carrier_register(\"Bare\""), "{inc}");
}

/// A class no instance's run-time class can be (abstract, `Protocol`) and a
/// monomorphized `0gen_` specialization get no table, and neither does a
/// class with a PEP 695 generic in its MRO, whose specializations all cross
/// under the template's name with different layouts -- except a
/// constructible published template (`Gen`), which keeps the table #1442
/// gave it; the private `_PGen` is constructible but unpublished, so it is
/// held to the exclusion.
#[test]
fn abstract_protocol_specialized_and_pep_695_generic_classes_get_no_table() {
    let source = "class A:\n\
        \x20   def __init__(self, n: int) -> None:\n\
        \x20       self.n = n\n\
        class P:\n\
        \x20   def __init__(self, n: int) -> None:\n\
        \x20       self.n = n\n\
        class Box[T]:\n\
        \x20   def __init__(self, v: T, n: int) -> None:\n\
        \x20       self.v = v\n\
        \x20       self.n = n\n\
        class Kept:\n\
        \x20   def __init__(self, n: int) -> None:\n\
        \x20       self.n = n\n\
        class Gen[T]:\n\
        \x20   def __init__(self, n: int) -> None:\n\
        \x20       self.n = n\n\
        class _PGen[T]:\n\
        \x20   def __init__(self, n: int) -> None:\n\
        \x20       self.n = n\n\
        def mk() -> int:\n\
        \x20   b = Box[int](3, 4)\n\
        \x20   return b.n + Gen[str](1).n + _PGen[str](2).n\n";
    let mut module = lower("1448_skipped", source);
    for (class, def) in &mut module.class_defs {
        match class.as_str() {
            "A" => def.is_abstract = true,
            "P" => def.is_protocol = true,
            _ => {}
        }
    }
    let carriers = collect_carrier_classes(&module);
    let carried: Vec<&str> = carriers.iter().map(|c| c.class.as_str()).collect();
    assert!(
        carried.iter().any(|class| class.starts_with("0gen_Box")),
        "the fixture specializes `Box`: {carried:?}"
    );
    assert!(carried.contains(&"Box"), "{carried:?}");
    let tables = collect_carrier_getsets(&module, &carriers);
    assert_eq!(
        tables
            .iter()
            .map(|table| table.class.as_str())
            .collect::<Vec<_>>(),
        vec!["Kept", "Gen"]
    );
}
