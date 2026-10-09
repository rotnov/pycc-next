//! The `Py_tp_getset` descriptors a carrier type carries (#1442, every
//! carrier type since #1448), and their slot setters (Part 1 of #1443): which slots and
//! properties get one, and the generated C each renders to. Lowered from real source through the `--ext`
//! frontend, so the slot types are the ones D-258 really assigns (an `Any`
//! or object-carrying container field is the opaque object).

use super::super::getset::PropertySetter;
use super::*;

/// The descriptor tables of `source` built as an `--ext` module, plus the
/// generated companion.
fn ext_build(tag: &str, source: &str) -> (Vec<ExtClassGetsets>, String) {
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
    let getsets = collect_carrier_getsets(&module, &carriers);
    let inc = generate_exports_inc_with_slots(
        "m",
        &exports,
        &[],
        &publications,
        &ctors,
        &carriers,
        &getsets,
        &[],
    );
    (getsets, inc)
}

/// `class`'s descriptors: empty when it has no table, which is exactly
/// when it has nothing to describe (or is a class #1448 skips).
fn getsets_of<'c>(tables: &'c [ExtClassGetsets], class: &str) -> &'c [ExtGetset] {
    tables
        .iter()
        .find(|table| table.class == class)
        .map_or(&[], |table| table.getsets.as_slice())
}

fn slot(name: &str, index: usize, ty: Ty) -> ExtGetset {
    ExtGetset::Slot {
        name: name.to_string(),
        index,
        ty,
    }
}

const SCALARS: &str = "from typing import Any, List\n\
    class P:\n\
    \x20   xs: List[int]\n\
    \x20   def __init__(self, n: int, r: float, b: bool, s: str, o: Any) -> None:\n\
    \x20       self.n = n\n\
    \x20       self.r = r\n\
    \x20       self.xs = [1]\n\
    \x20       self.b = b\n\
    \x20       self.s = s\n\
    \x20       self.o = o\n\
    \x20   def get(self) -> int:\n\
    \x20       return self.n\n";

/// Every slot whose word the boundary packs gets a descriptor at its own
/// slot index; a `list[int]` slot, which no packer carries, is skipped
/// without a `C0003` and without disturbing the later slots' indices.
#[test]
fn every_carriable_slot_gets_a_descriptor_at_its_slot_index() {
    let (ctors, _) = ext_build("1442_scalars", SCALARS);
    assert_eq!(
        getsets_of(&ctors, "P"),
        &[
            slot("n", 0, Ty::Int),
            slot("r", 1, Ty::Float),
            slot("b", 3, Ty::Bool),
            slot("s", 4, Ty::Str),
            slot("o", 5, Ty::Object),
        ]
    );
}

