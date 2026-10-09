//! Which carrier classes get comparison and hash slots (#1427), which
//! bindings are refused with `C0003`, and the C each slot renders to.
//! Lowered through the `--ext` frontend, so `Any` is the opaque object an
//! extension really carries.

use super::*;

use crate::ext_build::{
    collect_carrier_classes, collect_class_publications, collect_constructors, collect_exports,
    generate_exports_inc, generate_exports_inc_with_slots,
};

/// `source` resolved as the `--ext` module `m`.
fn ext_module(tag: &str, source: &str) -> HirModule {
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

fn slots_of(module: &HirModule) -> Result<Vec<ExtSlotDunders>, Vec<Diagnostic>> {
    collect_slot_dunders(module, &collect_carrier_classes(module), SlotArtifact::Ext)
}

fn entry<'s>(slots: &'s [ExtSlotDunders], class: &str) -> &'s ExtSlotDunders {
    slots
        .iter()
        .find(|entry| entry.class == class)
        .unwrap_or_else(|| panic!("`{class}` has slots"))
}

fn comparison_names(entry: &ExtSlotDunders) -> Vec<&'static str> {
    entry.comparisons.iter().map(|(name, _)| *name).collect()
}

const CLASSES: &str = "from typing import Any, Protocol\n\
    class Eq:\n\
    \x20   def __init__(self, v: int) -> None:\n\
    \x20       self.v = v\n\
    \x20   def __eq__(self, other: Any) -> Any:\n\
    \x20       return NotImplemented\n\
    class Derived(Eq):\n\
    \x20   pass\n\
    class Lt:\n\
    \x20   def __init__(self, v: int) -> None:\n\
    \x20       self.v = v\n\
    \x20   def __lt__(self, other: Any) -> Any:\n\
    \x20       return NotImplemented\n\
    \x20   def __ge__(self, other: Any) -> Any:\n\
    \x20       return NotImplemented\n\
    class Hashed:\n\
    \x20   def __init__(self, v: int) -> None:\n\
    \x20       self.v = v\n\
    \x20   def __eq__(self, other: Any) -> Any:\n\
    \x20       return NotImplemented\n\
    \x20   def __hash__(self) -> int:\n\
    \x20       return self.v\n\
    class HashOnly:\n\
    \x20   def __init__(self, v: int) -> None:\n\
    \x20       self.v = v\n\
    \x20   def __hash__(self) -> int:\n\
    \x20       return self.v\n\
    class Rehash(Hashed):\n\
    \x20   def __eq__(self, other: Any) -> Any:\n\
    \x20       return NotImplemented\n\
    class Plain:\n\
    \x20   def __init__(self, v: int) -> None:\n\
    \x20       self.v = v\n\
    class _Private:\n\
    \x20   def __init__(self, v: int) -> None:\n\
    \x20       self.v = v\n\
    \x20   def __ne__(self, other: Any) -> Any:\n\
    \x20       return NotImplemented\n\
    class Typed:\n\
    \x20   def __init__(self, v: int) -> None:\n\
    \x20       self.v = v\n\
    \x20   def __eq__(self, other: \"Typed\") -> bool:\n\
    \x20       return self.v == other.v\n\
    class Shape(Protocol):\n\
    \x20   def __eq__(self, other: int) -> bool: ...\n\
    def make() -> _Private:\n\
    \x20   return _Private(1)\n";

#[test]
fn the_lexical_predicate_admits_exactly_a_classes_seven_slot_dunders() {
    for name in SLOT_DUNDERS {
        assert!(is_slot_dunder_method(&format!("Grid.{name}")), "{name}");
    }
    for refused in [
        "Grid.__bool__",
        "Grid.__init__",
        "Grid.eq",
        "__eq__",
        ".__eq__",
        "0gen_Grid.__eq__",
        "Grid.__eq__.extra",
    ] {
        assert!(!is_slot_dunder_method(refused), "{refused}");
    }
}

