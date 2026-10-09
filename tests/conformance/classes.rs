//! Conformance cohort: class model and object-oriented semantics.
//!
//! A `#[path]`-declared submodule of the `tests/conformance.rs` harness (see
//! its `harness_modules!` block). The helpers, `pycc_bin`, and
//! `oracle_python_bin` are the root's private items, visible here through
//! `use super::*;`. Every fixture stays flat under `tests/fixtures/` (D-102).

use super::*;

#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0435_enum_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0435_enum.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0435_enum_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0435_enum.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0435_enum_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0435_enum.py"
    );
}

// PEP 673 (#387 Part 1): `Self` as a method return-type annotation. A
// method returning `Self` yields the class's own instance type, exactly
// like CPython 3.14's deferred-evaluation semantics.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0673_self_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0673_self.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0673_self_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0673_self.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0673_self_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0673_self.py"
    );
}

// PEP 649/749 (#387 Part 2): self-referential deferred annotations. A
// class's method may use the class's own name as a parameter/return type
// annotation, even though the class is not fully defined at the point the
// annotation text appears in source. CPython 3.14 defers evaluation by
// default; pycc resolves the class name at HIR-lowering time since the class
// name is already in scope within its own body.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0649_deferred_ann_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0649_deferred_ann.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0649_deferred_ann_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0649_deferred_ann.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0649_deferred_ann_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0649_deferred_ann.py"
    );
}

// PEP 563 (#919, D-229): `from __future__ import annotations` is a
// compile-time no-op -- pycc already evaluates annotations statically, so the
// directive contributes nothing and the file runs as if it were absent. The
// fixture exercises only what pycc supports today (a method annotated with
// its own class, quoted and unquoted, and a function annotated with an
// already-defined class); a forward reference to a later-defined name and
// `__annotations__` introspection are the row's recorded `core` gaps in the
// breadth manifest, so the matrix row is `◐` (#937), not `✅`.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0563_lazy_annotations_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0563_lazy_annotations.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0563_lazy_annotations_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0563_lazy_annotations.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0563_lazy_annotations_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0563_lazy_annotations.py"
    );
}

// PEP 695 (#387 Part 3): scoped generic classes with one type parameter.
// `class C[T]:` defines a generic class; `C[int](args)` instantiates it
// with a concrete scalar type. pycc monomorphizes the class's methods at
// each call site, reusing PR-13's D-133/D-134 call-site-substitution
// infrastructure.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0695_generic_classes_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0695_generic_classes.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0695_generic_classes_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0695_generic_classes.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0695_generic_classes_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0695_generic_classes.py"
    );
}

// #377: `@property` getter -- `obj.x` is transparently rewritten to
// `obj.x()` (a call to the getter method) at the HIR/MIR level. The
// fixture exercises a read-only property on a class with a backing slot.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn property_basic_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/property_basic.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("property_basic_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/property_basic.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("property_basic_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/property_basic.py"
    );
}

// #377: `@property` getter + `@<name>.setter` -- `obj.x = value` is
// transparently rewritten to `obj._set_x(value)` (a call to the setter
// method) at the HIR/MIR level. The fixture exercises a read-write
// property with a backing slot.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn property_setter_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/property_setter.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("property_setter_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/property_setter.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("property_setter_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/property_setter.py"
    );
}

// #432: basic single inheritance with method override. A derived class
// overrides a base class method; the derived method is called (static
// dispatch via the C3 MRO). A third class inherits without overriding,
// demonstrating inherited method resolution.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn inheritance_basic_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/inheritance_basic.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("inheritance_basic_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/inheritance_basic.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("inheritance_basic_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/inheritance_basic.py"
    );
}

// PEP 544 (#380): Protocols and structural typing. A @runtime_checkable
// protocol with a method requirement, two conforming concrete classes,
// a non-conforming class, protocol-typed variable assignments, isinstance
// against the runtime_checkable protocol, and a protocol-typed function
// parameter. The protocol is a compile-time-only interface in pycc and a
// deferred-annotation type hint in CPython 3.14, so only the concrete
// method calls and isinstance results produce observable output.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0544_protocol_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0544_protocol.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0544_protocol_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0544_protocol.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0544_protocol_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0544_protocol.py"
    );
}

