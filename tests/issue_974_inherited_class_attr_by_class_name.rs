//! Issue #974: a class-name-qualified read of an *inherited* class attribute
//! (`Derived.LIMIT`, where a base declares `LIMIT`) resolves, end to end
//! through the public `pycc` CLI.
//!
//! Before the fix, `pycc_types`' class-name read looked only at the named
//! class's *own* `class_attrs` and rejected anything else with `T0044`, so
//! `B.LIMIT` failed while `b.LIMIT` -- which already walked the MRO --
//! succeeded. CPython accepts both.
//!
//! Two properties beyond "it now compiles" are pinned here, because the two
//! obvious fixes are each wrong in their own way:
//!
//! - The read must **not** reuse the instance path's #960 precedence. A class
//!   object has no instance `__dict__`, so an instance slot contributed by an
//!   earlier MRO base does not shadow a later base's class attribute. CPython
//!   agrees, and the divergence test below pins both halves of the pair in one
//!   program: `c.x` is `1` and `C.x` is `2`.
//! - The read must **not** walk `class_attrs` alone either. A derived class
//!   that re-declares an inherited attribute's name as a method, static
//!   method, class method or property shadows the base's class attribute in
//!   CPython; folding the base's constant there would turn today's spurious
//!   `T0044` into a silently wrong value. The four shadow tests pin that the
//!   bare read stays `T0044`, and the paired call test pins that the
//!   `@staticmethod` shape still compiles and still prints its own answer.
//!
//! Every accepting test asserts the program's *stdout*, never a bare exit 0:
//! the risk this change carries is a wrong constant, not a failed build.
//! Rejections are matched on the diagnostic code plus a message substring
//! rather than on a rendered path, which prints with forward slashes on
//! Windows CI. Every expected value below was measured against CPython 3.13.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(source.as_bytes()).unwrap();
    path
}

/// Builds and runs `source`, asserting the program's stdout.
fn assert_runs(tag: &str, source: &str, expected_stdout: &str) {
    let dir = ScratchDir::new(tag).expect("failed to create scratch dir");
    let src = write_fixture(&dir, "main.py", source);
    let out = dir.join("main.bin");
    let build = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "pycc build should succeed for {tag}:\n{}",
        String::from_utf8_lossy(&build.stdout)
    );
    let run = Command::new(&out).output().unwrap();
    assert!(run.status.success(), "compiled program {tag} should exit 0");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        expected_stdout,
        "stdout for {tag}"
    );
}

/// Asserts that `pycc check` rejects `source` with a diagnostic containing
/// both `code` and `needle`.
fn assert_rejected(tag: &str, source: &str, code: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("failed to create scratch dir");
    let src = write_fixture(&dir, "main.py", source);
    let out = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    let rendered = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !out.status.success(),
        "pycc check should reject {tag}, but it succeeded"
    );
    assert!(
        rendered.contains(code) && rendered.contains(needle),
        "diagnostic for {tag} should contain {code:?} and {needle:?}, got:\n{rendered}"
    );
}

/// The issue's own program: a `ClassVar` on a base dataclass, read through
/// the derived dataclass's name.
#[test]
fn an_inherited_dataclass_class_var_reads_through_the_derived_class_name() {
    assert_runs(
        "issue974_dataclass",
        "\
from dataclasses import dataclass
from typing import ClassVar

@dataclass
class A:
    x: int
    LIMIT: ClassVar[int] = 8

@dataclass
class B(A):
    y: int

print(B.LIMIT)
",
        "8\n",
    );
}

/// The same defect without any dataclass machinery: it was never
/// dataclass-specific.
#[test]
fn an_inherited_plain_class_attribute_reads_through_the_derived_class_name() {
    assert_runs(
        "issue974_plain",
        "\
class A:
    LIMIT: int = 8

class B(A):
    pass

print(B.LIMIT)
",
        "8\n",
    );
}