/// Each slot type is packed by the packer its word needs, the two packers
/// that discharge a reference are handed one taken first, and the table is
/// installed as `Py_tp_getset` with each slot's setter.
#[test]
fn each_slot_getter_packs_its_word_and_the_table_is_installed() {
    let (_, inc) = ext_build("1442_scalars_c", SCALARS);
    let getter = |name: &str, index: usize, pack: &str| {
        format!(
            "static PyObject *pycc_ext_get_1_P_1_{name}(PyObject *self, void *closure)\n{{\n    \
             void *inst = ((PyccExtInstance *)self)->inst;\n    long long word;\n{local}    \
             (void)closure;\n    if (inst == NULL) {{\n        \
             PyErr_SetString(PyExc_AttributeError, \"'P' object has no attribute '{name}'\");\n        \
             return NULL;\n    }}\n    word = pycc_rt_instance_get_slot_checked(inst, {index});\n    \
             if (pycc_rt_ext_pending_type() >= 0) {{\n        pycc_ext_raise_pending();\n        \
             return NULL;\n    }}\n{pack}}}\n\n",
            local = if name == "r" {
                "    double value;\n"
            } else {
                ""
            },
        )
    };
    for expected in [
        getter(
            "n",
            0,
            "    pycc_rt_bigint_retain(word);\n    return pycc_ext_pack_int(\"P.n\", word);\n",
        ),
        getter(
            "r",
            1,
            "    memcpy(&value, &word, sizeof value);\n    return pycc_ext_pack_float(value);\n",
        ),
        getter("b", 3, "    return pycc_ext_pack_bool((char)word);\n"),
        getter(
            "s",
            4,
            "    pycc_rt_str_incref((void *)(intptr_t)word);\n    \
             return pycc_ext_pack_str((void *)(intptr_t)word);\n",
        ),
        getter(
            "o",
            5,
            "    return pycc_ext_pack_object((void *)(intptr_t)word);\n",
        ),
    ] {
        assert!(inc.contains(&expected), "missing:\n{expected}\nin:\n{inc}");
    }
    assert!(
        inc.contains(
            "static PyGetSetDef pycc_ext_type_getset_P[] = {\n    \
             {\"n\", pycc_ext_get_1_P_1_n, pycc_ext_set_1_P_1_n, NULL, NULL},\n    \
             {\"r\", pycc_ext_get_1_P_1_r, pycc_ext_set_1_P_1_r, NULL, NULL},\n    \
             {\"b\", pycc_ext_get_1_P_1_b, pycc_ext_set_1_P_1_b, NULL, NULL},\n    \
             {\"s\", pycc_ext_get_1_P_1_s, pycc_ext_set_1_P_1_s, NULL, NULL},\n    \
             {\"o\", pycc_ext_get_1_P_1_o, pycc_ext_set_1_P_1_o, NULL, NULL},\n    \
             {NULL, NULL, NULL, NULL, NULL},\n};\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    {Py_tp_dealloc, pycc_ext_instance_dealloc},\n    \
             {Py_tp_getset, pycc_ext_type_getset_P},\n    {0, NULL},\n"
        ),
        "{inc}"
    );
    assert!(!inc.contains("pycc_ext_get_1_P_2_xs"), "{inc}");
    assert!(!inc.contains("pycc_ext_set_1_P_2_xs"), "{inc}");
}

/// Part 1 of #1443: the whole setter of an `int` slot -- the never-initialized
/// guard split by operation, the `del` through the runtime with the pending
/// `AttributeError` raised, and the store through the parameter row's unpack
/// helper with the slot's kind byte.
#[test]
fn an_int_slot_setter_unpacks_by_the_parameter_row_and_stores_by_kind() {
    let (_, inc) = ext_build("1443_setter_int", SCALARS);
    let setter = "static int pycc_ext_set_1_P_1_n(PyObject *self, PyObject *value, void *closure)\n{\n    \
                  void *inst = ((PyccExtInstance *)self)->inst;\n    long long v;\n    long long word;\n    \
                  (void)closure;\n    if (inst == NULL) {\n        if (value == NULL) {\n            \
                  PyErr_SetString(PyExc_AttributeError, \"'P' object has no attribute 'n'\");\n        \
                  } else {\n            \
                  PyErr_SetString(PyExc_AttributeError, \
                  \"cannot set 'n' on a 'P' object whose __init__ never ran\");\n        }\n        \
                  return -1;\n    }\n    if (value == NULL) {\n        \
                  if (pycc_rt_ext_instance_delete_slot(inst, 0, 'i') != 0) {\n            \
                  pycc_ext_raise_pending();\n            return -1;\n        }\n        \
                  return 0;\n    }\n    \
                  if (pycc_ext_unpack_int(value, \"P.n\", 0, &v) != 0) {\n        return -1;\n    }\n    \
                  word = v;\n    pycc_rt_ext_instance_store_slot(inst, 0, 'i', word);\n    \
                  return 0;\n}\n\n";
    assert!(inc.contains(setter), "missing:\n{setter}\nin:\n{inc}");
}

