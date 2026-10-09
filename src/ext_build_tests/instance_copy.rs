//! `copy.copy` of an `--ext` carrier (#1455): the per-class kind table the
//! shared `__copy__` consults, the row every published method table gains,
//! and the shim half that reads them. Lowered from real source through the
//! `--ext` frontend, so the slot types are the ones D-258 really assigns.

use super::*;

/// `source` resolved as an `--ext` module, with its carriers and companion.
fn ext_build(tag: &str, source: &str) -> (HirModule, Vec<ExtCarrierClass>, String) {
    let dir = pycc_scratch::ScratchDir::new(tag).expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, source).expect("write source");
    let module = crate::frontend::resolve_frontend_with(
        &src,
        Some("m"),
        crate::modules::RelativeImports::Project,
    )
    .unwrap_or_else(|_| panic!("the fixture must type-check in an ext build: {source}"));
    let exports = collect_exports(&module).expect("the module exports");
    let publications = collect_class_publications(&module, &exports);
    let ctors = collect_constructors(&module, &publications);
    let carriers = collect_carrier_classes(&module);
    let inc = generate_exports_inc("m", &exports, &[], &publications, &ctors, &carriers);
    (module, carriers, inc)
}

fn copy_of<'c>(carriers: &'c [ExtCarrierClass], class: &str) -> &'c CarrierCopy {
    &carriers
        .iter()
        .find(|carrier| carrier.class == class)
        .unwrap_or_else(|| panic!("`{class}` is a carrier: {carriers:?}"))
        .copy
}

fn kinds(text: &str) -> CarrierCopy {
    CarrierCopy::Kinds(text.to_string())
}

fn dunder(name: &'static str) -> CarrierCopy {
    CarrierCopy::Refused(CopyRefusal::Dunder(name))
}

/// lark's `ParseConf` shape -- an erased `Generic[T]` class with a
/// `T`-typed slot -- plus one slot of every admitted kind, a subclass that
/// adds a slot of its own, and the classes the dunder rule refuses or, for
/// `__deepcopy__`, deliberately does not.
const KINDS: &str = "from typing import Any, Generic, List, TypeVar\n\
    T = TypeVar(\"T\")\n\
    class Q:\n\
    \x20   def __init__(self, n: int) -> None:\n\
    \x20       self.n = n\n\
    class Conf(Generic[T]):\n\
    \x20   def __init__(self, n: int, s: str, o: Any, xs: List[int], t: T) -> None:\n\
    \x20       self.n = n\n\
    \x20       self.s = s\n\
    \x20       self.o = o\n\
    \x20       self.xs = xs\n\
    \x20       self.t = t\n\
    class P:\n\
    \x20   def __init__(self, r: float, b: bool, q: Q, s: str) -> None:\n\
    \x20       self.r = r\n\
    \x20       self.b = b\n\
    \x20       self.q = q\n\
    \x20       self.s = s\n\
    class Sub(P):\n\
    \x20   def __init__(self, k: int) -> None:\n\
    \x20       super().__init__(1.0, True, Q(1), \"x\")\n\
    \x20       self.k = k\n\
    class WithCopy:\n\
    \x20   def __init__(self, n: int) -> None:\n\
    \x20       self.n = n\n\
    \x20   def __copy__(self) -> \"WithCopy\":\n\
    \x20       return WithCopy(self.n + 1)\n\
    class Reducing:\n\
    \x20   def __init__(self, n: int) -> None:\n\
    \x20       self.n = n\n\
    \x20   def __reduce__(self) -> int:\n\
    \x20       return 0\n\
    class FromReducing(Reducing):\n\
    \x20   pass\n\
    class DeepOnly:\n\
    \x20   def __init__(self, n: int) -> None:\n\
    \x20       self.n = n\n\
    \x20   def __deepcopy__(self, memo: Any) -> \"DeepOnly\":\n\
    \x20       return DeepOnly(self.n)\n";

#[test]
fn each_slot_gets_its_kind_in_flat_layout_order() {
    let (_, carriers, _) = ext_build("copy_kinds", KINDS);
    assert_eq!(*copy_of(&carriers, "Q"), kinds("i"));
    // `o: Any` is the opaque object; `t: T` is erased to it too, so lark's
    // `ParseConf` shape is copyable.
    assert_eq!(*copy_of(&carriers, "Conf"), kinds("isowo"));
    assert_eq!(*copy_of(&carriers, "P"), kinds("wwws"));
    // The base's slots first, then the subclass's own.
    assert_eq!(*copy_of(&carriers, "Sub"), kinds("wwwsi"));
    assert_eq!(*copy_of(&carriers, "DeepOnly"), kinds("i"));
}

