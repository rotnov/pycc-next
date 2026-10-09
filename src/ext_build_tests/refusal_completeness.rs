//! The §3.3 completeness guard for D-244 rule 7's refusal set: every type
//! the `ext` boundary admits at a parameter position emits a refusal arm,
//! and the shim declares exactly the unpack helpers those arms name.
//!
//! `tests/issue_1067_neg004_ext_conformance.rs` pins what a refusal *says*
//! to a host, one shape at a time, through a real CPython. It cannot say
//! that no admitted type was left without an arm at all -- a new carrier
//! added without a refusal path would simply not appear in its table. This
//! file is the closed-set half of that statement, and it is deliberately
//! non-`#[ignore]`d: the coverage job runs `llvm-cov` without
//! `--include-ignored`, so this is where the lines of this change that sit
//! inside `scripts/check_diff_coverage.py`'s denominator are executed.

use super::*;
use std::collections::BTreeSet;

/// Every `pycc_ext_unpack_*` suffix the tracked shim mentions.
///
/// Scanned out of the shim text rather than listed by hand on both sides:
/// the point of the assertion below is that the two sets agree, which a
/// second hand-written list could not establish. The suffix runs to the
/// first character a C identifier cannot continue with, so
/// `pycc_ext_unpack_int_at` yields `int_at` and never a spurious `int`.
fn shim_unpack_suffixes() -> BTreeSet<String> {
    const PREFIX: &str = "pycc_ext_unpack_";
    let shim = shim_c();
    let mut found = BTreeSet::new();
    for (start, _) in shim.match_indices(PREFIX) {
        let rest = &shim[start + PREFIX.len()..];
        let end = rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '_');
        found.insert(rest[..end.unwrap_or(rest.len())].to_string());
    }
    found
}

/// The exact refusal arms one argument slot emits, from each `if` up to
/// and including the `!= 0) {` that opens its failure block.
///
/// The `match` carries no wildcard arm on purpose. A third
/// [`BoundaryCarrier`] variant -- a new shape the boundary admits -- then
/// fails to compile here instead of silently inheriting a scalar's single
/// arm, which is the whole completeness property this module exists to
/// state. `docs/TESTING.md`'s coverage notes forbid the catch-all that
/// would otherwise absorb it unexercised.
fn refusal_arms(carrier: &BoundaryCarrier, name: &str, index: usize) -> Vec<String> {
    match carrier {
        BoundaryCarrier::Scalar(_, helper) => vec![format!(
            "if (pycc_ext_unpack_{helper}(args[{index}], \"{name}\", {index}, &a{index}) != 0) {{"
        )],
        BoundaryCarrier::Tuple(elements) => {
            let len = elements.len();
            let head = format!(
                "if (pycc_ext_unpack_tuple(args[{index}], \"{name}\", {index}, {len}) != 0) {{"
            );
            let arms = elements.iter().enumerate().map(|(element, (_, helper))| {
                format!(
                    "if (pycc_ext_unpack_{helper}_at(PyTuple_GetItem(args[{index}], {element}), \
                     \"{name}\", {index}, {element}, &a{index}_{element}) != 0) {{"
                )
            });
            std::iter::once(head).chain(arms).collect()
        }
        // One arm, exactly like a scalar's, but the helper is fixed rather
        // than carried: `BoundaryCarrier::Buffer` names no suffix, because
        // a `memoryview` has no packer and so no symmetric `pycc_ext_*`
        // pair to name. The four run-time checks the helper applies
        // (exports a buffer, C-contiguous, `ndim == 1`, format `"d"`) are all
        // inside it, so this one arm is the whole refusal at this slot.
        // `0` is the writability argument: every program in this
        // harness only reads its buffer parameter, so Part 1 of #1142's
        // bit is off and the request stays read-only.
        BoundaryCarrier::Buffer { .. } => vec![format!(
            "if (pycc_ext_unpack_memoryview(args[{index}], \"{name}\", {index}, 0, &b{index}) \
             != 0) {{"
        )],
        // Part 1 of #1447 (#1449): one arm, a scalar's shape plus the class
        // the shim checks the instance's run-time MRO against.
        BoundaryCarrier::Instance(class) => vec![format!(
            "if (pycc_ext_unpack_instance(args[{index}], \"{name}\", {index}, \"{class}\", \
             &a{index}) != 0) {{"
        )],
    }
}