/// Each other slot type's setter: its unpack helper, the conversion of the
/// unpacked local to the slot word `scalar_to_slot_word` writes, and its
/// kind byte -- `s` and `o` for the two reference-holding words, `w` for a
/// plain one.
#[test]
fn each_slot_setter_converts_its_type_to_the_compiled_slot_word() {
    let (_, inc) = ext_build("1443_setter_types", SCALARS);
    for (name, index, local, unpack, to_word, kind) in [
        (
            "r",
            1,
            "double v;",
            "pycc_ext_unpack_float(value, \"P.r\", 0, &v)",
            "memcpy(&word, &v, sizeof word);",
            'w',
        ),
        (
            "b",
            3,
            "char v;",
            "pycc_ext_unpack_bool(value, \"P.b\", 0, &v)",
            "word = (unsigned char)v;",
            'w',
        ),
        (
            "s",
            4,
            "void *v;",
            "pycc_ext_unpack_str(value, \"P.s\", 0, &v)",
            "word = (long long)(intptr_t)v;",
            's',
        ),
        (
            "o",
            5,
            "void *v;",
            "pycc_ext_unpack_object(value, \"P.o\", 0, &v)",
            "word = (long long)(intptr_t)v;",
            'o',
        ),
    ] {
        let symbol = format!("static int pycc_ext_set_1_P_1_{name}(");
        let start = inc
            .find(&symbol)
            .unwrap_or_else(|| panic!("{symbol} in:\n{inc}"));
        let body = &inc[start..start + inc[start..].find("\n}\n").expect("the setter ends")];
        for expected in [
            format!("    {local}\n"),
            format!("pycc_rt_ext_instance_delete_slot(inst, {index}, '{kind}')"),
            format!("    if ({unpack} != 0) {{\n"),
            format!(
                "    {to_word}\n    pycc_rt_ext_instance_store_slot(inst, {index}, '{kind}', word);\n"
            ),
        ] {
            assert!(
                body.contains(&expected),
                "missing:\n{expected}\nin:\n{body}"
            );
        }
    }
}

/// #1458: a getter-only property's row names a setter that raises
/// CPython's `has no setter` for a store and `has no deleter` for a `del`,
/// without looking at the instance, which CPython never consults for either.
#[test]
fn a_getter_only_property_refuses_with_cpython_s_wording() {
    let (_, inc) = ext_build("1458_property_row", PROPERTIES);
    assert!(
        inc.contains(
            "static PyGetSetDef pycc_ext_type_getset_Base[] = {\n    \
             {\"stack\", pycc_ext_get_4_Base_5_stack, pycc_ext_set_4_Base_5_stack, NULL, NULL},\n    \
             {\"top\", pycc_ext_get_4_Base_3_top, pycc_ext_set_4_Base_3_top, NULL, NULL},\n    \
             {\"shadowed\", pycc_ext_get_4_Base_8_shadowed, pycc_ext_set_4_Base_8_shadowed, NULL, NULL},\n"
        ),
        "{inc}"
    );
    let refusal = "static int pycc_ext_set_4_Base_3_top(PyObject *self, PyObject *value, void *closure)\n{\n    \
                   (void)self;\n    (void)closure;\n    if (value == NULL) {\n        \
                   PyErr_SetString(PyExc_AttributeError, \"property 'top' of 'Base' object has no deleter\");\n        \
                   return -1;\n    }\n    \
                   PyErr_SetString(PyExc_AttributeError, \"property 'top' of 'Base' object has no setter\");\n        \
                   return -1;\n}\n\n";
    assert!(inc.contains(refusal), "missing:\n{refusal}\nin:\n{inc}");
}