#[test]
fn each_carrier_class_gets_the_slots_its_mro_resolves() {
    let module = ext_module("rc-classes", CLASSES);
    let slots = slots_of(&module).expect("every dunder is installable");

    // `__eq__` alone: the data model makes the type unhashable.
    let eq = entry(&slots, "Eq");
    assert_eq!(comparison_names(eq), ["__eq__"]);
    assert_eq!(eq.hash, SlotHash::NotImplemented);
    assert_eq!(eq.comparisons[0].1.receiver, ExtReceiver::SelfInstance);
    assert_eq!(eq.comparisons[0].1.params.len(), 1);

    // Inherited through type lookup, through the receiver-exact copy.
    let derived = entry(&slots, "Derived");
    assert_eq!(comparison_names(derived), ["__eq__"]);
    assert_eq!(derived.comparisons[0].1.class.as_deref(), Some("Derived"));
    assert_eq!(derived.hash, SlotHash::NotImplemented);

    // Ordering only: `object`'s identity hash, installed explicitly.
    let lt = entry(&slots, "Lt");
    assert_eq!(comparison_names(lt), ["__lt__", "__ge__"]);
    assert_eq!(lt.hash, SlotHash::Identity);

    let hashed = entry(&slots, "Hashed");
    assert_eq!(comparison_names(hashed), ["__eq__"]);
    assert!(
        matches!(&hashed.hash, SlotHash::Compiled(export) if export.method.as_deref() == Some("__hash__"))
    );

    // `__hash__` alone: no `tp_richcompare`, but its own hash.
    let hash_only = entry(&slots, "HashOnly");
    assert!(hash_only.comparisons.is_empty());
    assert!(matches!(hash_only.hash, SlotHash::Compiled(_)));

    // An own `__eq__` stops the base's `__hash__` from being inherited.
    assert_eq!(entry(&slots, "Rehash").hash, SlotHash::NotImplemented);

    let private = entry(&slots, "_Private");
    assert_eq!(comparison_names(private), ["__ne__"]);
    assert_eq!(private.hash, SlotHash::Identity);

    // No dunder resolved, or a Protocol: no entry at all.
    assert!(slots.iter().all(|entry| entry.class != "Plain"));
    assert!(slots.iter().all(|entry| entry.class != "Shape"));
}

/// The `C0003` messages `collect_slot_dunders` refuses `source` with.
fn refusals(tag: &str, source: &str) -> Vec<String> {
    let module = ext_module(tag, source);
    slots_of(&module)
        .expect_err("the fixture's dunder cannot be installed")
        .into_iter()
        .map(|gap| {
            assert_eq!(gap.code, EXT_CAPABILITY_CODE);
            assert!(gap.message.contains("#1427"), "{}", gap.message);
            gap.message
        })
        .collect()
}

#[test]
fn a_dunder_bound_as_anything_but_an_instance_method_is_refused() {
    let cases = [
        (
            "rc-static",
            "class S:\n    @staticmethod\n    def __eq__(a: int, b: int) -> bool:\n        return True\n",
            "`S.__eq__` as the host-visible `__eq__` of `S` instances: it is bound as a `@staticmethod`",
        ),
        (
            "rc-classmethod",
            "class S:\n    @classmethod\n    def __lt__(cls, b: int) -> bool:\n        return True\n",
            "it is bound as a `@classmethod`",
        ),
        (
            "rc-property",
            "class S:\n    def __init__(self) -> None:\n        self.v = 1\n    @property\n    def __hash__(self) -> int:\n        return 1\n",
            "it is bound as a `@property`",
        ),
        (
            "rc-attr",
            "class S:\n    __le__ = 1\n",
            "it is bound as a class attribute",
        ),
    ];
    for (tag, source, expected) in cases {
        let messages = refusals(tag, source);
        assert!(
            messages.iter().any(|message| message.contains(expected)),
            "{tag}: {messages:?}"
        );
    }
}

#[test]
fn an_inherited_refusal_names_the_owner_and_the_receiving_class() {
    let messages = refusals(
        "rc-inherited",
        "class S:\n    @staticmethod\n    def __gt__(a: int, b: int) -> bool:\n        return True\n\
         class T(S):\n    pass\n",
    );
    assert!(
        messages.iter().any(|message| message
            .contains("install `S.__gt__` as the host-visible `__gt__` of `T` instances")),
        "{messages:?}"
    );
}

#[test]
fn a_generic_class_and_an_uncarriable_signature_are_refused() {
    let generic = refusals(
        "rc-generic",
        "from typing import Any\nclass G[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n    \
         def __eq__(self, other: Any) -> Any:\n        return NotImplemented\n\
         def make() -> int:\n    g = G[int](1)\n    return 1\n",
    );
    assert!(
        generic
            .iter()
            .any(|message| message.contains("PEP 695 generic class")
                && message.contains("erased `Generic[T]` base")
                && !message.contains("define it as an instance method")),
        "{generic:?}"
    );
    let uncarriable = refusals(
        "rc-uncarriable",
        "class D:\n    def __init__(self) -> None:\n        self.v = 1\n    \
         def __lt__(self, other: dict[str, int]) -> bool:\n        return True\n    \
         def __hash__(self) -> list[int]:\n        return [1]\n",
    );
    assert!(
        uncarriable
            .iter()
            .any(|message| message.contains("`__lt__` of `D` instances: its ")),
        "{uncarriable:?}"
    );
}

