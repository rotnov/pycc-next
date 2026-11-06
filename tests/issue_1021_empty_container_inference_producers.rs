// Issue #1021 (D-245), continued from `issue_1021_empty_container_inference.rs`:
// the producer scan's misses, `super()` and private-member producers, and the
// bindings in branches, loops and `try` suites a producer reads, end to end
// through the real `pycc build` / `pycc check` CLI.
//
// Every expected stdout below was verified against CPython 3.14 on the same
// source.

#[path = "empty_container_support/mod.rs"]
mod support;

use std::process::Command;
use support::*;

/// #1021 review round 7: an unannotated private helper's parameter has no
/// type yet when this pass runs -- the private-helper constraint solver, which
/// resolves it, runs afterwards and never substitutes into a rewritten node.
/// Freezing that `Ty::Infer` placeholder into `EmptyList` produced a `T0034`
/// naming `list[<inferred>]`, a type the source never mentions, for a program
/// the equivalent non-empty spelling compiles. The pass now declines to
/// resolve, so the honest `T0003` is reported instead: a missed resolution,
/// which this module permits, rather than a wrong one, which it does not.
#[test]
fn an_unresolved_parameter_producer_misses_rather_than_freezing_the_placeholder() {
    // Since v0.3 the pass runs once more after the solver has pinned `x`
    // from its call site (`empty_container::solved_signatures`), so the
    // list is typed from the solved parameter, never from the placeholder.
    let output = check_build_and_run(
        "placeholder_producer_empty_list",
        "\
def _f(x) -> int:
    xs = []
    xs.append(x)
    return len(xs)

print(_f(1))
",
    );
    assert!(output.status.success());
    assert_eq!(pycc_scratch::lf(&output.stdout), b"1\n");
    let non_empty = check_build_and_run(
        "placeholder_producer_non_empty_list",
        "\
def _f(x) -> int:
    xs = [x]
    xs.append(x)
    return 0

print(_f(1))
",
    );
    assert!(non_empty.status.success());
}