/// #1458's shape: a property with a compiled setter of each kind the
/// boundary decides on. `Derived` inherits `n` and `p`; `p`'s setter calls
/// an overridden method, so `Derived` gets a receiver-exact copy of it
/// (D-254), while `n`'s inherited setter needs none.
const SETTERS: &str = "from typing import List, Tuple\n\
    class Leaf:\n\
    \x20   def __init__(self, k: int) -> None:\n\
    \x20       self.k = k\n\
    class Base:\n\
    \x20   def __init__(self, n: int) -> None:\n\
    \x20       self._n = n\n\
    \x20       self._xs: List[int] = []\n\
    \x20   def scale(self) -> int:\n\
    \x20       return 2\n\
    \x20   @property\n\
    \x20   def n(self) -> int:\n\
    \x20       return self._n\n\
    \x20   @n.setter\n\
    \x20   def n(self, v: int) -> None:\n\
    \x20       self._n = v\n\
    \x20   @property\n\
    \x20   def p(self) -> int:\n\
    \x20       return self._n\n\
    \x20   @p.setter\n\
    \x20   def p(self, v: int) -> None:\n\
    \x20       self._n = v * self.scale()\n\
    \x20   @property\n\
    \x20   def leaf(self) -> Leaf:\n\
    \x20       return Leaf(self._n)\n\
    \x20   @leaf.setter\n\
    \x20   def leaf(self, v: Leaf) -> None:\n\
    \x20       self._n = v.k\n\
    \x20   @property\n\
    \x20   def count(self) -> int:\n\
    \x20       return len(self._xs)\n\
    \x20   @count.setter\n\
    \x20   def count(self, v: List[int]) -> None:\n\
    \x20       self._xs = v\n\
    \x20   @property\n\
    \x20   def pair(self) -> int:\n\
    \x20       return self._n\n\
    \x20   @pair.setter\n\
    \x20   def pair(self, v: Tuple[int, int]) -> None:\n\
    \x20       self._n = v[0]\n\
    \x20   @property\n\
    \x20   def ret(self) -> int:\n\
    \x20       return self._n\n\
    \x20   @ret.setter\n\
    \x20   def ret(self, v: int) -> int:\n\
    \x20       self._n = v\n\
    \x20       return v\n\
    class Derived(Base):\n\
    \x20   def scale(self) -> int:\n\
    \x20       return 10\n";