/// A derived class that re-declares the name as a class attribute of its own
/// wins over the base's, exactly as the instance read path already did.
#[test]
fn a_derived_class_attribute_wins_over_the_inherited_one() {
    assert_runs(
        "issue974_override",
        "\
class A:
    LIMIT: int = 8

class B(A):
    LIMIT: int = 9

print(B.LIMIT)
",
        "9\n",
    );
}

/// Three levels of single inheritance: the walk is not depth-limited to the
/// direct bases.
#[test]
fn a_class_attribute_two_levels_up_reads_through_the_class_name() {
    assert_runs(
        "issue974_chain",
        "\
class A:
    LIMIT: int = 1

class B(A):
    pass

class C(B):
    pass

print(C.LIMIT)
",
        "1\n",
    );
}

/// A diamond, where the answer distinguishes an MRO walk from a naive
/// depth-first base walk: C3 puts `C` before `A`, so `D.LIMIT` is `3`.
/// A depth-first walk would reach `A` through `B` first and print `1`.
#[test]
fn a_diamond_resolves_the_class_attribute_in_c3_order() {
    assert_runs(
        "issue974_diamond",
        "\
class A:
    LIMIT: int = 1

class B(A):
    pass

class C(A):
    LIMIT: int = 3

class D(B, C):
    pass

print(D.LIMIT)
",
        "3\n",
    );
}

/// The C2 divergence, pinned as one program: the instance read applies #960's
/// slot-wins precedence and prints `1`, while the class-name read has no
/// instance `__dict__` to consult and prints `2`. CPython prints the same
/// pair. A fix that routed the class-name read through the instance path's
/// walk would print `1` twice.
#[test]
fn the_class_name_read_does_not_apply_the_instance_paths_slot_precedence() {
    assert_runs(
        "issue974_divergence",
        "\
class A:
    def __init__(self) -> None:
        self.x = 1

class B:
    x: int = 2

class C(A, B):
    pass

c = C()
print(c.x)
print(C.x)
",
        "1\n2\n",
    );
}

/// A name that exists only as an inherited *instance* slot is still `T0044`
/// through the class name -- there is no class object to read it from.
#[test]
fn an_inherited_instance_slot_is_still_rejected_through_the_class_name() {
    assert_rejected(
        "issue974_instance_slot",
        "\
class A:
    def __init__(self) -> None:
        self.x = 1

class B(A):
    pass

print(B.x)
",
        "T0044",
        "class `B` has no attribute named `x`",
    );
}

/// A name declared nowhere in the MRO is still `T0044`.
#[test]
fn a_name_nowhere_in_the_mro_is_still_rejected() {
    assert_rejected(
        "issue974_absent",
        "\
class A:
    LIMIT: int = 8

class B(A):
    pass

print(B.NOPE)
",
        "T0044",
        "class `B` has no attribute named `NOPE`",
    );
}

/// An inherited *method* read without calling it is still `T0044`: pycc's
/// static-dispatch model has no value representation for an unbound method.
#[test]
fn an_inherited_method_read_without_a_call_is_still_rejected() {
    assert_rejected(
        "issue974_method_read",
        "\
class A:
    def m(self) -> int:
        return 1

class B(A):
    pass

print(B.m)
",
        "T0044",
        "class `B` has no attribute named `m`",
    );
}

/// Shadow shape 1 of 4: a derived `@staticmethod` shadows the base's class
/// attribute, so the bare read must stay `T0044` rather than fold to `2`.
#[test]
fn a_derived_static_method_shadows_an_inherited_class_attribute() {
    assert_rejected(
        "issue974_shadow_static",
        "\
class A:
    x: int = 2

class B(A):
    @staticmethod
    def x() -> int:
        return 1

print(B.x)
",
        "T0044",
        "class `B` has no attribute named `x`",
    );
}

/// The paired positive for the shape above: calling the shadowing static
/// method still compiles and still answers `1`, not the base's `2`. This is
/// the program a `lookup_class_attr_through_mro`-only fix would have
/// mis-compiled.
#[test]
fn calling_the_shadowing_static_method_still_answers_its_own_value() {
    assert_runs(
        "issue974_shadow_static_call",
        "\
class A:
    x: int = 2

class B(A):
    @staticmethod
    def x() -> int:
        return 1

print(B.x())
",
        "1\n",
    );
}

