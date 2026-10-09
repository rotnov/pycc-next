//! Issue #969: multiple inheritance whose base layouts are not prefixes of the
//! derived layout is rejected with `C0001`, end to end through the public
//! `pycc` CLI.
//!
//! Under D-154 every method is lowered exactly once, against its own class's
//! flat attribute-slot layout, and a derived class's layout is assigned
//! most-base-first over its MRO. When two classes in one MRO each declare
//! their own instance attributes, the earlier base's slots get re-based in the
//! derived layout while its already-lowered methods keep addressing the old
//! indices. The result is a silently wrong value, or an abort on a slot the
//! running constructor never wrote. No reordering fixes it: for two non-empty
//! disjoint base layouts, no single flat ordering makes both a prefix.
//!
//! `pycc_hir` therefore rejects the shape at lowering time (D-234). The exact
//! predicate is a *name-sequence prefix* test, not "two bases with slots":
//! bases that declare the same attribute names share one slot and stay
//! accepted, as do methods-only mixins, class-attribute-only bases, builtin
//! exception bases, and attribute-free diamond intermediates. The accepting
//! tests below assert the program's *stdout*, never a bare exit 0, because
//! the defect this gate replaces was a wrong value from a program that
//! compiled fine. Rejections are matched on the diagnostic code plus a
//! message substring rather than on a rendered path, which prints with
//! forward slashes on Windows CI.

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

const NOT_A_PREFIX: &str = "instance layout is not a prefix of";

/// The issue's own program. Before this change it printed `3` for both reads:
/// `A.get_a` addressed slot 0, which the derived layout `[a, b]` had given to
/// `a`, but `C.__init__` ran `A.__init__` through `super()` and then wrote
/// `self.b`, so the two reads collided. CPython prints `7` then `3`.
#[test]
fn two_bases_each_declaring_their_own_attributes_are_rejected() {
    assert_rejected(
        "issue969_wrong_value",
        "\
class A:
    def __init__(self) -> None:
        self.a = 7

    def get_a(self) -> int:
        return self.a


class B:
    def __init__(self) -> None:
        self.b = 3

    def get_b(self) -> int:
        return self.b


class C(A, B):
    def __init__(self) -> None:
        super().__init__()
        self.b = 3


c = C()
print(c.get_a())
print(c.get_b())
",
        "C0001",
        NOT_A_PREFIX,
    );
}

/// The same defect's louder half, previously pinned as a known limitation in
/// `tests/issue_966_inherited_init_rank.rs`: `C(B, D)` inherits `B`'s
/// constructor, which writes slot 0, leaving `D.w`'s slot unwritten. Reading
/// `c.z` aborted in `pycc_rt`; reading `c.w` printed `B.z`'s value where
/// CPython raises `AttributeError`.
#[test]
fn an_inherited_constructor_that_leaves_a_sibling_base_s_slot_unwritten_is_rejected() {
    assert_rejected(
        "issue969_unwritten_slot",
        "\
class B:
    def __init__(self) -> None:
        self.z = 1


class D:
    def __init__(self) -> None:
        self.w = 2


class C(B, D):
    pass


c = C()
print(c.z)
",
        "C0001",
        NOT_A_PREFIX,
    );
}

/// `super()` re-entering a *shadowed* base member is why the gate cannot be
/// refined to "only reject when the mis-addressed member is reachable from the
/// derived class". `D` overrides both `__init__` and `f`, so neither of `B`'s
/// own definitions is directly reachable -- yet `D.f` calls `super().f()`,
/// which is `B.f`, lowered against `B`'s layout `[n]` while `D`'s is
/// `[X, n]`. CPython prints `7`; pycc aborted in `pycc_rt`.
#[test]
fn a_super_call_into_a_shadowed_base_member_is_rejected() {
    assert_rejected(
        "issue969_super_reentry",
        "\
class B:
    def __init__(self) -> None:
        self.n = 0

    def f(self) -> int:
        return self.n


class C:
    def __init__(self) -> None:
        self.X = 5


class D(B, C):
    def __init__(self) -> None:
        self.n = 7

    def f(self) -> int:
        return super().f()


print(D().f())
",
        "C0001",
        NOT_A_PREFIX,
    );
}

/// A diamond is rejected exactly when *both* branches add a slot of their own:
/// `Base`/`A(Base)`/`B(Base)`/`C(A, B)` linearizes to `[C, A, B, Base]`, so
/// `C`'s layout is assigned most-base-first as `[base, b, a]` while `A`'s own
/// is `[base, a]` -- they agree on slot 0 and diverge at slot 1, and no flat
/// ordering can put both `b` and `a` there. A *one-sided* diamond, where only
/// one branch adds anything, stays accepted (below), as does one whose
/// intermediates add nothing at all.
#[test]
fn a_diamond_whose_both_branches_add_an_attribute_is_rejected() {
    assert_rejected(
        "issue969_diamond_reject",
        "\
class Base:
    def __init__(self) -> None:
        self.base = 1


class A(Base):
    def __init__(self) -> None:
        super().__init__()
        self.a = 2


class B(Base):
    def __init__(self) -> None:
        super().__init__()
        self.b = 3


class C(A, B):
    def __init__(self) -> None:
        self.base = 1
        self.a = 2
        self.b = 3


print(C().a)
",
        "C0001",
        NOT_A_PREFIX,
    );
}