#[test]
fn an_uncarriable_hash_is_refused() {
    let messages = refusals(
        "rc-hash",
        "class D:\n    def __init__(self) -> None:\n        self.v = 1\n    \
         def __hash__(self, salt: dict[str, int]) -> int:\n        return 1\n",
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("`__hash__` of `D` instances: its ")),
        "{messages:?}"
    );
}

#[test]
fn a_richcompare_renders_object_richcompare_around_the_defined_methods() {
    let module = ext_module("rc-render", CLASSES);
    let slots = slots_of(&module).expect("installable");
    let mut emitted = Vec::new();

    let eq = slot_functions_c(entry(&slots, "Eq"), &mut emitted);
    assert!(eq.contains(
        "static PyObject *pycc_ext_richcompare_Eq(PyObject *self, PyObject *other, int op)"
    ));
    assert!(eq.contains("if (op == Py_EQ) {\n        return pycc_ext_wrap_"));
    // `!=` inverts the type's own `==`, passing `NotImplemented` through.
    assert!(eq.contains("PyObject *res = pycc_ext_richcompare_Eq(self, other, Py_EQ);"));
    assert!(eq.contains("return PyBool_FromLong(!truth);"));
    assert!(!eq.contains("self == other"));
    assert!(eq.ends_with("    Py_RETURN_NOTIMPLEMENTED;\n}\n\n"));
    assert!(!eq.contains("pycc_ext_hash_Eq"));
    assert_eq!(
        slot_rows(entry(&slots, "Eq")),
        "    {Py_tp_richcompare, pycc_ext_richcompare_Eq},\n    {Py_tp_hash, pycc_ext_unhashable},\n"
    );

    // Without `__eq__`, `==` is identity; with `__ne__`, no inversion.
    let private = slot_functions_c(entry(&slots, "_Private"), &mut emitted);
    assert!(private.contains("if (op == Py_EQ && self == other) {\n        Py_RETURN_TRUE;"));
    assert!(private.contains("if (op == Py_NE) {\n        return pycc_ext_wrap_"));
    assert!(!private.contains("PyBool_FromLong"));
    assert_eq!(
        slot_rows(entry(&slots, "_Private")),
        "    {Py_tp_richcompare, pycc_ext_richcompare__Private},\n    {Py_tp_hash, pycc_ext_identity_hash},\n"
    );

    let hash_only = slot_functions_c(entry(&slots, "HashOnly"), &mut emitted);
    assert!(!hash_only.contains("richcompare"));
    assert!(hash_only.contains(
        "static Py_hash_t pycc_ext_hash_HashOnly(PyObject *self)\n{\n    return pycc_ext_finish_hash(pycc_ext_wrap_"
    ));
    assert_eq!(
        slot_rows(entry(&slots, "HashOnly")),
        "    {Py_tp_hash, pycc_ext_hash_HashOnly},\n"
    );

    // An operand annotated with a compiled class: any other operand is
    // `NotImplemented` before the wrapper's ingress check can refuse it.
    let typed = slot_functions_c(entry(&slots, "Typed"), &mut emitted);
    assert!(typed.contains(
        "    if (op == Py_EQ) {\n        if (!pycc_ext_slot_operand_is(other, \"Typed\")) {\n            Py_RETURN_NOTIMPLEMENTED;\n        }\n        return pycc_ext_wrap_"
    ));
    assert!(!eq.contains("pycc_ext_slot_operand_is"));

    // A wrapper already emitted for the artifact is not redefined.
    let again = slot_functions_c(entry(&slots, "Eq"), &mut emitted);
    assert!(!again.contains("static PyObject *pycc_ext_wrap_"));
}