/// Shadow shape 2 of 4: a plain method.
#[test]
fn a_derived_method_shadows_an_inherited_class_attribute() {
    assert_rejected(
        "issue974_shadow_method",
        "\
class A:
    x: int = 2

class B(A):
    def x(self) -> int:
        return 1

print(B.x)
",
        "T0044",
        "class `B` has no attribute named `x`",
    );
}

/// Shadow shape 3 of 4: a `@classmethod`.
#[test]
fn a_derived_class_method_shadows_an_inherited_class_attribute() {
    assert_rejected(
        "issue974_shadow_classmethod",
        "\
class A:
    x: int = 2

class B(A):
    @classmethod
    def x(cls) -> int:
        return 1

print(B.x)
",
        "T0044",
        "class `B` has no attribute named `x`",
    );
}

/// Shadow shape 4 of 4: a `@property`.
#[test]
fn a_derived_property_shadows_an_inherited_class_attribute() {
    assert_rejected(
        "issue974_shadow_property",
        "\
class A:
    x: int = 2

class B(A):
    @property
    def x(self) -> int:
        return 1

print(B.x)
",
        "T0044",
        "class `B` has no attribute named `x`",
    );
}

/// A protocol class really can precede a class attribute's owner in a live
/// MRO, and a bare `x: int` requirement -- the `ProtocolMember::Attribute`
/// half -- must not shadow: it is an interface requirement, not a binding on
/// the class object. CPython prints `2` here too.
#[test]
fn a_protocol_attribute_member_does_not_shadow_an_inherited_class_attribute() {
    assert_runs(
        "issue974_protocol",
        "\
from typing import Protocol

class P(Protocol):
    x: int

class A:
    x: int = 2

class C(P, A):
    pass

print(C.x)
",
        "2\n",
    );
}

/// The `ProtocolMember::Method` half is the opposite: a `Protocol` class
/// executes its body like any other class, so `def x(self) -> int: ...`
/// really does bind `x` in `P.__dict__`. CPython resolves `C.x` to that
/// function object rather than continuing on to `A`'s class attribute, so
/// folding `2` here would be a mis-compile -- and one the shared predicate
/// could not catch by itself, because both crates would agree on the wrong
/// answer. pycc does not model a bare, uncalled method read through a class
/// name, so the read stays `T0044`.
#[test]
fn a_protocol_method_member_shadows_an_inherited_class_attribute() {
    assert_rejected(
        "issue974_protocol_method",
        "\
from typing import Protocol

class P(Protocol):
    def x(self) -> int: ...

class A:
    x: int = 2

class C(P, A):
    pass

print(C.x)
",
        "T0044",
        "class `C` has no attribute named `x`",
    );
}

/// An `ABC` base: the marker base is consumed at lowering time, so the walk
/// must still resolve every MRO entry it visits.
#[test]
fn an_inherited_class_attribute_reads_through_an_abstract_base() {
    assert_runs(
        "issue974_abc",
        "\
from abc import ABC

class Base(ABC):
    LIMIT: int = 4

class Impl(Base):
    pass

print(Impl.LIMIT)
",
        "4\n",
    );
}

/// A hierarchy rooted at a builtin exception class, whose `HirClassDef`s are
/// synthesized rather than lowered from source.
#[test]
fn an_inherited_class_attribute_reads_through_an_exception_hierarchy() {
    assert_runs(
        "issue974_exception",
        "\
class Base(Exception):
    LIMIT: int = 7

class Derived(Base):
    pass

print(Derived.LIMIT)
",
        "7\n",
    );
}