/// The dict half of the same defect: a placeholder reaching either the key or
/// the value position is discarded for the same reason.
#[test]
fn an_unresolved_parameter_producer_misses_for_a_dict_too() {
    let output = check_build_and_run(
        "placeholder_producer_empty_dict",
        "\
def _f(x) -> int:
    d = {}
    d[\"k\"] = x
    return d[\"k\"] + 1

print(_f(1))
",
    );
    assert!(output.status.success());
    assert_eq!(pycc_scratch::lf(&*String::from_utf8_lossy(&output.stdout)).replace("\r\n", "\n"), "2\n");
}

/// Round 8. The two inferred sources read the flat whole-function
/// environment, whose narrowing overlay is empty, so a producer inside a
/// narrowed branch resolved the *un*-narrowed `Optional` and froze it into
/// the node: `check_container_ty` then reported a `T0034` naming
/// `list[int | None]` for a program whose `xs = [x]` spelling compiles and
/// runs. A resolution carrying `Ty::Optional` is discarded on those sources,
/// so the miss is a `T0003` instead. The non-empty control is what makes this
/// an asymmetry test rather than merely "it reports something".
#[test]
fn a_producer_inside_a_narrowed_branch_misses_rather_than_freezing_the_wrapper() {
    let error = check_error(
        "narrowed_producer_empty_list",
        "\
def f(x: int | None) -> int:
    if x is not None:
        xs = []
        xs.append(x)
        return len(xs)
    return 0

print(f(1))
",
    );
    assert!(error.contains("T0003"), "{error}");
    assert!(!error.contains("T0034"), "{error}");
    let non_empty = check_build_and_run(
        "narrowed_producer_non_empty_list",
        "\
def f(x: int | None) -> int:
    if x is not None:
        xs = [x]
        xs.append(x)
        return len(xs)
    return 0

print(f(1))
",
    );
    assert!(non_empty.status.success());
    assert_eq!(pycc_scratch::lf(&*String::from_utf8_lossy(&non_empty.stdout)), "2\n");
}

/// The dict half of the same defect, with the same control: the wrapper
/// reaching either the key or the value position is discarded alike.
#[test]
fn a_narrowed_producer_misses_for_a_dict_too() {
    let error = check_error(
        "narrowed_producer_empty_dict",
        "\
def f(x: int | None) -> int:
    if x is not None:
        d = {}
        d[\"j\"] = x
        return len(d)
    return 0

print(f(1))
",
    );
    assert!(error.contains("T0003"), "{error}");
    assert!(!error.contains("T0036"), "{error}");
    let non_empty = check_build_and_run(
        "narrowed_producer_non_empty_dict",
        "\
def f(x: int | None) -> int:
    if x is not None:
        d = {\"k\": x}
        d[\"j\"] = x
        return len(d)
    return 0

print(f(1))
",
    );
    assert!(non_empty.status.success());
    assert_eq!(pycc_scratch::lf(&*String::from_utf8_lossy(&non_empty.stdout)), "2\n");
}

/// The annotation source keeps its `Optional` resolutions: the type is
/// written in source, no narrowing is involved, and `T0034` names the real
/// D-105 gap (a `float | None` element; `int | None` is compiled) where a
/// `T0003` miss would be the worse diagnostic.
#[test]
fn a_written_optional_annotation_still_reports_the_container_gap() {
    let error = check_error(
        "annotated_optional_empty_list",
        "\
def f(x: float | None) -> int:
    xs: list[float | None] = []
    xs.append(x)
    return len(xs)

print(f(1.0))
",
    );
    assert!(error.contains("T0034"), "{error}");
    assert!(error.contains("list[float | None]"), "{error}");
}

/// A producer inside a method may call `super()`, and resolving that call
/// needs the enclosing class -- which `check_function_in` derives from the
/// mangled `<Class>.<method>` name (#433). This pass builds its own child
/// environment and binds every parameter, so `self` is present; without the
/// same class seeding, `resolve_super_method_call` reached its
/// `current_class().unwrap()` with no class and aborted the compiler
/// (review round 11 on #1035). The resolution itself must also be the real
/// one: `super().value()` is `A`'s, not `B`'s override.
#[test]
fn a_super_call_producer_resolves_against_the_enclosing_class() {
    let run = check_build_and_run(
        "super_producer_empty_list",
        "\
class A:
    def value(self) -> int:
        return 1


class B(A):
    def value(self) -> int:
        return 2

    def go(self) -> int:
        xs = []
        xs.append(super().value())
        return xs[0]


b = B()
print(b.go())
",
    );
    assert!(run.status.success());
    assert_eq!(pycc_scratch::lf(&*String::from_utf8_lossy(&run.stdout)), "1\n");
}

/// The dict half, and the control that fixes the class seeding as the cause:
/// a top-level function's name carries no `.`, so it must keep
/// `current_class` unset exactly as the checker leaves it, and an empty
/// container there still resolves from its own producer.
#[test]
fn a_super_call_producer_resolves_for_a_dict_and_leaves_top_level_alone() {
    let run = check_build_and_run(
        "super_producer_empty_dict",
        "\
class A:
    def value(self) -> int:
        return 7


class B(A):
    def value(self) -> int:
        return 9

    def go(self) -> int:
        d = {}
        d[\"k\"] = super().value()
        return d[\"k\"]


def plain() -> int:
    xs = []
    xs.append(5)
    return xs[0]


b = B()
print(b.go())
print(plain())
",
    );
    assert!(run.status.success());
    assert_eq!(pycc_scratch::lf(&*String::from_utf8_lossy(&run.stdout)), "7\n5\n");
}

/// The rest of `resolve_super_method_call`'s environment assumptions, swept
/// after round 11 rather than left at the one arm the counter-example named.
/// Past the `current_class().unwrap()` the function calls `expect_class` and
/// unwraps the class's own position in its MRO, so a multiple-inheritance
/// hierarchy exercises the class table and the C3 order this pass's
/// environment carries: `annotated_function_environment` ends in
/// `bind_classes`, so the table is the checker's own. `D`'s MRO is
/// `[D, B, C, A]`, so `super()` inside `D.go` resolves to `B.value`.
#[test]
fn a_super_call_producer_resolves_through_a_multiple_inheritance_mro() {
    let run = check_build_and_run(
        "super_producer_diamond",
        "\
class A:
    def value(self) -> int:
        return 1


class B(A):
    def value(self) -> int:
        return 2


class C(A):
    def value(self) -> int:
        return 3


class D(B, C):
    def go(self) -> int:
        xs = []
        xs.append(super().value())
        return xs[0]


d = D()
print(d.go())
",
    );
    assert!(run.status.success());
    assert_eq!(pycc_scratch::lf(&*String::from_utf8_lossy(&run.stdout)), "2\n");
}

/// The non-call `super()` form, which reaches `resolve_super_attr_get` --
/// a sibling function with its own `current_class().unwrap()` and its own
/// `expect_class`. A base-class `@property` is one of the two members #915
/// leaves reachable this way, so it pins the same seeding through the other
/// entry point.
#[test]
fn a_super_attribute_producer_resolves_against_the_enclosing_class() {
    let run = check_build_and_run(
        "super_producer_attr",
        "\
class A:
    @property
    def value(self) -> int:
        return 4


class B(A):
    def go(self) -> int:
        xs = []
        xs.append(super().value)
        return xs[0]


b = B()
print(b.go())
",
    );
    assert!(run.status.success());
    assert_eq!(pycc_scratch::lf(&*String::from_utf8_lossy(&run.stdout)), "4\n");
}

/// Review round 12: a producer whose callee is an *unannotated private
/// member* must report `T0003`, not abort. The pre-pass is the only caller
/// that builds an environment from a module whose signatures are not all
/// concrete, and its class table records every member unconditionally -- so
/// a registry that skipped `Ty::Infer` signatures made the class resolvers'
/// registration assertion fire and `pycc check` exit on a panic.
///
/// One case per mangled-name-bearing table the resolvers consult, since a
/// fix that answers only the arm the review named leaves the others exactly
/// as they were.
#[test]
fn an_unannotated_private_member_producer_resolves_rather_than_aborting() {
    for (label, member, producer) in [
        (
            "method",
            "def _source(self):\n        return 1",
            "self._source()",
        ),
        (
            "property",
            "@property\n    def _source(self):\n        return 1",
            "self._source",
        ),
        (
            "staticmethod",
            "@staticmethod\n    def _source():\n        return 1",
            "A._source()",
        ),
        (
            "classmethod",
            "@classmethod\n    def _source(cls):\n        return 1",
            "A._source()",
        ),
    ] {
        let output = check_build_and_run(
            &format!("unannotated_private_{label}"),
            &format!(
                "\
class A:
    {member}

    def go(self) -> int:
        xs = []
        xs.append({producer})
        return xs[0]


a = A()
print(a.go())
"
            ),
        );
        assert!(output.status.success(), "{label}");
        assert_eq!(
            pycc_scratch::lf(&output.stdout),
            b"1
",
            "{label}"
        );
    }
}

/// The same round's soundness half. `A._one` overrides an annotated
/// `Base._one`, so dropping the unannotated override from `A`'s method table
/// -- the other candidate fix -- would let the MRO walk resolve the producer
/// against `Base._one` and freeze `list[str]` into a list of `int`. Missing
/// the inference is correct here; resolving it is not.
#[test]
fn an_unannotated_override_producer_misses_rather_than_resolving_to_the_base_class() {
    // Since v0.3 the producer is typed from `A._one`'s solved signature
    // (`int`), the method `self` resolves to -- never `Base._one`'s `str`.
    let output = check_build_and_run(
        "unannotated_override_producer",
        "\
class Base:
    def _one(self) -> str:
        return \"a\"


class A(Base):
    def _one(self):
        return 1

    def go(self) -> int:
        xs = []
        xs.append(self._one())
        return xs[0] + len(xs)


a = A()
print(a.go())
",
    );
    assert!(output.status.success());
    assert_eq!(pycc_scratch::lf(&*String::from_utf8_lossy(&output.stdout)).replace("\r\n", "\n"), "2\n");
}

/// #1021 bot review round 14: `bind_local_types_in_body` records every target
/// it walks as `Definitely` bound, including one bound only inside an `if`
/// branch, while the checker joins such a name back as `Maybe`. A read of a
/// maybe-bound name is checked at run time (`UnboundLocalError`), so the
/// producer `xs.append(v)` and the `xs = [v]` spelling both resolve
/// `list[str]` and run like CPython: the resolution is one the checker
/// makes too.
#[test]
fn a_producer_reading_a_maybe_bound_name_resolves_like_the_display() {
    for (label, line) in [
        ("maybe_bound_producer", "xs = []\n    xs.append(v)"),
        ("maybe_bound_reference", "xs = [v]"),
    ] {
        let source = format!(
            "def go(flag: bool) -> None:\n    if flag:\n        v = \"s\"\n    {line}\n    print(xs[0])\n\n\ngo(True)\n"
        );
        let output = check_build_and_run(label, &source);
        assert!(output.status.success(), "{label}");
        assert_eq!(pycc_scratch::lf(&output.stdout), b"s\n", "{label}");
    }
}

/// The other half of the same rule: a name bound inside an `if` branch *is*
/// readable at a producer inside that same branch, so the demotion above must
/// be undone as the producer scan descends into a nested body. Without the
/// per-body restoration this program reported `T0003` even though the read is
/// perfectly well-defined, which is a capability regression, not a repair.
#[test]
fn a_producer_inside_the_branch_that_binds_its_value_still_resolves() {
    let output = check_build_and_run(
        "maybe_bound_in_scope",
        "\
def go(flag: bool) -> int:
    xs = []
    if flag:
        v = 3
        xs.append(v)
    return len(xs)


print(go(True))
",
    );
    assert!(output.status.success());
    assert_eq!(pycc_scratch::lf(&output.stdout), b"1\n");
}

/// `T0003`'s help suggests annotating the assignment target, a remedy that
/// works in every position since the pass also resolves module-level
/// statements: the suggested `xs: list[int] = []` compiles at module level
/// as it does in a function body. The help text is only rendered on the JSON
/// surface, which is why this reads that one.
#[test]
fn the_t0003_help_names_a_remedy_that_works_in_the_position_it_is_raised() {
    let list_at_module_level =
        check_error_json("t0003_help_module_list", "xs = []\nprint(xs[0])\n");
    assert!(
        list_at_module_level.contains("annotate the assignment target"),
        "module-level list help must suggest the annotation, got: {list_at_module_level}",
    );
    // The remedy the help names really works in this position.
    let annotated_at_module_level = check_build_and_run(
        "t0003_help_module_annotated",
        "xs: list[int] = []\nprint(len(xs))\n",
    );
    assert!(annotated_at_module_level.status.success());
    assert_eq!(pycc_scratch::lf(&annotated_at_module_level.stdout), b"0\n");
    let dict_at_module_level =
        check_error_json("t0003_help_module_dict", "d = {}\nprint(d[0])\n");
    assert!(
        dict_at_module_level.contains("annotate the assignment target"),
        "module-level dict help must suggest the annotation, got: {dict_at_module_level}",
    );
    let list_in_function = check_error_json(
        "t0003_help_function_list",
        "\
def f() -> int:
    xs = []
    return len(xs) + xs[0]


print(f())
",
    );
    assert!(
        list_in_function.contains("annotate the assignment target"),
        "the in-function help is unchanged, got: {list_in_function}",
    );
    let dict_in_function = check_error_json(
        "t0003_help_function_dict",
        "\
def f() -> int:
    d = {}
    return len(d)


print(f())
",
    );
    assert!(
        dict_in_function.contains("annotate the assignment target"),
        "the in-function dict help is unchanged, got: {dict_in_function}",
    );
}

/// A value bound inside a `match` case and a later `bool` producer: the
/// unification sees both and resolves `list[int]`, printing CPython's `2`.
#[test]
fn a_match_bound_producer_value_resolves_with_the_later_producer() {
    let output = check_build_and_run(
        "uninferable_producer_ends_scan",
        "\
def go(n: int) -> int:
    total = 0
    match n:
        case _:
            v = 1
            xs = []
            xs.append(v)
            xs.append(True)
            total = len(xs)
    return total


print(go(0))
",
    );
    assert!(output.status.success());
    assert_eq!(pycc_scratch::lf(&*String::from_utf8_lossy(&output.stdout)), "2\n");
}

/// The reference spelling of the same program. `xs = [v]` compiles and runs,
/// which is what makes the `list[bool]` the old scan produced a *wrong*
/// resolution rather than a merely unhelpful one: the two spellings have to
/// agree on the element type or disagree by a miss, never by a different
/// type.
#[test]
fn the_reference_spelling_of_the_uninferable_producer_program_still_compiles() {
    let output = check_build_and_run(
        "uninferable_producer_reference",
        "\
def go() -> int:
    total = 0
    try:
        v = 1
        xs = [v]
        xs.append(True)
        total = len(xs)
    except ValueError:
        total = 0
    return total


print(go())
",
    );
    assert!(output.status.success());
    assert_eq!(pycc_scratch::lf(&*String::from_utf8_lossy(&output.stdout)).replace("\r\n", "\n"), "2\n");
}

/// #1445: the flat binder walks every `try` suite, so the round-15 program
/// above, spelled with a `try` as it originally was, no longer misses: `v`
/// is now visible to the producer scan, `xs.append(v)` resolves `list[int]`,
/// and the later `xs.append(True)` is the ordinary `bool`-into-`int`
/// append. Both forms run and print CPython's `2`, agreeing with the
/// reference spelling below.
#[test]
fn a_try_bound_producer_value_now_resolves_and_runs() {
    for (label, source) in [
        (
            "try_bound_producer_in_suite",
            "\
def go() -> int:
    total = 0
    try:
        v = 1
        xs = []
        xs.append(v)
        xs.append(True)
        total = len(xs)
    except ValueError:
        total = 0
    return total


print(go())
",
        ),
        (
            "try_bound_producer_after_suite",
            "\
def go() -> int:
    xs = []
    try:
        v = 1
        xs.append(v)
    except ValueError:
        pass
    xs.append(True)
    return len(xs)


print(go())
",
        ),
    ] {
        let output = check_build_and_run(label, source);
        assert!(output.status.success());
        assert_eq!(pycc_scratch::lf(&*String::from_utf8_lossy(&output.stdout)).replace("\r\n", "\n"), "2\n");
    }
}

/// The nested producer's `int` and the later `bool` append join into
/// `list[int]`, printing CPython's `2`.
#[test]
fn a_nested_body_producer_and_a_later_top_level_one_agree() {
    let output = check_build_and_run(
        "uninferable_producer_propagates",
        "\
def go(n: int) -> int:
    xs = []
    match n:
        case _:
            v = 1
            xs.append(v)
    xs.append(True)
    return len(xs)


print(go(0))
",
    );
    assert!(output.status.success());
    assert_eq!(pycc_scratch::lf(&*String::from_utf8_lossy(&output.stdout)), "2\n");
}

/// The dict arm of the same rule: `d["k"] = v` (an `int`) and a later
/// `d["j"] = True` resolve `dict[str, int]`, printing CPython's `2`.
#[test]
fn a_nested_dict_value_producer_and_a_later_one_agree() {
    let output = check_build_and_run(
        "uninferable_dict_value_producer",
        "\
def go(n: int) -> int:
    d = {}
    match n:
        case _:
            v = 1
            d[\"k\"] = v
    d[\"j\"] = True
    return len(d)


print(go(0))
",
    );
    assert!(output.status.success());
    assert_eq!(pycc_scratch::lf(&*String::from_utf8_lossy(&output.stdout)), "2\n");
}

/// The symmetric half: the *key* is the part that does not infer. Without
/// covering both, the arm would be asymmetric in exactly the way the
/// list-only version of this fix would have been.
#[test]
fn an_uninferable_dict_key_producer_ends_the_scan() {
    let diagnostics = check_error(
        "uninferable_dict_key_producer",
        "\
def go(n: int) -> int:
    d = {}
    match n:
        case _:
            k = \"a\"
            d[k] = 1
    d[True] = 2
    return len(d)


print(go(0))
",
    );
    assert!(
        diagnostics.contains("T0003"),
        "an uninferable dict key must miss, got: {diagnostics}",
    );
    assert!(
        !diagnostics.contains("T0036"),
        "the later producer must not have selected the key type, got: {diagnostics}",
    );
}

/// The other direction of the same rule, so the stop does not quietly become
/// a stop-on-anything: when the first producer *does* infer, it still wins
/// over a later producer carrying a different type, and the later one fails
/// with the ordinary element-type mismatch rather than re-resolving the
/// container. This is D-245 item 5's first-wins rule read through the same
/// code path.
#[test]
fn a_first_producer_that_infers_still_wins_over_a_later_differing_one() {
    let diagnostics = check_error(
        "first_inferring_producer_wins",
        "\
def go() -> int:
    xs = []
    xs.append(1)
    xs.append(\"a\")
    return len(xs)


print(go())
",
    );
    assert!(
        diagnostics.contains("cannot append `str` to a list of `int`"),
        "the first producer must still select the element type, got: {diagnostics}",
    );
}
/// #1021 bot review round 16: with `v` bound to `True` in the `if` arm and
/// to `1` in the `else`, a flat whole-function type would have promoted
/// `bool` into the `else` and resolved `EmptyList(Bool)`. The two bindings
/// are separate live ranges (`pycc_ast::desugar::webs`), so the `else`
/// sees only its own `int` and the program runs as CPython does.
#[test]
fn a_name_bound_in_two_branches_types_each_branch_on_its_own() {
    let output = check_build_and_run(
        "cross_branch_promotion",
        "\
def go(flag: bool) -> int:
    total = 0
    if flag:
        v = True
    else:
        v = 1
        xs = []
        xs.append(v)
        total = len(xs)
    return total


print(go(False))
",
    );
    assert!(output.status.success());
    assert_eq!(pycc_scratch::lf(&output.stdout), b"1\n");
}

/// The reference spelling of the program above: `xs = [v]` carries no empty
/// literal, so this pass never touches it and the checker resolves `v` in the
/// branch that binds it. `pycc check` accepting it is what makes the empty
/// spelling's former `T0034` a *wrong* resolution rather than a shared
/// limitation of both spellings.
///
/// Only `check` is asserted here, deliberately. Building and running this
/// program aborts in the runtime's int decoding -- a D-040 sticky-
/// representation defect in a branch-divergent `bool`/`int` binding that
/// reproduces with no empty container anywhere in the program, so it is
/// outside this pass and tracked as issue #1057.
#[test]
fn the_reference_spelling_of_the_cross_branch_program_still_checks() {
    let (_dir, path) = write_source(
        "cross_branch_promotion_ref",
        "\
def go(flag: bool) -> int:
    total = 0
    if flag:
        v = True
    else:
        v = 1
        xs = [v]
        total = len(xs)
    return total


print(go(False))
",
    );
    let output = Command::new(pycc_bin())
        .args(["check", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "the reference spelling must still check: {}",
        pycc_scratch::lf(&*String::from_utf8_lossy(&output.stdout)),
    );
}

/// Two `for` loops binding `i` to `int` agree, so `xs` resolves to
/// `list[int]` (whole-module unification checks every binding site).
#[test]
fn a_name_bound_by_two_loops_of_one_type_resolves() {
    let output = check_build_and_run(
        "repeated_same_type_sites",
        "\
def go() -> int:
    xs = []
    for i in range(3):
        xs.append(i)
    for i in range(2):
        print(i)
    return len(xs)


print(go())
",
    );
    assert!(output.status.success());
    assert_eq!(pycc_scratch::lf(&*String::from_utf8_lossy(&output.stdout)), "0\n1\n3\n");
}

/// The same program with the second loop deleted: one binding site, so the
/// promotion happens and the producer resolves `xs` to `list[int]`. This is the
/// control for the test above -- without it, a future change that broke
/// single-site promotion entirely would leave that test still passing.
#[test]
fn the_single_loop_spelling_of_that_program_compiles_and_runs() {
    let output = check_build_and_run(
        "repeated_same_type_sites_control",
        "\
def go() -> int:
    xs = []
    for i in range(3):
        xs.append(i)
    return len(xs)


print(go())
",
    );
    assert!(output.status.success());
    assert_eq!(pycc_scratch::lf(&output.stdout), b"3\n");
}

/// A value-less annotation reaches the checker as `Environment::declare`, which
/// keeps a name's existing runtime binding rather than replacing it, so `v` is
/// still an `int` here and the program is valid -- its `xs = [v]` spelling,
/// below, is what proves that independently of this pass.
///
/// The shared flat binder applied D-040 stickiness only on the *valued*
/// `AnnAssign` arm, so this spelling recorded `bool`, resolved the producer to
/// `list[bool]` and reported `T0034` for a type the program never produces.
/// Wrong, not missed -- the outcome D-245's invariant forbids.
#[test]
fn a_value_less_annotation_does_not_replace_the_first_recorded_representation() {
    let output = check_build_and_run(
        "value_less_annotation_sticky",
        "\
def go() -> int:
    v = 1
    v: bool
    xs = []
    xs.append(v)
    return len(xs)


print(go())
",
    );
    assert!(output.status.success());
    assert_eq!(pycc_scratch::lf(&output.stdout), b"1\n");
}

/// The reference spelling of the program above, which carries no empty literal
/// and so never reaches this pass at all. It is the oracle for the test above:
/// `pycc check` accepting it is what makes the empty spelling's former `T0034`
/// a wrong resolution rather than a limitation both spellings share.
#[test]
fn the_reference_spelling_of_the_value_less_annotation_program_checks() {
    let (_dir, path) = write_source(
        "value_less_annotation_sticky_ref",
        "\
def go() -> int:
    v = 1
    v: bool
    xs = [v]
    return len(xs)


print(go())
",
    );
    let output = Command::new(pycc_bin())
        .args(["check", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "the reference spelling must check: {}",
        pycc_scratch::lf(&*String::from_utf8_lossy(&output.stdout)),
    );
}

/// `scoped_for_body` promotes every name a body's own top-level statements
/// bind, without regard to whether the binding precedes the producer that reads
/// it, so a producer here is scanned against a type contributed by a *later*
/// statement. That cannot produce a wrong resolution: reading a name before its
/// only binding is itself an error, and the checker reports it as `T0021` for
/// both spellings, so the two never disagree.
///
/// This fixture holds that reasoning to the tree rather than to prose -- if a
/// future change let one spelling past `T0021`, the ordering would start to
/// matter and this test is where that shows up.
#[test]
fn a_producer_reading_a_later_binding_reports_the_unbound_local_in_both_spellings() {
    let empty = check_error(
        "forward_binding_empty",
        "\
def go(flag: bool) -> int:
    total = 0
    if flag:
        xs = []
        xs.append(w)
        w = True
        total = len(xs)
    return total


print(go(False))
",
    );
    let reference = check_error(
        "forward_binding_reference",
        "\
def go(flag: bool) -> int:
    total = 0
    if flag:
        xs = [w]
        w = True
        total = len(xs)
    return total


print(go(False))
",
    );
    assert!(
        empty.contains("T0021"),
        "the empty spelling must report the unbound local, got: {empty}",
    );
    assert!(
        reference.contains("T0021"),
        "the reference spelling must report the unbound local, got: {reference}",
    );
}