// PEP 3119 (#380): ABC and @abstractmethod. An ABC base class with an
// @abstractmethod, two concrete subclasses that override it (one with
// constructor parameters), super().__init__() chaining, and instantiation
// + method calls. The ABC and @abstractmethod are compile-time-only
// markers in pycc and runtime enforcement in CPython 3.14, but only the
// concrete method calls produce observable output.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_3119_abc_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_3119_abc.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_3119_abc_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_3119_abc.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_3119_abc_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_3119_abc.py"
    );
}

// PEP 3135 (#580, Part 4 of #572): zero-argument `super()`. A three-level
// inheritance chain exercising `super().__init__()` with and without
// arguments and an overridden method calling `super().<method>()`, so each
// level's own `super()` must resolve against its *defining* class rather
// than the runtime type of `self` (pycc lowers it with static dispatch per
// D-160; CPython's zero-arg form reads the same `__class__` cell).
//
// #587 also covers `super().<attr>`, split by what a `super` object
// actually proxies. A base class `@property` is a class-level descriptor
// found along the MRO, so `super().power` calls the base getter rather
// than the subclass's override -- that half is exercised here. An
// *instance* attribute established by `self.<attr> = ...` is not proxied,
// and CPython raises `AttributeError`; pycc rejects that form at compile
// time with `T0047`, so it cannot appear in a fixture that must match the
// oracle byte-for-byte and stays a declared gap in the breadth manifest
// instead (`tests/issue_433_super.rs` asserts the rejection).
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_3135_super_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_3135_super.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_3135_super_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_3135_super.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_3135_super_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_3135_super.py"
    );
}

// PEP 557 (#579, Part 3 of #572): dataclasses. `@dataclass` with required
// annotated fields, the synthesized `__init__`/`__eq__`/`__repr__`, and
// dataclass inheritance (parent fields first). The fixture carries
// `from dataclasses import dataclass` because CPython evaluates decorators
// eagerly and would otherwise raise `NameError`; pycc recognizes the bare
// name without the import and only needs the import itself to resolve.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0557_dataclasses_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0557_dataclasses.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0557_dataclasses_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0557_dataclasses.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0557_dataclasses_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0557_dataclasses.py"
    );
}

// PEP 698 (#579, Part 3 of #572): `@override`. A pure runtime marker in
// CPython (it returns the decorated function unchanged), so the observable
// output comes entirely from ordinary method overriding; pycc additionally
// verifies at compile time that the decorated name exists in a base class.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0698_override_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0698_override.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0698_override_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0698_override.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0698_override_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0698_override.py"
    );
}

// PEP 3129 (#579, Part 3 of #572): class decorators. Exercises the one
// class-decorator form pycc supports and CPython agrees with (`@dataclass`);
// `@dataclass_transform()` is deliberately absent because pycc treats it as
// `@dataclass` while CPython synthesizes nothing, a divergence that cannot
// appear in a byte-for-byte fixture and is recorded in D-196 (see #749).
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_3129_class_deco_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_3129_class_deco.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_3129_class_deco_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_3129_class_deco.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_3129_class_deco_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_3129_class_deco.py"
    );
}

// #610 (PEP 560): value-position `C[x]` dispatches to
// `C.__class_getitem__(x)`. Covers both the `@staticmethod` and the
// `@classmethod` spelling of the hook, inheritance of the hook through the
// MRO, dispatch from inside a function body, and that an ordinary instance
// attribute on the same class is unaffected.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0560_class_getitem_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0560_class_getitem.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0560_class_getitem_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0560_class_getitem.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0560_class_getitem_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0560_class_getitem.py"
    );
}

// #1209 (Part 1 of #1018): `obj.attr op= v` and `d[k] op= v`, including a
// property's get/value/set order and a `KeyError` raised before the value.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn aug_assign_targets_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/aug_assign_targets.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("aug_assign_targets_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/aug_assign_targets.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("aug_assign_targets_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/aug_assign_targets.py"
    );
}

// #1213 (Part 5 of #1018): chained assignment. The value is evaluated once
// and assigned left to right, each target's own base and key evaluated as
// it is assigned: names, attributes (with a property setter and `__init__`'s
// pre-scan), a `dict[str, int]` subscript and the key-rebinding traps.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn chain_assign_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chain_assign.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("chain_assign_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/chain_assign.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("chain_assign_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/chain_assign.py"
    );
}

// #1244 (Part 1 of #1216): `del name` at module and function scope, in both
// `if` arms, in every loop kind and handler kind, followed by a rebinding.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn del_name_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/del_name.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("del_name_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/del_name.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("del_name_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/del_name.py"
    );
}