#[test]
fn an_unpublished_class_gets_a_hidden_carrier_type_registered_up_front() {
    let module = ext_module("rc-hidden", CLASSES);
    let slots = slots_of(&module).expect("installable");
    let exports = collect_exports(&module).expect("the module exports");
    let publications = collect_class_publications(&module, &exports);
    let mut emitted = Vec::new();
    let (defs, registration) = hidden_carrier_types_c(&publications, &[], &slots, &mut emitted);
    assert!(defs.contains("static PyType_Spec pycc_ext_carrier_spec__Private = {"));
    assert!(defs.contains("PYCC_EXT_MODULE_NAME_STR \"._Private\","));
    assert!(defs.contains(
        "Py_TPFLAGS_DEFAULT | Py_TPFLAGS_DISALLOW_INSTANTIATION | Py_TPFLAGS_IMMUTABLETYPE"
    ));
    assert!(defs.contains("{Py_tp_richcompare, pycc_ext_richcompare__Private},"));
    assert!(defs.contains("pycc_ext_instance_copy"));
    assert!(registration.contains("pycc_ext_carrier_register(\"_Private\", type)"));
    // A published class keeps its slots on its own type object.
    assert!(!defs.contains("pycc_ext_carrier_spec_Eq "));
    assert!(!registration.contains("\"Eq\""));

    let carriers = collect_carrier_classes(&module);
    let ctors = collect_constructors(&module, &publications);
    let inc = generate_exports_inc_with_slots(
        "m",
        &exports,
        &[],
        &publications,
        &ctors,
        &carriers,
        &[],
        &slots,
    );
    assert!(inc.contains("    {Py_tp_richcompare, pycc_ext_richcompare_Eq},\n"));
    assert!(inc.contains("pycc_ext_carrier_register(\"_Private\", type)"));
    // An unhashable type's marker is swapped for the real
    // `PyObject_HashNotImplemented` before the spec is read; a hashable
    // one's array is left alone.
    assert!(inc.contains(
        "    pycc_ext_unhashable_slots(pycc_ext_type_slots_Eq);\n    type = PyType_FromSpec(&pycc_ext_type_spec_Eq);"
    ));
    assert!(!inc.contains("pycc_ext_unhashable_slots(pycc_ext_carrier_slots__Private)"));
    assert!(!inc.contains("pycc_ext_unhashable_slots(pycc_ext_type_slots_HashOnly)"));
    // Without slots the companion carries none of this.
    let bare = generate_exports_inc("m", &exports, &[], &publications, &ctors, &carriers);
    assert!(!bare.contains("Py_tp_richcompare"));
    assert!(!bare.contains("pycc_ext_carrier_spec_"));
}

/// #1470: the embedded executable's refusals name it instead of `--ext`,
/// whose text keeps #1427's remedy byte for byte.
#[test]
fn an_embedded_refusal_names_the_embedded_executable() {
    let module = ext_module(
        "rc-embed-wording",
        "class S:\n    @staticmethod\n    def __eq__(a: int, b: int) -> bool:\n        return True\n",
    );
    let carriers = collect_carrier_classes(&module);
    let embedded = collect_slot_dunders(&module, &carriers, SlotArtifact::Embedded)
        .expect_err("a static `__eq__` is refused in an embedded executable");
    assert_eq!(embedded.len(), 1, "{embedded:?}");
    assert_eq!(embedded[0].code, EXT_CAPABILITY_CODE);
    assert!(
        embedded[0].message.starts_with(
            "an embedded executable cannot install `S.__eq__` as the host-visible `__eq__` of \
             `S` instances: it is bound as a `@staticmethod`"
        ),
        "{}",
        embedded[0].message
    );
    assert!(
        embedded[0].message.ends_with(" (#1427, #1470)"),
        "{}",
        embedded[0].message
    );
    assert!(
        !embedded[0].message.contains("--ext"),
        "{}",
        embedded[0].message
    );
    let ext = slots_of(&module).expect_err("and in an `--ext` module");
    assert!(
        ext[0]
            .message
            .starts_with("--ext cannot install `S.__eq__`"),
        "{}",
        ext[0].message
    );
    assert!(
        ext[0].message.ends_with(", or build without --ext (#1427)"),
        "{}",
        ext[0].message
    );
}

/// #1470: an embedded executable is compiled without export thunks, so a
/// slot dunder whose signature carries a `tuple` is refused there, while an
/// `--ext` module installs it through its thunk.
#[test]
fn a_tuple_signature_slot_is_refused_only_in_an_embedded_executable() {
    let module = ext_module(
        "rc-embed-tuple",
        "class T:\n    def __init__(self) -> None:\n        self.v = 1\n    \
         def __eq__(self, other: tuple[int, int]) -> bool:\n        return True\n    \
         def __hash__(self) -> int:\n        return 1\n",
    );
    let ext = slots_of(&module).expect("an `--ext` module installs a tuple-carrying slot");
    assert_eq!(comparison_names(entry(&ext, "T")), ["__eq__"]);
    let carriers = collect_carrier_classes(&module);
    let embedded = collect_slot_dunders(&module, &carriers, SlotArtifact::Embedded)
        .expect_err("an embedded executable refuses it");
    assert_eq!(embedded.len(), 1, "{embedded:?}");
    assert!(
        embedded[0].message.contains(
            "install `T.__eq__` as the host-visible `__eq__` of `T` instances: its signature \
             needs an export thunk (it carries a `tuple` or returns a slice of a `memoryview` \
             argument), and an embedded executable is compiled without one"
        ) && embedded[0].message.contains("take and return only scalars"),
        "{}",
        embedded[0].message
    );
}