/// The shared shadowing predicate checks five class-level namespaces, and
/// `.iter().any(..)` never runs its closure on an empty table. This fixture
/// makes four of them non-empty on the same walk with names unrelated to the
/// one being read, so every closure executes and answers `false` before the
/// base's class attribute is found. (`enum_members` cannot ride along -- an
/// enum class's MRO is self-only and nothing may inherit from one -- so it is
/// exercised by `an_enum_non_member_read_is_still_rejected` below.)
#[test]
fn every_shadow_namespace_is_consulted_before_the_inherited_class_attribute() {
    assert_runs(
        "issue974_all_namespaces",
        "\
class A:
    LIMIT: int = 8

class B(A):
    def helper(self) -> int:
        return 1

    @property
    def doubled(self) -> int:
        return 2

    @staticmethod
    def made() -> int:
        return 3

    @classmethod
    def named(cls) -> int:
        return 4

print(B.LIMIT)
",
        "8\n",
    );
}

/// Reading a name that is not one of an enum class's members falls past
/// `enum_member_attr_type` into the class-name class-attribute lookup, whose
/// walk consults the enum class's non-empty `enum_members` table and still
/// answers `T0044`. This is the only shape that exercises that namespace's
/// closure: an enum's MRO is self-only by construction.
#[test]
fn an_enum_non_member_read_is_still_rejected() {
    assert_rejected(
        "issue974_enum_non_member",
        "\
from enum import Enum

class Color(Enum):
    RED = 1

print(Color.NOPE)
",
        "T0044",
        "class `Color` has no attribute named `NOPE`",
    );
}

// Codex review round on PR #992: the `#436` class-name `AttrGet` arm fired
// whenever `env.lookup_class(name)` succeeded, without first checking whether
// an active value binding shadows that name. CPython resolves the binding, so
// all three shapes below must read the parameter's instance slot. The
// inherited shape was a regression this issue's own MRO walk introduced (base
// `3ba4a027` rejected it with `T0044`); the own-declared and enum-member
// shapes predate it and the one guard fixes them too.

#[test]
fn an_active_binding_shadows_an_inherited_class_attribute_read() {
    assert_runs(
        "issue974_shadowed_inherited",
        "\
class A:
    X: int = 2


class B(A):
    pass


class D:
    def __init__(self) -> None:
        self.X = 3


def f(B: D) -> int:
    return B.X


def main() -> None:
    print(f(D()))


main()
",
        "3\n",
    );
}

#[test]
fn an_active_binding_shadows_an_own_class_attribute_read() {
    assert_runs(
        "issue974_shadowed_own",
        "\
class B:
    X: int = 2


class D:
    def __init__(self) -> None:
        self.X = 3


def f(B: D) -> int:
    return B.X


def main() -> None:
    print(f(D()))


main()
",
        "3\n",
    );
}

#[test]
fn an_active_binding_shadows_an_enum_member_read() {
    assert_runs(
        "issue974_shadowed_enum",
        "\
from enum import Enum


class Color(Enum):
    RED = 1


class D:
    def __init__(self) -> None:
        self.RED = 3


def f(Color: D) -> int:
    return Color.RED


def main() -> None:
    print(f(D()))


main()
",
        "3\n",
    );
}

#[test]
fn an_active_binding_shadows_a_class_name_static_method_call() {
    assert_runs(
        "issue974_shadowed_static_method",
        "\
class B:
    @staticmethod
    def m() -> int:
        return 2


class D:
    def m(self) -> int:
        return 3


def f(B: D) -> int:
    return B.m()


def main() -> None:
    print(f(D()))


main()
",
        "3\n",
    );
}

#[test]
fn an_active_binding_shadows_a_class_name_class_method_call() {
    assert_runs(
        "issue974_shadowed_class_method",
        "\
class B:
    @classmethod
    def m(cls) -> int:
        return 2


class D:
    def m(self) -> int:
        return 3


def f(B: D) -> int:
    return B.m()


def main() -> None:
    print(f(D()))


main()
",
        "3\n",
    );
}