// #1262 (Part 1 of #1218): `list[int]`/`dict[str, int]` instance attributes
// seeded from an `__init__` parameter, read, mutated through an alias,
// reassigned in a method and inherited by a subclass.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn instance_container_slots_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/instance_container_slots.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("instance_container_slots_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/instance_container_slots.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("instance_container_slots_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/instance_container_slots.py"
    );
}

// #1263 (Part 2 of #1218): `.append()`, `.pop()` and `.get(k, default)` on
// an instance-attribute receiver, including evaluation order and an
// empty-list `IndexError` caught at module scope.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn attr_container_methods_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/attr_container_methods.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("attr_container_methods_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/attr_container_methods.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("attr_container_methods_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/attr_container_methods.py"
    );
}

// #1264 (Part 3 of #1218): annotated empty `list[int]`/`dict[str, int]`
// initialisers in `__init__`, a fresh container per instance, a reset in a
// method, a store from a free function, and a `this` receiver.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn instance_annotated_container_slots_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/instance_annotated_container_slots.py");
    let (debug_pycc, debug_cpython) = run_conformance_fixture_with_profile(
        "instance_annotated_container_slots_debug",
        &fixture,
        false,
    );
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on \
         tests/fixtures/instance_annotated_container_slots.py"
    );
    let (release_pycc, release_cpython) = run_conformance_fixture_with_profile(
        "instance_annotated_container_slots_release",
        &fixture,
        true,
    );
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on \
         tests/fixtures/instance_annotated_container_slots.py"
    );
}

// #1265 (Part 4 of #1218): unannotated empty `[]` initialisers in
// `__init__`, typed from a producer in another method, a producer in
// `__init__` itself, or an inherited slot; a reset in a method; and a
// `this` receiver.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn instance_unannotated_list_slots_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/instance_unannotated_list_slots.py");
    let (debug_pycc, debug_cpython) = run_conformance_fixture_with_profile(
        "instance_unannotated_list_slots_debug",
        &fixture,
        false,
    );
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on \
         tests/fixtures/instance_unannotated_list_slots.py"
    );
    let (release_pycc, release_cpython) = run_conformance_fixture_with_profile(
        "instance_unannotated_list_slots_release",
        &fixture,
        true,
    );
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on \
         tests/fixtures/instance_unannotated_list_slots.py"
    );
}

// #1266 (Part 5 of #1218): a value-less class-body annotation declares an
// instance attribute's type; `[]`/`{}` establishing a declared container slot
// takes the declared type, and a subclass may declare its own attribute.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn instance_declared_attrs_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/instance_declared_attrs.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("instance_declared_attrs_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/instance_declared_attrs.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("instance_declared_attrs_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/instance_declared_attrs.py"
    );
}

// #1368: `__slots__` in a class body -- every value-bound spelling, an
// empty tuple, inherited and redeclared slots, a slotted class over an
// unslotted base, a diamond, and a slotted exception class.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn instance_slots_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/instance_slots.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("instance_slots_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/instance_slots.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("instance_slots_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/instance_slots.py"
    );
}

// #1254 (Part 1 of #1214, D-250): list, set and dict comprehensions in any
// expression position, including nested ones and ones inside `try`.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn comprehension_expr_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/comprehension_expr.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("comprehension_expr_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/comprehension_expr.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("comprehension_expr_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/comprehension_expr.py"
    );
}

// #1337 (D-254): an inherited method runs the body its receiver's class
// resolves to -- `self.m()`, `super()`, class attributes, properties,
// `isinstance(self, ...)` and classmethods, each compiled per receiver.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn receiver_exact_dispatch_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/receiver_exact_dispatch.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("receiver_exact_dispatch_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/receiver_exact_dispatch.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("receiver_exact_dispatch_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/receiver_exact_dispatch.py"
    );
}

// #1337 (D-254): an inherited method runs the body its receiver's class
// resolves to -- `self.m()`, `super()`, class attributes, properties,
// `isinstance(self, ...)` and classmethods, each compiled per receiver.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn receiver_exact_protocol_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/receiver_exact_protocol.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("receiver_exact_protocol_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/receiver_exact_protocol.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("receiver_exact_protocol_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/receiver_exact_protocol.py"
    );
}

// #1337 (WI-6a): `isinstance` on a caught builtin exception value is decided
// from its runtime type tag; an `except*` group and a plain user exception
// instance keep the compile-time answer.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn exception_isinstance_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/exception_isinstance.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("exception_isinstance_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/exception_isinstance.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("exception_isinstance_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/exception_isinstance.py"
    );
}