fn setter_of(ctors: &[ExtClassGetsets], class: &str, prop: &str) -> PropertySetter {
    getsets_of(ctors, class)
        .iter()
        .find_map(|getset| match getset {
            ExtGetset::Property { name, setter, .. } if name == prop => Some(setter.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("`{class}.{prop}` is a property descriptor"))
}

fn compiled_setter(name: &str, class: &str, prop: &str, ty: Ty, ret: Ty) -> PropertySetter {
    PropertySetter::Compiled(Box::new(ExtExport {
        name: name.to_string(),
        class: Some(class.to_string()),
        method: Some(prop.to_string()),
        returns_buffer_slice: false,
        receiver: ExtReceiver::SelfInstance,
        params: vec![ty],
        param_writable: vec![false],
        defaults: Vec::new(),
        return_ty: ret,
        keyword_names: None,
    }))
}

/// A property with a compiled setter whose value type the boundary carries
/// routes a store to the receiver-exact setter; one whose value type is not
/// carried (`list[int]`, `tuple`) stays read-only, as an uncarried getter
/// gets no descriptor.
#[test]
fn a_property_setter_routes_to_the_receiver_exact_compiled_setter() {
    let (ctors, _) = ext_build("1458_setters", SETTERS);
    let leaf = Ty::Instance(Box::new("Leaf".to_string()));
    for (class, prop, setter, ty) in [
        ("Base", "n", "Base.n.setter", Ty::Int),
        ("Base", "leaf", "Base.leaf.setter", leaf),
        ("Derived", "n", "Base.n.setter", Ty::Int),
        ("Base", "p", "Base.p.setter", Ty::Int),
        ("Derived", "p", "Derived.p.setter", Ty::Int),
    ] {
        assert_eq!(
            setter_of(&ctors, class, prop),
            compiled_setter(setter, class, prop, ty, Ty::None),
            "{class}.{prop}"
        );
    }
    // A carried non-`None` return is packed by the wrapper and dropped.
    assert_eq!(
        setter_of(&ctors, "Base", "ret"),
        compiled_setter("Base.ret.setter", "Base", "ret", Ty::Int, Ty::Int)
    );
    for prop in ["count", "pair"] {
        assert_eq!(
            setter_of(&ctors, "Base", prop),
            PropertySetter::ReadOnly,
            "{prop}"
        );
    }
}

/// The compiled setter's C: `del` is CPython's `has no deleter`, a carrier
/// whose `__init__` never ran is refused, and a store hands the value to the
/// setter's wrapper as its one argument and drops the wrapper's result. The
/// wrapper of an inherited setter is rendered once for both classes, and an
/// uncarried setter leaves a `NULL` row.
#[test]
fn a_compiled_property_setter_calls_the_setter_wrapper() {
    let (_, inc) = ext_build("1458_setters_c", SETTERS);
    for class in ["Base", "Derived"] {
        let len = class.len();
        let setter = format!(
            "static int pycc_ext_set_{len}_{class}_1_n(PyObject *self, PyObject *value, void *closure)\n{{\n    \
             PyObject *result;\n    (void)closure;\n    if (value == NULL) {{\n        \
             PyErr_SetString(PyExc_AttributeError, \"property 'n' of '{class}' object has no deleter\");\n        \
             return -1;\n    }}\n    if (((PyccExtInstance *)self)->inst == NULL) {{\n        \
             PyErr_SetString(PyExc_AttributeError, \
             \"cannot set 'n' on a '{class}' object whose __init__ never ran\");\n        \
             return -1;\n    }}\n    result = pycc_ext_wrap_0m4_Base1_n6_setter(self, &value, 1);\n    \
             if (result == NULL) {{\n        return -1;\n    }}\n    Py_DECREF(result);\n    \
             return 0;\n}}\n\n"
        );
        assert!(inc.contains(&setter), "missing:\n{setter}\nin:\n{inc}");
        for row in [
            format!(
                "{{\"n\", pycc_ext_get_{len}_{class}_1_n, pycc_ext_set_{len}_{class}_1_n, NULL, NULL}},\n"
            ),
            format!("{{\"count\", pycc_ext_get_{len}_{class}_5_count, NULL, NULL, NULL}},\n"),
        ] {
            assert!(inc.contains(&row), "missing {row} in:\n{inc}");
        }
    }
    let wrapper = "static PyObject *pycc_ext_wrap_0m4_Base1_n6_setter(PyObject *self, \
                   PyObject *const *args, Py_ssize_t nargs)\n";
    assert_eq!(inc.matches(wrapper).count(), 1, "{inc}");
    assert!(
        inc.contains("result = pycc_ext_wrap_0m7_Derived1_p6_setter(self, &value, 1);"),
        "{inc}"
    );
    assert!(!inc.contains("pycc_ext_set_4_Base_5_count("), "{inc}");
}

/// A class with no carriable slot and no property installs no table, so
/// its slot array is exactly what it was before #1442.
#[test]
fn a_class_with_nothing_to_describe_installs_no_getset_slot() {
    let (ctors, inc) = ext_build(
        "1442_none",
        "from typing import List\n\
         class Q:\n\
         \x20   xs: List[int]\n\
         \x20   def __init__(self) -> None:\n\
         \x20       self.xs = [1]\n\
         \x20   def first(self) -> int:\n\
         \x20       return self.xs[0]\n",
    );
    assert!(getsets_of(&ctors, "Q").is_empty());
    assert!(!inc.contains("Py_tp_getset"), "{inc}");
    assert!(!inc.contains("PyGetSetDef"), "{inc}");
}

const PROPERTIES: &str = "from typing import Any, List\n\
    class Base:\n\
    \x20   def __init__(self, stack: Any) -> None:\n\
    \x20       self.stack = stack\n\
    \x20   @property\n\
    \x20   def top(self) -> Any:\n\
    \x20       return self.stack[-1]\n\
    \x20   @property\n\
    \x20   def shadowed(self) -> int:\n\
    \x20       return 1\n\
    \x20   @property\n\
    \x20   def items(self) -> List[int]:\n\
    \x20       return [1]\n\
    \x20   def size(self) -> int:\n\
    \x20       return len(self.stack)\n\
    class Derived(Base):\n\
    \x20   def shadowed(self) -> int:\n\
    \x20       return 2\n";

/// A property is a descriptor where it wins the namespace walk: inherited
/// by `Derived` unchanged, but shadowed there by an ordinary method, and
/// skipped -- not a `C0003` -- when its return type has no packer.
#[test]
fn a_property_is_described_where_it_wins_the_namespace_walk() {
    let (ctors, _) = ext_build("1442_properties", PROPERTIES);
    let names = |class: &str| {
        getsets_of(&ctors, class)
            .iter()
            .map(|getset| getset.name().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(names("Base"), ["stack", "top", "shadowed"]);
    assert_eq!(names("Derived"), ["stack", "top"]);
    assert_eq!(
        getsets_of(&ctors, "Derived")[1],
        ExtGetset::Property {
            name: "top".to_string(),
            getter: ExtExport {
                name: "Base.top".to_string(),
                class: Some("Derived".to_string()),
                method: Some("top".to_string()),
                returns_buffer_slice: false,
                receiver: ExtReceiver::SelfInstance,
                params: Vec::new(),
                param_writable: Vec::new(),
                defaults: Vec::new(),
                return_ty: Ty::Object,
                keyword_names: None,
            },
            setter: PropertySetter::Refuse,
        }
    );
}

/// Two classes reading one inherited getter share one wrapper -- a second
/// definition of the same `pycc_ext_wrap_` symbol is a C redefinition
/// error -- while each class keeps its own NULL-guarded getter shim.
#[test]
fn an_inherited_property_s_wrapper_is_rendered_once() {
    let (_, inc) = ext_build("1442_properties_c", PROPERTIES);
    let wrapper = "static PyObject *pycc_ext_wrap_0m4_Base3_top(PyObject *self, \
                   PyObject *const *args, Py_ssize_t nargs)\n";
    assert_eq!(inc.matches(wrapper).count(), 1, "{inc}");
    for class in ["Base", "Derived"] {
        let shim = format!(
            "static PyObject *pycc_ext_get_{len}_{class}_3_top(PyObject *self, void *closure)\n{{\n    \
             (void)closure;\n    if (((PyccExtInstance *)self)->inst == NULL) {{\n        \
             PyErr_SetString(PyExc_AttributeError, \"'{class}' object has no attribute 'top'\");\n        \
             return NULL;\n    }}\n    return pycc_ext_wrap_0m4_Base3_top(self, NULL, 0);\n}}\n\n",
            len = class.len()
        );
        assert!(inc.contains(&shim), "missing:\n{shim}\nin:\n{inc}");
    }
}

/// A getter whose body depends on its receiver is compiled once more for
/// the subclass (#1337, D-254), and the subclass's descriptor reads that
/// copy rather than the base's compilation.
#[test]
fn a_subclass_s_property_descriptor_reads_its_receiver_exact_copy() {
    let (ctors, inc) = ext_build(
        "1442_copy",
        "class A:\n\
         \x20   def __init__(self) -> None:\n\
         \x20       self.k = 0\n\
         \x20   def m(self) -> int:\n\
         \x20       return 1\n\
         \x20   @property\n\
         \x20   def p(self) -> int:\n\
         \x20       return self.m()\n\
         class B(A):\n\
         \x20   def m(self) -> int:\n\
         \x20       return 2\n",
    );
    let getter_of = |class: &str| {
        getsets_of(&ctors, class)
            .iter()
            .find_map(|getset| match getset {
                ExtGetset::Property { getter, .. } => Some(getter.name.clone()),
                ExtGetset::Slot { .. } => None,
            })
            .expect("`p` is a property descriptor")
    };
    assert_eq!(getter_of("A"), "A.p");
    assert_eq!(getter_of("B"), "B.p");
    assert!(
        inc.contains("return pycc_ext_wrap_0m1_B1_p(self, NULL, 0);"),
        "{inc}"
    );
}

/// #1453: lark's `ParserState.parse_conf: ParseConf` shape. A slot or
/// property declared as a regular class of the module -- generic or not --
/// is described; an enum- or exception-class-typed slot or property is
/// not, and the skip leaves the later slot indices undisturbed.
const INSTANCE_FIELDS: &str = "from enum import Enum\n\
    from typing import Generic, TypeVar\n\
    T = TypeVar('T')\n\
    class Color(Enum):\n\
    \x20   RED = 1\n\
    class Oops(Exception):\n\
    \x20   def __init__(self, m: str) -> None:\n\
    \x20       self.m = m\n\
    class Conf(Generic[T]):\n\
    \x20   def __init__(self, n: int) -> None:\n\
    \x20       self.n = n\n\
    class St(Generic[T]):\n\
    \x20   parse_conf: Conf[T]\n\
    \x20   color: Color\n\
    \x20   err: Oops\n\
    \x20   def __init__(self, parse_conf: Conf[T], k: int) -> None:\n\
    \x20       self.parse_conf = parse_conf\n\
    \x20       self.color = Color.RED\n\
    \x20       self.err = Oops('x')\n\
    \x20       self.k = k\n\
    \x20   @property\n\
    \x20   def conf(self) -> Conf[T]:\n\
    \x20       return self.parse_conf\n\
    \x20   @property\n\
    \x20   def tint(self) -> Color:\n\
    \x20       return self.color\n\
    \x20   def get(self) -> int:\n\
    \x20       return self.k\n";

fn conf_ty() -> Ty {
    Ty::Instance(Box::new("Conf".to_string()))
}

#[test]
fn a_same_module_instance_slot_and_property_get_a_descriptor() {
    let (ctors, _) = ext_build("1453_instance_fields", INSTANCE_FIELDS);
    let getsets = getsets_of(&ctors, "St");
    let names: Vec<&str> = getsets.iter().map(ExtGetset::name).collect();
    assert_eq!(names, ["parse_conf", "k", "conf"]);
    assert_eq!(getsets[0], slot("parse_conf", 0, conf_ty()));
    assert_eq!(getsets[1], slot("k", 3, Ty::Int));
    // The slots come first, so the walk passes both arms before `conf`.
    let property_return = getsets
        .iter()
        .find_map(|getset| match getset {
            ExtGetset::Property { getter, .. } => Some(getter.return_ty.clone()),
            ExtGetset::Slot { .. } => None,
        })
        .expect("`conf` is a property descriptor");
    assert_eq!(property_return, conf_ty());
}

/// The instance slot's getter packs its word through #1449's egress, which
/// returns the instance's live carrier when it has one; the property's
/// wrapper returns through the same packer.
#[test]
fn an_instance_slot_getter_packs_through_the_instance_egress() {
    let (_, inc) = ext_build("1453_instance_fields_c", INSTANCE_FIELDS);
    let getter = "static PyObject *pycc_ext_get_2_St_10_parse_conf(PyObject *self, void *closure)\n{\n    \
                  void *inst = ((PyccExtInstance *)self)->inst;\n    long long word;\n    \
                  (void)closure;\n    if (inst == NULL) {\n        \
                  PyErr_SetString(PyExc_AttributeError, \"'St' object has no attribute 'parse_conf'\");\n        \
                  return NULL;\n    }\n    word = pycc_rt_instance_get_slot_checked(inst, 0);\n    \
                  if (pycc_rt_ext_pending_type() >= 0) {\n        pycc_ext_raise_pending();\n        \
                  return NULL;\n    }\n    \
                  return pycc_ext_pack_instance((void *)(intptr_t)word);\n}\n\n";
    assert!(inc.contains(getter), "missing:\n{getter}\nin:\n{inc}");
    // Part 1 of #1443: the instance slot's setter admits a carrier of the
    // declared class (or a subclass) through #1449's ingress, a plain word.
    for expected in [
        "    if (pycc_ext_unpack_instance(value, \"St.parse_conf\", 0, \"Conf\", &v) != 0) {\n",
        "    word = (long long)(intptr_t)v;\n    \
         pycc_rt_ext_instance_store_slot(inst, 0, 'w', word);\n",
    ] {
        assert!(inc.contains(expected), "missing:\n{expected}\nin:\n{inc}");
    }
    assert!(
        inc.contains("return pycc_ext_activation_exit(pycc_ext_pack_instance(result));"),
        "{inc}"
    );
    for absent in ["_St_5_color", "_St_3_err", "_St_4_tint"] {
        assert!(!inc.contains(absent), "{absent} in:\n{inc}");
    }
}