// The five cases below add one axis to the four class-name shapes above: a
// module that declares a PEP 695 generic function. `monomorphize`'s own
// expression walk recurses into every subexpression looking for generic calls
// to rewrite, and its `AttrGet`/`MethodCall` arms used to recurse into a bare
// class-name base -- inferring it as a value and failing with `T0021` before
// the class-name arms ever ran. The declaration alone is enough to trigger the
// walk; the generic function is never called in any of them, which is exactly
// the shape the defect was reported in. This is not a #974 regression: the own
// -attribute and static-method shapes failed identically before #974's MRO
// walk existed. Values measured against CPython 3.13.

#[test]
fn a_generic_declaration_does_not_break_an_inherited_class_attr_read() {
    assert_runs(
        "issue974_generic_inherited_attr",
        "\
class A:
    X: int = 2


class B(A):
    pass


def ident[T](x: T) -> T:
    return x


def main() -> None:
    print(B.X)


main()
",
        "2\n",
    );
}

#[test]
fn a_generic_declaration_does_not_break_an_own_class_attr_read() {
    assert_runs(
        "issue974_generic_own_attr",
        "\
class A:
    X: int = 2


def ident[T](x: T) -> T:
    return x


def main() -> None:
    print(A.X)


main()
",
        "2\n",
    );
}

#[test]
fn a_generic_declaration_does_not_break_a_class_name_static_method_call() {
    assert_runs(
        "issue974_generic_static_method",
        "\
class A:
    @staticmethod
    def m() -> int:
        return 2


def ident[T](x: T) -> T:
    return x


def main() -> None:
    print(A.m())


main()
",
        "2\n",
    );
}

#[test]
fn a_generic_declaration_does_not_break_a_class_name_class_method_call() {
    assert_runs(
        "issue974_generic_class_method",
        "\
class A:
    @classmethod
    def m(cls) -> int:
        return 2


def ident[T](x: T) -> T:
    return x


def main() -> None:
    print(A.m())


main()
",
        "2\n",
    );
}

#[test]
fn a_binding_still_shadows_a_class_name_when_the_module_declares_a_generic() {
    assert_runs(
        "issue974_generic_shadowed_method",
        "\
class B:
    @staticmethod
    def m() -> int:
        return 2


class D:
    def m(self) -> int:
        return 3


def ident[T](x: T) -> T:
    return x


def f(B: D) -> int:
    return B.m()


def main() -> None:
    print(f(D()))


main()
",
        "3\n",
    );
}

// -- Round 5: a module-scope rebinding of a class's own name ---------
//
// The round-2/3 shadowing guard asks `binding_state`, which inside a
// function body is the module environment as it stands after *all*
// top-level code has run (D-041 late binding). That snapshot cannot tell a
// rebinding that executes before the call from one that executes after it,
// and CPython answers those two differently, so the read is rejected with
// `C0001` rather than resolved to one of the two answers. The four cases
// below are exactly the four rows measured against CPython 3.13: the
// `_after_` pair prints `2` there and the `_before_` pair prints `1`.
//
// A parameter or function-local shadow is untouched -- it is bound at the
// call, not by module top-level code, so it has no ordering ambiguity; the
// accepting tests above already pin that behavior.

/// `A.X` read through the class's own declaration, with the rebinding
/// executing *after* the call. CPython prints `2`.
#[test]
fn rebinding_a_class_name_after_the_read_is_rejected() {
    assert_rejected(
        "issue974_rebind_own_after",
        "\
class A:
    X: int = 2


class D:
    def __init__(self) -> None:
        self.X = 1


def f() -> int:
    return A.X


print(f())
A = D()
",
        "C0001",
        "also bound to a value at module scope",
    );
}

/// The same program with the rebinding executing *before* the call.
/// CPython prints `1` -- a different answer from the same compile-time
/// environment, which is why neither is resolved.
#[test]
fn rebinding_a_class_name_before_the_read_is_rejected() {
    assert_rejected(
        "issue974_rebind_own_before",
        "\
class A:
    X: int = 2


class D:
    def __init__(self) -> None:
        self.X = 1


def f() -> int:
    return A.X


A = D()
print(f())
",
        "C0001",
        "also bound to a value at module scope",
    );
}