/// The text of the one call an export's wrapper makes into compiled code.
///
/// Two spellings, because #1050 gave a `tuple`-carrying signature a thunk:
/// a scalar-only export calls through the `fnptr_<name>` global cast to a
/// function-pointer type, and every other one calls the thunk symbol
/// directly. Both are matched up to their opening parenthesis, which is
/// what makes the position below the call's and not a declaration's --
/// the `extern` declaration of either spells its parameter *types* there,
/// never `a0`.
fn call_site(name: &str, params: &[(&str, Ty)], return_ty: &Ty) -> String {
    let types: Vec<Ty> = params.iter().map(|(_, ty)| ty.clone()).collect();
    let first = match boundary_carrier(&types[0]).expect("an admitted argument type") {
        BoundaryCarrier::Scalar(..) | BoundaryCarrier::Instance(_) => "a0".to_string(),
        BoundaryCarrier::Tuple(_) => "a0_0".to_string(),
        BoundaryCarrier::Buffer { .. } => "&a0".to_string(),
    };
    // `false`: this walk enumerates *declared* signatures, and the buffer
    // sub-range fact (#1179) is a property of a body, not of a signature.
    if pycc_codegen::ext_thunk_required(name, &types, return_ty, false) {
        format!("{}({first}", pycc_codegen::ext_thunk_symbol(name))
    } else {
        format!(")fnptr_{name})({first}")
    }
}

/// Asserts that `arm` appears in `inc` and that the block it opens returns
/// `NULL` before it closes -- a refusal arm that fell through to the call
/// would contain the arm text just the same.
fn assert_arm_refuses(inc: &str, arm: &str) {
    assert!(inc.contains(arm), "no {arm} in\n{inc}");
    let tail = &inc[inc.find(arm).expect("just asserted") + arm.len()..];
    let end = tail.find("\n    }").expect("the arm block is closed");
    assert!(tail[..end].contains("return NULL;"), "{inc}");
}

#[test]
fn the_shim_declares_exactly_the_unpack_helpers_the_boundary_refuses_through() {
    let expected: BTreeSet<String> = [
        "bool",
        "bool_at",
        "float",
        "float_at",
        "instance",
        "int",
        "int_at",
        "memoryview",
        "object",
        "str",
        "tuple",
    ]
    .iter()
    .map(|suffix| (*suffix).to_string())
    .collect();
    assert_eq!(shim_unpack_suffixes(), expected);
}

#[test]
fn every_admitted_argument_type_refuses_before_the_call_and_after_the_arity_check() {
    // One row per type the boundary admits at a parameter position, and so
    // at least one row per `BoundaryCarrier` variant: D-116 fixes a tuple's
    // element types to exactly the three below, so the `_at` helpers are
    // covered here as well. Each export takes two arguments of its row's
    // type, which is what makes "every slot" more than "the first slot".
    let rows: Vec<(&str, Ty)> = vec![
        ("take_int", Ty::Int),
        ("take_float", Ty::Float),
        ("take_bool", Ty::Bool),
        ("take_str", Ty::Str),
        (
            "take_tuple_int",
            Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])),
        ),
        ("take_tuple_float", Ty::Tuple(Box::new(vec![Ty::Float]))),
        ("take_tuple_bool", Ty::Tuple(Box::new(vec![Ty::Bool]))),
        // Part 1 of #1027's third carrier. Two parameters of it is the row
        // that matters most: the bail cleanup for argument 2's refusal has
        // to release argument 1's already-acquired `Py_buffer`, which the
        // one-parameter shape cannot state at all.
        ("take_memoryview", Ty::MemoryView),
        // D-258 rule 5 (#1397): the opaque object, a scalar-shaped slot
        // whose helper admits every object but still sits behind the same
        // `!= 0` arm, so the wrapper's shape stays uniform.
        ("take_object", Ty::Object),
        // Part 1 of #1447 (#1449): a regular class compiled in this module,
        // which the module below defines -- the admission is narrowed by
        // the module's class table, not by the type alone.
        ("take_instance", Ty::Instance(Box::new("Conf".to_string()))),
    ];
    for (name, ty) in rows {
        let params = [("a", ty.clone()), ("b", ty.clone())];
        let hir = module_with_classes(
            vec![func(name, &params, Ty::Int)],
            vec![("Conf".to_string(), class_def("Conf", None))],
        );
        let exports = collect_exports(&hir).expect("every row is a carriable signature");
        let carrier = boundary_carrier(&ty).expect("every row is an admitted argument type");
        let inc = generate_exports_inc("m", &exports, &[], &[], &[], &[]);
        for index in 0..params.len() {
            for arm in refusal_arms(&carrier, name, index) {
                assert_arm_refuses(&inc, &arm);
            }
        }
        // Rule 7's "before the compiled body runs" is an ordering claim
        // about the *last* argument, not only the first: an export whose
        // argument 1 conforms and whose argument 2 does not must still
        // never reach the call. Asserting that each arm exists and returns
        // leaves that unstated -- the call moved up between two arms
        // satisfies every assertion above -- so pin the call itself behind
        // the last unpack the wrapper emits.
        let call = call_site(name, &params, &Ty::Int);
        let call_at = inc.find(&call).expect("the compiled call");
        let last_unpack = inc
            .rfind("pycc_ext_unpack_")
            .expect("at least one refusal arm");
        assert!(last_unpack < call_at, "{inc}");
        // Rule 7 refuses a non-conforming call "before the compiled body
        // runs", and a wrong argument *count* is the one shape that cannot
        // be diagnosed per slot: reading `args[1]` of a one-argument call
        // is out of bounds, not a `TypeError`. So the arity check precedes
        // every unpack, and this is the assertion that says so.
        let arity = inc.find("if (nargs != 2)").expect("the arity check");
        let first = inc
            .find("pycc_ext_unpack_")
            .expect("at least one refusal arm");
        assert!(arity < first, "{inc}");
    }
}