#[test]
fn a_class_binding_a_copy_protocol_name_is_refused_naming_it() {
    let (_, carriers, inc) = ext_build("copy_dunders", KINDS);
    assert_eq!(*copy_of(&carriers, "WithCopy"), dunder("__copy__"));
    assert_eq!(*copy_of(&carriers, "Reducing"), dunder("__reduce__"));
    // Inherited: the base's namespace takes part in the copy protocol.
    assert_eq!(*copy_of(&carriers, "FromReducing"), dunder("__reduce__"));
    assert!(
        inc.contains(
            "    if (len == 8 && memcmp(cls, \"WithCopy\", 8) == 0) {\n        \
             *refused = \"its compiled __copy__ is not published\";\n        return 0;\n    }\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    if (len == 4 && memcmp(cls, \"Conf\", 4) == 0) {\n        \
             *kinds = \"isowo\";\n        *nkinds = 5;\n        return 1;\n    }\n"
        ),
        "{inc}"
    );
}

#[test]
fn every_published_method_table_carries_the_shared_copy_row() {
    let (_, _, inc) = ext_build("copy_rows", KINDS);
    let row = "    {\"__copy__\", (PyCFunction)(void (*)(void))pycc_ext_instance_copy, \
               METH_NOARGS, NULL},\n    {NULL, NULL, 0, NULL},\n};\n";
    for class in ["Q", "P", "Sub", "WithCopy", "DeepOnly"] {
        let table = format!("static PyMethodDef pycc_ext_type_methods_{class}[] = {{\n");
        let at = inc.find(&table).unwrap_or_else(|| panic!("{class}: {inc}"));
        assert!(inc[at..].contains(row), "{class}: {inc}");
    }
    // The lookup follows the isinstance table in the companion.
    let isinstance = inc.find(CARRIER_CLASS_ISINSTANCE_DECL).unwrap();
    let copy = inc.find(CARRIER_CLASS_COPY_KINDS_DECL).unwrap();
    assert!(isinstance < copy, "{inc}");
}

/// A PEP 695 template is refused, and its `0gen_` specialization gets no
/// row: the specialization's instances carry the template's layout name.
#[test]
fn a_pep_695_template_is_refused_and_its_specializations_get_no_row() {
    let source = "class G[T]:\n\
                  \x20   def __init__(self, x: T) -> None:\n\
                  \x20       self.x = x\n\
                  def make() -> int:\n\
                  \x20   return G[int](1).x\n";
    let (_, carriers, inc) = ext_build("copy_generic", source);
    assert_eq!(
        *copy_of(&carriers, "G"),
        CarrierCopy::Refused(CopyRefusal::GenericLayout)
    );
    assert!(
        carriers
            .iter()
            .any(|carrier| carrier.class.starts_with("0gen_")),
        "{carriers:?}"
    );
    let table = &inc[inc.find(CARRIER_CLASS_COPY_KINDS_DECL).unwrap()..];
    assert!(!table.contains("0gen_"), "{table}");
    assert!(
        table.contains("*refused = \"its generic specializations share one instance layout\";"),
        "{table}"
    );
}

/// A slot whose type has no kind fails closed. Type-checked source never
/// yields one, so the module's own class definition is edited.
#[test]
fn a_slot_without_a_kind_refuses_the_class() {
    let (mut module, _, _) = ext_build("copy_uncopyable", KINDS);
    let p = module
        .class_defs
        .iter()
        .position(|(name, _)| name == "P")
        .unwrap();
    module.class_defs[p]
        .1
        .attrs
        .push(("odd".to_string(), Ty::Set(Box::new(Ty::Int))));
    let def = module.class_defs[p].1.clone();
    assert_eq!(
        carrier_copy(&module, &def),
        CarrierCopy::Refused(CopyRefusal::UncopyableSlot)
    );
    // A subclass inherits the uncopyable slot.
    let sub = module
        .class_defs
        .iter()
        .find(|(name, _)| name == "Sub")
        .unwrap()
        .1
        .clone();
    assert_eq!(
        carrier_copy(&module, &sub),
        CarrierCopy::Refused(CopyRefusal::UncopyableSlot)
    );
}

#[test]
fn copy_kind_maps_each_admitted_slot_type_and_refuses_the_rest() {
    let instance = Ty::Instance(Box::new("Q".to_string()));
    let cases = [
        (Ty::Str, Some(b's')),
        (Ty::Int, Some(b'i')),
        (Ty::Object, Some(b'o')),
        (Ty::Float, Some(b'w')),
        (Ty::Bool, Some(b'w')),
        (Ty::List(Box::new(Ty::Int)), Some(b'w')),
        (Ty::Dict(Box::new((Ty::Str, Ty::Int))), Some(b'w')),
        (instance, Some(b'w')),
        (Ty::Param(Box::new("T".to_string())), None),
        (Ty::Set(Box::new(Ty::Int)), None),
        (Ty::Tuple(Box::new(vec![Ty::Int])), None),
    ];
    for (ty, expected) in cases {
        assert_eq!(copy_kind(&ty), expected, "{ty:?}");
    }
}