/// Single inheritance is never rejected, however deep the chain and however
/// many levels declare their own attributes: each ancestor's layout is a
/// literal prefix of its descendant's by construction.
#[test]
fn a_single_inheritance_chain_with_attributes_at_every_level_still_compiles() {
    assert_runs(
        "issue969_chain",
        "\
class A:
    def __init__(self) -> None:
        self.a = 1

    def get_a(self) -> int:
        return self.a


class B(A):
    def __init__(self) -> None:
        super().__init__()
        self.b = 2

    def get_b(self) -> int:
        return self.b


class C(B):
    def __init__(self) -> None:
        super().__init__()
        self.c = 3


c = C()
print(c.get_a())
print(c.get_b())
print(c.c)
",
        "1\n2\n3\n",
    );
}

/// A methods-only mixin declares no instance attributes, so its layout is
/// empty and the empty sequence is a prefix of everything.
#[test]
fn a_methods_only_mixin_second_base_still_compiles() {
    assert_runs(
        "issue969_mixin",
        "\
class A:
    def __init__(self) -> None:
        self.a = 4

    def get_a(self) -> int:
        return self.a


class Mixin:
    def doubled(self) -> int:
        return 2


class C(A, Mixin):
    pass


c = C()
print(c.get_a())
print(c.doubled())
",
        "4\n2\n",
    );
}

/// Two bases declaring the *same* attribute name share one slot, so both
/// layouts are `[used]` and the prefix test passes. This is the shape of
/// `tests/fixtures/pep_3135_super.py`'s `class Mixed(Slow, Fast)`, and it is
/// why the predicate is a name-sequence prefix test rather than a
/// "two slot-bearing bases" count.
#[test]
fn two_bases_declaring_the_same_attribute_name_still_compile() {
    assert_runs(
        "issue969_shared_name",
        "\
class Slow:
    def __init__(self) -> None:
        self.used = 1

    def which(self) -> int:
        return self.used


class Fast:
    def __init__(self) -> None:
        self.used = 2


class Mixed(Slow, Fast):
    def __init__(self) -> None:
        self.used = 3


print(Mixed().which())
",
        "3\n",
    );
}

/// A diamond whose intermediates declare nothing of their own: every layout in
/// the MRO is `[base]`, so every prefix test is an equality.
#[test]
fn a_diamond_with_attribute_free_intermediates_still_compiles() {
    assert_runs(
        "issue969_diamond_accept",
        "\
class Base:
    def __init__(self) -> None:
        self.base = 9

    def get_base(self) -> int:
        return self.base


class A(Base):
    def tag_a(self) -> int:
        return 1


class B(Base):
    def tag_b(self) -> int:
        return 2


class C(A, B):
    pass


c = C()
print(c.get_base())
print(c.tag_a())
print(c.tag_b())
",
        "9\n1\n2\n",
    );
}

/// A *one-sided* diamond is accepted: only `B` adds a slot of its own, so
/// `C`'s layout `[base, b]` has `B`'s own `[base, b]` and `A`'s `[base]` as
/// prefixes. This is the shape that distinguishes the real predicate -- a
/// name-sequence prefix test -- from the coarser "a diamond with two
/// slot-bearing branches" reading.
#[test]
fn a_one_sided_diamond_still_compiles() {
    assert_runs(
        "issue969_diamond_one_sided",
        "\
class Base:
    def __init__(self) -> None:
        self.base = 1

    def get_base(self) -> int:
        return self.base


class A(Base):
    pass


class B(Base):
    def __init__(self) -> None:
        super().__init__()
        self.b = 2

    def get_b(self) -> int:
        return self.b


class C(A, B):
    def __init__(self) -> None:
        self.base = 1
        self.b = 2


c = C()
print(c.get_base())
print(c.get_b())
",
        "1\n2\n",
    );
}

/// A class-attribute-only sibling base establishes no instance slot, so the
/// six shapes pinned by `tests/issue_960_sibling_base_class_attr.rs` are
/// untouched by this gate. The instance is bound to a name before the read
/// because #960's `T0044` read-side contract, which this change does not
/// touch, still rejects folding a class attribute out of a call expression.
#[test]
fn a_class_attribute_only_sibling_base_still_compiles() {
    assert_runs(
        "issue969_class_attr_base",
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
",
        "1\n",
    );
}