/// Whether the boundary is expected to carry `ty` at a parameter position,
/// stated here once for every variant of [`Ty`] with no wildcard arm.
///
/// [`boundary_carrier`] ends in `_ => None`, so the closed-set assertions
/// above guard only the `BoundaryCarrier` axis: a new `Ty` admitted by a
/// new arm there would map to an existing carrier shape and leave both the
/// hand-maintained row list and the shim's helper set untouched. This
/// function is the other axis. Adding a variant to `Ty` fails to compile
/// here, and admitting an existing one flips an assertion below -- either
/// way the change is stated rather than inherited.
fn expected_to_carry(ty: &Ty) -> bool {
    match ty {
        Ty::Int | Ty::Float | Ty::Bool | Ty::Str => true,
        // D-116 fixes a tuple's element types to the three scalars a
        // `pycc_ext_unpack_*_at` helper exists for: `str` is a single C
        // slot at a top-level position but has no element shim, and a
        // nested tuple is not a single slot at all.
        Ty::Tuple(elements) => elements
            .iter()
            .all(|element| matches!(element, Ty::Int | Ty::Float | Ty::Bool)),
        Ty::None
        | Ty::Infer
        | Ty::Param(_)
        | Ty::List(_)
        | Ty::Dict(_)
        | Ty::Set(_)
        | Ty::FrozenSet(_)
        | Ty::Protocol(_)
        | Ty::Optional(_) => false,
        // Part 1 of #1027: admitted at a parameter position, and refused at
        // a return position by `into_scalar` answering `None` for its
        // carrier -- which is the asymmetry this predicate deliberately
        // does not model, since it answers only the parameter question.
        Ty::MemoryView => true,
        // D-258 rule 5 (#1397), replacing Part 1 of #1026's refusal: the
        // opaque CPython object crosses as the `PyObject *` itself.
        Ty::Object => true,
        // Part 1 of #1447 (#1449): every compiled-class instance has a
        // carrier shape. Which class names a module may actually carry is
        // `unsupported_boundary_ty`'s narrowing against the module's class
        // table, which this type-only predicate deliberately does not
        // model -- see `an_instance_is_admitted_only_for_a_regular_class_of_the_module`.
        Ty::Instance(_) => true,
    }
}

#[test]
fn no_type_outside_the_admitted_set_is_carried_at_a_parameter_position() {
    let name = || Box::new("T".to_string());
    let samples: Vec<Ty> = vec![
        Ty::Int,
        Ty::Float,
        Ty::Bool,
        Ty::Str,
        Ty::None,
        Ty::Infer,
        Ty::Param(name()),
        Ty::List(Box::new(Ty::Int)),
        Ty::Dict(Box::new((Ty::Str, Ty::Int))),
        Ty::Set(Box::new(Ty::Int)),
        Ty::FrozenSet(Box::new(Ty::Int)),
        Ty::Instance(name()),
        Ty::Protocol(name()),
        Ty::Optional(Box::new(Ty::Int)),
        Ty::Object,
        Ty::MemoryView,
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Float, Ty::Bool])),
        // No `_at` element shim, exactly like `tuple[str]` below.
        Ty::Tuple(Box::new(vec![Ty::MemoryView])),
        // Nor for D-258's object (#1397): carried at a top-level slot only.
        Ty::Tuple(Box::new(vec![Ty::Object])),
        // Nor for a compiled-class instance (Part 1 of #1447).
        Ty::Tuple(Box::new(vec![Ty::Instance(name())])),
        Ty::Optional(Box::new(Ty::Instance(name()))),
        // The two element shapes the boundary refuses: neither has an
        // `_at` helper, and both are unreachable from source today only
        // because `T0039` refuses the annotation first.
        Ty::Tuple(Box::new(vec![Ty::Str])),
        Ty::Tuple(Box::new(vec![Ty::Tuple(Box::new(vec![Ty::Int]))])),
    ];
    for ty in samples {
        assert_eq!(carries_param(&ty), expected_to_carry(&ty), "{ty:?}");
    }
}