/// The rebind-after case reading an *inherited* attribute, the shape #974's
/// own MRO walk added.
#[test]
fn rebinding_a_derived_class_name_after_the_read_is_rejected() {
    assert_rejected(
        "issue974_rebind_inherited_after",
        "\
class A:
    X: int = 2


class B(A):
    pass


class D:
    def __init__(self) -> None:
        self.X = 1


def f() -> int:
    return B.X


print(f())
B = D()
",
        "C0001",
        "also bound to a value at module scope",
    );
}

/// The rebind-before case reading an inherited attribute.
#[test]
fn rebinding_a_derived_class_name_before_the_read_is_rejected() {
    assert_rejected(
        "issue974_rebind_inherited_before",
        "\
class A:
    X: int = 2


class B(A):
    pass


class D:
    def __init__(self) -> None:
        self.X = 1


def f() -> int:
    return B.X


B = D()
print(f())
",
        "C0001",
        "also bound to a value at module scope",
    );
}

/// Round 6 (D-068 re-review of this pull request): the reverse ordering --
/// a top-level *value* binding followed by a `class` statement of the same
/// name -- is rejected at HIR lowering. Before the check, `HirItem` had no
/// `ClassDef` variant, so the checker's sequential top-level pass never
/// re-bound the name back to the class and the read below silently resolved
/// through the stale `D` instance, printing `1` where CPython prints `2`.
#[test]
fn a_class_statement_after_a_value_binding_of_the_same_name_is_rejected() {
    assert_rejected(
        "issue974_class_after_value",
        "\
class D:
    def __init__(self) -> None:
        self.X = 1


A = D()


class A:
    X: int = 2


print(A.X)
",
        "C0001",
        "collides with a value of the same name already bound at module scope",
    );
}

/// The same rejection through an annotated assignment: `killed_names` treats
/// a *valued* `AnnAssign` as a binding exactly as it treats a plain one.
#[test]
fn a_class_statement_after_an_annotated_value_binding_is_rejected() {
    assert_rejected(
        "issue974_class_after_annotated_value",
        "\
class D:
    def __init__(self) -> None:
        self.X = 1


A: D = D()


class A:
    X: int = 2


print(A.X)
",
        "C0001",
        "collides with a value of the same name already bound at module scope",
    );
}

/// A *value-less* annotation binds nothing (CPython records only the
/// annotation), so the class statement below is the name's first binding and
/// the program is accepted -- `killed_names` deliberately excludes it.
#[test]
fn a_class_statement_after_a_value_less_annotation_is_accepted() {
    assert_runs(
        "issue974_class_after_value_less_annotation",
        "\
A: int


class A:
    X: int = 2


print(A.X)
",
        "2\n",
    );
}

/// The hard side-condition of the round-6 check: the *other* ordering --
/// a class statement followed by a value binding of the same name -- stays
/// accepted, because there the assignment is itself a top-level statement
/// the checker's sequential pass does observe.
#[test]
fn a_value_binding_after_a_class_statement_of_the_same_name_still_runs() {
    assert_runs(
        "issue974_value_after_class",
        "\
class A:
    X: int = 2


A = [1]

print(A[0])
",
        "1\n",
    );
}

/// A top-level value binding of an *unrelated* name does not block a later
/// class statement: the check compares the class's own name against the
/// names earlier statements bind, not against the mere presence of one.
#[test]
fn a_class_statement_after_an_unrelated_value_binding_still_runs() {
    assert_runs(
        "issue974_class_after_unrelated_value",
        "\
A = [9]


class B:
    X: int = 2


A = [1]

print(A[0] + B.X)
",
        "3\n",
    );
}

/// `killed_names` recurses into loop bodies and reports loop variables, so a
/// top-level `for` whose variable matches a later class name is rejected on
/// the same grounds as a plain assignment.
#[test]
fn a_class_statement_after_a_top_level_loop_variable_is_rejected() {
    assert_rejected(
        "issue974_class_after_loop_variable",
        "\
for A in range(3):
    print(A)


class A:
    X: int = 2


print(A.X)
",
        "C0001",
        "collides with a value of the same name already bound at module scope",
    );
}