#[test]
fn the_kind_lookup_renders_each_row_and_answers_minus_one_otherwise() {
    let carrier = |class: &str, copy: CarrierCopy| ExtCarrierClass {
        class: class.to_string(),
        mro: vec![class.to_string()],
        copy,
    };
    let c = carrier_class_copy_kinds_c(&[
        carrier("A", kinds("si")),
        carrier("Bad", CarrierCopy::Refused(CopyRefusal::UncopyableSlot)),
        carrier("Empty", kinds("")),
        carrier("0gen_G__T_int", kinds("i")),
    ]);
    let expected = format!(
        "{CARRIER_CLASS_COPY_KINDS_DECL}\n{{\n    \
         if (len == 1 && memcmp(cls, \"A\", 1) == 0) {{\n        \
         *kinds = \"si\";\n        *nkinds = 2;\n        return 1;\n    }}\n    \
         if (len == 3 && memcmp(cls, \"Bad\", 3) == 0) {{\n        \
         *refused = \"a slot has an uncopyable kind\";\n        return 0;\n    }}\n    \
         if (len == 5 && memcmp(cls, \"Empty\", 5) == 0) {{\n        \
         *kinds = \"\";\n        *nkinds = 0;\n        return 1;\n    }}\n    \
         return -1;\n}}\n\n"
    );
    assert_eq!(c, expected);
    // No row at all -- an empty module, or only specializations -- still
    // defines the function, with every parameter used.
    let empty = format!(
        "{CARRIER_CLASS_COPY_KINDS_DECL}\n{{\n    (void)cls;\n    (void)len;\n    \
         (void)kinds;\n    (void)nkinds;\n    (void)refused;\n    return -1;\n}}\n\n"
    );
    assert_eq!(carrier_class_copy_kinds_c(&[]), empty);
    assert_eq!(
        carrier_class_copy_kinds_c(&[carrier("0gen_G__T_int", kinds("i"))]),
        empty
    );
}

#[test]
fn every_copy_protocol_dunder_has_its_own_refusal_clause() {
    for name in COPY_PROTOCOL_DUNDERS {
        assert_eq!(
            CopyRefusal::Dunder(name).reason(),
            format!("its compiled {name} is not published")
        );
    }
    assert!(!COPY_PROTOCOL_DUNDERS.contains(&"__deepcopy__"));
}

/// The shim half: the forward declaration the generated rows need, the
/// definition after the companion include, the on-demand method table and
/// its slot, and the two runtime entry points it calls.
#[test]
fn the_shim_installs_the_shared_copy_on_every_carrier_type() {
    let shim = shim_c();
    for needle in [
        "extern void *pycc_rt_ext_instance_copy(void *instance, const char *kinds, size_t kinds_len);",
        "extern long long pycc_rt_instance_get_slot(void *instance, long long slot);",
        "static PyObject *pycc_ext_instance_copy(PyObject *self, PyObject *unused);",
        "static PyMethodDef pycc_ext_carrier_methods[] = {\n    \
         {\"__copy__\", (PyCFunction)(void (*)(void))pycc_ext_instance_copy, METH_NOARGS, NULL},\n",
        "static PyType_Slot pycc_ext_carrier_slots[] = {\n    \
         {Py_tp_dealloc, pycc_ext_instance_dealloc},\n    \
         {Py_tp_methods, pycc_ext_carrier_methods},\n",
    ] {
        assert!(shim.contains(needle), "missing {needle:?}");
    }
    let declared = shim
        .find("static PyObject *pycc_ext_instance_copy(PyObject *self, PyObject *unused);")
        .unwrap();
    let include = shim.find("#include \"pycc_ext_exports.inc\"").unwrap();
    let defined = shim
        .find("static PyObject *pycc_ext_instance_copy(PyObject *self, PyObject *unused)\n{")
        .unwrap();
    assert!(declared < include && include < defined);
    let body = &shim[defined..];
    let body = &body[..body.find("\n}\n").unwrap()];
    for needle in [
        "found = pycc_ext_carrier_class_copy_kinds(cls, len, &kinds, &nkinds, &refused);",
        "\"cannot copy '%U' object: %s\"",
        "\"pycc: copy kind table does not match the instance layout\"",
        "clone = pycc_rt_ext_instance_copy(inst, kinds, nkinds);",
        "pycc_rt_ext_instance_set_carrier(clone, carrier);",
        "(allocfunc)PyType_GetSlot(tp, Py_tp_alloc)",
        "name = PyType_GetName(tp);",
    ] {
        assert!(body.contains(needle), "missing {needle:?} in:\n{body}");
    }
    // The declaration the generated call is checked against.
    assert!(CARRIER_CLASS_COPY_KINDS_DECL.contains("pycc_ext_carrier_class_copy_kinds("));
}
