//! Issue #146 Part 1: public-CLI coverage for heap-bigint refcounting at
//! named-storage and loop-induction sites.
//!
//! D-058 originally leaked every `BigIntObj` forever. #147 (D-179) widened
//! that leak to one object per `range` iteration, which is what makes it
//! observable as unbounded process growth rather than a bounded overflow-path
//! concession. Part 1 gives `BigIntObj` a refcount and releases it at the two
//! site families where a *named* storage location stops referring to a word:
//! ordinary assignment targets and `for`/comprehension induction variables.
//!
//! Two families live here:
//!
//! * **Value correctness under release.** Every shape below would print the
//!   wrong value, or crash, if a release fired while a live name still
//!   referred to the word. The aliasing shapes are the interesting ones: a
//!   `range` whose bound is a *named local* makes the loop's `start_v`,
//!   `stop_v`, and the visible induction target all alias one heap object,
//!   so an unbalanced release there is a use-after-free rather than a leak.
//! * **Peak-RSS ratio.** A missed *release* leaves the leak in place and is
//!   invisible to every value assertion above. The ratio test at the bottom
//!   of this file is the one that fails loudly for it.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

/// `2^62`, the smallest magnitude that does not round-trip through D-061's
/// tagged 63-bit encoding and therefore always allocates a `BigIntObj`.
const PROMOTED: &str = "4611686018427387904";

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

/// Writes `source` into a fresh scratch directory and returns the directory
/// handle together with the source path. The caller must keep the handle
/// alive while the path is in use -- dropping it removes the directory.
fn write_case(case: &str, source: &str) -> (ScratchDir, std::path::PathBuf) {
    let dir = ScratchDir::new(&format!("issue146_{case}")).expect("failed to create scratch dir");
    let src = dir.join("case.py");
    std::fs::File::create(&src)
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    (dir, src)
}

/// Runs `source` through `pycc run` and asserts it exits `0` with exactly
/// `expected` on stdout.
fn assert_runs_and_prints(case: &str, source: &str, expected: &str) {
    let (_dir, src) = write_case(case, source);
    let run = Command::new(pycc_bin())
        .args(["run", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        run.status.code(),
        Some(0),
        "{case} should run to completion, got {:?}: {}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        expected,
        "{case} printed the wrong output"
    );
}

// ---------------------------------------------------------------------------
// Aliasing: a `range` bound that is a named local.
//
// Every pre-existing bigint-`range` artifact (`tests/issue_147_bigint_range.
// rs`, `tests/fixtures/bigint_range.py`) uses *literal* bounds. A literal
// bound is materialized fresh at rc=1 per evaluation and is referred to by no
// name, so its birth reference silently absorbs one extra release -- a
// `start_v` double-release would still pass every one of those tests. These
// shapes are the ones that do not absorb it: the bound outlives the loop
// under a name, so it must still be printable afterwards.
// ---------------------------------------------------------------------------

#[test]
fn a_named_bigint_start_bound_survives_the_loop_that_consumes_it() {
    // On iteration 1 the induction phi's incoming value *is* `start_v`, which
    // is *also* the word held by `b`'s storage slot. Three names for one
    // object; exactly one of them (`b`'s slot) still owns a reference when
    // the loop ends.
    assert_runs_and_prints(
        "named_start_bound",
        &format!(
            "b: int = {PROMOTED}\nfor i in range(b, 4611686018427387907):\n    print(i)\nprint(b)\n"
        ),
        "4611686018427387904\n4611686018427387905\n4611686018427387906\n4611686018427387904\n",
    );
}

#[test]
fn a_named_bigint_bound_used_for_both_start_and_stop_survives_an_empty_loop() {
    // `range(b, b)` is empty, so the loop body never runs and the induction
    // phi is only ever `start_v`. Both normalized operands alias `b`'s single
    // heap object, so the preheader retains it twice and `after_bb` must
    // release it exactly twice -- one release too many frees a word `b` still
    // names, and the trailing `print(b)` reads freed memory.
    assert_runs_and_prints(
        "aliased_empty_range",
        &format!("b: int = {PROMOTED}\nfor i in range(b, b):\n    print(i)\nprint(b)\n"),
        "4611686018427387904\n",
    );
}

#[test]
fn a_named_bigint_bound_used_for_start_stop_and_step_survives() {
    // The third operand joins the alias set. `step` must be non-zero, so this
    // uses `range(b, b, b)`: an empty loop again, now with three normalized
    // operands all naming the same object.
    assert_runs_and_prints(
        "aliased_three_operands",
        &format!("b: int = {PROMOTED}\nfor i in range(b, b, b):\n    print(i)\nprint(b)\n"),
        "4611686018427387904\n",
    );
}

#[test]
fn a_bigint_loop_variable_reassigned_to_a_smallint_inside_the_body_still_iterates() {
    // Overwriting the visible target inside the body releases the bigint the
    // bind just retained. The induction phi keeps its own reference, so the
    // loop must keep advancing from the right value regardless.
    assert_runs_and_prints(
        "reassign_bound_target",
        &format!(
            "for i in range({PROMOTED}, 4611686018427387907):\n    print(i)\n    i = 5\n    print(i)\n"
        ),
        "4611686018427387904\n5\n4611686018427387905\n5\n4611686018427387906\n5\n",
    );
}

// ---------------------------------------------------------------------------
// Named storage: assignment, globals, parameters, returns, attributes.
// ---------------------------------------------------------------------------

#[test]
fn repeatedly_overwriting_a_named_bigint_local_keeps_the_live_value_intact() {
    assert_runs_and_prints(
        "overwrite_named_local",
        &format!("x: int = {PROMOTED}\nx = x + 1\nx = x + 1\nprint(x)\n"),
        "4611686018427387906\n",
    );
}

#[test]
fn aliasing_a_named_bigint_local_keeps_both_names_readable() {
    // `y = x` is the duplicate-reference shape D-060 already handles for
    // `str`: the retain on the source is what keeps `x` alive once `x` is
    // itself overwritten.
    assert_runs_and_prints(
        "alias_named_local",
        &format!("x: int = {PROMOTED}\ny: int = x\nx = 0\nprint(y)\n"),
        "4611686018427387904\n",
    );
}

#[test]
fn a_bigint_module_global_survives_being_read_from_a_function() {
    assert_runs_and_prints(
        "global_read_from_function",
        &format!(
            "g: int = {PROMOTED}\n\ndef read() -> int:\n    return g\n\nprint(read())\nprint(g)\n"
        ),
        "4611686018427387904\n4611686018427387904\n",
    );
}

#[test]
fn a_bigint_passed_as_an_argument_survives_the_call() {
    assert_runs_and_prints(
        "argument_survives_call",
        &format!(
            "def twice(v: int) -> int:\n    return v + v\n\nx: int = {PROMOTED}\nprint(twice(x))\nprint(x)\n"
        ),
        "9223372036854775808\n4611686018427387904\n",
    );
}

#[test]
fn a_walrus_wrapped_bigint_passed_as_an_argument_survives_the_call() {
    // PEP 572 (#774): `y := PROMOTED` stores into `y`'s slot and yields a
    // *borrow* of that same slot's reference, not a fresh one -- so passing
    // it directly as a call argument needs `retain_if_int_duplicate` to
    // retain it for the duration of the call, exactly like passing a plain
    // `Name` (see `a_bigint_passed_as_an_argument_survives_the_call` above).
    // Without that retain, the call's own argument release would retire the
    // slot's only reference, and the later `print(y)` would read a freed
    // `BigIntObj`.
    assert_runs_and_prints(
        "walrus_argument_survives_call",
        &format!(
            "def twice(v: int) -> int:\n    return v + v\n\ny = 0\nprint(twice(y := {PROMOTED}))\nprint(y)\n"
        ),
        "9223372036854775808\n4611686018427387904\n",
    );
}

#[test]
fn a_bigint_returned_from_a_function_survives_the_return() {
    assert_runs_and_prints(
        "return_survives",
        &format!("def make() -> int:\n    v: int = {PROMOTED}\n    return v\n\nprint(make())\n"),
        "4611686018427387904\n",
    );
}

#[test]
fn a_bigint_stored_in_an_instance_attribute_survives_a_later_overwrite() {
    assert_runs_and_prints(
        "attribute_overwrite",
        &format!(
            "from dataclasses import dataclass\n\n@dataclass\nclass Box:\n    v: int\n\nb = Box({PROMOTED})\nprint(b.v)\nb.v = b.v + 1\nprint(b.v)\n"
        ),
        "4611686018427387904\n4611686018427387905\n",
    );
}

#[test]
fn a_bool_assigned_into_a_bigint_valued_int_slot_releases_the_old_word() {
    // `value.ty()` here is `Ty::Bool`, not `Ty::Int`: the release has to be
    // gated on the *slot's* declared type or this store silently skips it.
    // `coerce_scalar_to_type` turns the `i8` into D-141's encoded word `6`.
    assert_runs_and_prints(
        "bool_into_int_slot",
        &format!("x: int = {PROMOTED}\nx = True\nprint(x)\n"),
        // D-141 keeps bool identity through the int-encoded word, so this
        // prints `True`, not `1`.
        "True\n",
    );
}

#[test]
fn a_bool_assigned_into_a_bigint_valued_int_attribute_slot_releases_the_old_word() {
    // #627/D-187, the attribute-slot mirror of the local-slot case just
    // above. `MirStmt::AttrSet` carries only a slot *index*, so codegen
    // gates its release on the value's `Ty`; `pycc_mir` now widens a
    // `bool` bound for an `int`-declared attribute through
    // `MirExpr::IntBoundary`, which both encodes the D-141 word and makes
    // the value report `Ty::Int` so that gate fires (D-180 Consequences
    // item 6).
    //
    // Before that widening this program did not merely leak: the raw
    // `bool` word `0` is not a valid encoded int word, so `print(h.n)`
    // after `h.n = False` aborted with
    // `pycc_rt: invalid encoded int word 0x0` (exit 134).
    assert_runs_and_prints(
        "bool_into_int_attribute_slot",
        &format!(
            "class Holder:\n    def __init__(self) -> None:\n        self.n = 0\n\n\nh = Holder()\nh.n = {PROMOTED}\nprint(h.n)\nh.n = True\nprint(h.n)\nh.n = False\nprint(h.n)\n"
        ),
        // D-141 keeps bool identity through the int-encoded word.
        &format!("{PROMOTED}\nTrue\nFalse\n"),
    );
}

// ---------------------------------------------------------------------------
// Comprehensions: the same induction-variable ownership, three more emitters.
// ---------------------------------------------------------------------------

#[test]
fn a_list_comprehension_over_a_named_bigint_bound_leaves_the_bound_intact() {
    assert_runs_and_prints(
        "list_comp_named_bound",
        &format!(
            "b: int = {PROMOTED}\nxs = [0 for i in range(b, 4611686018427387906)]\nprint(len(xs))\nprint(b)\n"
        ),
        "2\n4611686018427387904\n",
    );
}

#[test]
fn a_set_comprehension_over_a_named_bigint_bound_leaves_the_bound_intact() {
    assert_runs_and_prints(
        "set_comp_named_bound",
        &format!(
            "b: int = {PROMOTED}\nxs = {{0 for i in range(b, 4611686018427387906)}}\nprint(len(xs))\nprint(b)\n"
        ),
        "1\n4611686018427387904\n",
    );
}

#[test]
fn a_dict_comprehension_over_a_named_bigint_bound_leaves_the_bound_intact() {
    assert_runs_and_prints(
        "dict_comp_named_bound",
        &format!(
            "b: int = {PROMOTED}\nxs = {{\"k\": 0 for i in range(b, 4611686018427387906)}}\nprint(len(xs))\nprint(b)\n"
        ),
        "1\n4611686018427387904\n",
    );
}

// ---------------------------------------------------------------------------
// Peak-RSS ratio: the test that actually fails for a *missed release*.
//
// Deliberately a ratio and never an absolute bound: `rusage::ru_maxrss` is
// bytes on macOS/BSD and kilobytes on Linux, and the baseline footprint of a
// pycc binary differs per platform and per libc. A leak of one `BigIntObj`
// per iteration is linear in the trip count, so doubling the iterations
// doubles the peak. A fixed program's peak is flat.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// #146 Part 2 (D-181): unbound arithmetic temporaries.
//
// Every shape below evaluates an `int` value that no name ever holds, at a
// site that consumes the word and discards it. Part 2 releases the birth
// reference at each of those sites, so each of these would be a
// use-after-free (wrong value, or a crash) if a release fired on a word that
// was really borrowed, and a leak -- invisible here, caught by the peak-RSS
// gates at the bottom of this file -- if one were missing.
// ---------------------------------------------------------------------------

/// The two `BinOp` operand release sites, in the two shapes that differ in
/// how the operands are produced: a borrowed name plus a folded immediate,
/// and a borrowed name aliased on both sides.
#[test]
fn nested_bigint_arithmetic_temporaries_produce_the_right_value() {
    assert_runs_and_prints(
        "nested_temporaries",
        &format!(
            "x: int = {PROMOTED}\ny: int = (x + 1) + 2\nprint(y)\nprint((x + x) + x)\nprint(x)\n"
        ),
        "4611686018427387907\n13835058055282163712\n4611686018427387904\n",
    );
}

/// A bigint *literal* operand: outside D-061's tagged range, so
/// `int_const::emit_int_constant` allocates one `BigIntObj` per evaluation
/// rather than folding an immediate. Evaluated twice so a release that
/// freed a still-shared object would corrupt the second read.
#[test]
fn a_bigint_literal_operand_is_released_without_corrupting_a_later_one() {
    assert_runs_and_prints(
        "literal_operand",
        &format!("print({PROMOTED} + 1)\nprint({PROMOTED} + 2)\n"),
        "4611686018427387905\n4611686018427387906\n",
    );
}

/// `print`'s own argument and an f-string interpolation: both consume the
/// word through `to_str` and discard it.
#[test]
fn a_bigint_temporary_printed_directly_and_interpolated_survives_to_str() {
    assert_runs_and_prints(
        "print_and_fstring",
        &format!("x: int = {PROMOTED}\nprint(x + 1)\nprint(f\"{{x + 1}}\")\nprint(x)\n"),
        "4611686018427387905\n4611686018427387905\n4611686018427387904\n",
    );
}

/// `MirStmt::ExprStmt`: the value is discarded outright.
#[test]
fn a_bigint_temporary_in_statement_position_is_discarded_safely() {
    assert_runs_and_prints(
        "expr_stmt",
        &format!("x: int = {PROMOTED}\nx + 1\nx + x\nprint(x)\n"),
        "4611686018427387904\n",
    );
}

/// The `MirStmt::If` condition site. A heap bigint is always truthy, so the
/// interesting half is that `truthy` still reads the word: the release is
/// emitted *after* it for exactly that reason.
#[test]
fn a_bigint_temporary_used_as_an_if_condition_is_read_before_it_is_released() {
    assert_runs_and_prints(
        "if_condition",
        &format!("x: int = {PROMOTED}\nif x + 1:\n    print(7)\nprint(x)\n"),
        "7\n4611686018427387904\n",
    );
}

/// The `MirStmt::While` condition site, entered once and then falsified.
/// `x - <bigint literal>` is re-evaluated every trip, so the literal
/// operand's release has to be correct on both the truthy and the falsey
/// pass.
#[test]
fn a_bigint_temporary_used_as_a_while_condition_is_re_evaluated_safely() {
    assert_runs_and_prints(
        "while_condition",
        &format!(
            "x: int = 9223372036854775807\nn: int = 0\nwhile x - {PROMOTED}:\n    \
             n = n + 1\n    x = {PROMOTED}\nprint(n)\nprint(x)\n"
        ),
        "1\n4611686018427387904\n",
    );
}

/// The three comprehension `if`-filter condition sites.
#[test]
fn a_bigint_temporary_used_as_a_comprehension_filter_is_read_before_release() {
    assert_runs_and_prints(
        "comprehension_filter",
        &format!(
            "x: int = {PROMOTED}\nn: int = 3\nxs = [0 for i in range(n) if x + 1]\n\
             ss = {{1 for i in range(n) if x + 1}}\nds = {{\"k\": 2 for i in range(n) if x + 1}}\n\
             print(len(xs))\nprint(len(ss))\nprint(len(ds))\nprint(x)\n"
        ),
        "3\n1\n1\n4611686018427387904\n",
    );
}

/// Freshly built `range` bounds, in a `for` statement and in all three
/// comprehension emitters. `pycc_rt_range_continue` re-reads `stop`/`step`
/// on every trip, so a bound released at the point it is built would be a
/// use-after-free on iteration two -- the comprehension emitters carry
/// theirs out to `after_bb` for exactly that reason.
#[test]
fn freshly_built_bigint_range_bounds_survive_every_iteration() {
    assert_runs_and_prints(
        "fresh_range_bounds",
        &format!(
            "x: int = {PROMOTED}\nn: int = 0\nfor i in range(x + 0, x + 3):\n    n = n + 1\n\
             xs = [0 for i in range(x + 0, x + 3)]\nss = {{1 for i in range(x + 0, x + 3)}}\n\
             ds = {{\"k\": 2 for i in range(x + 0, x + 3)}}\n\
             print(n)\nprint(len(xs))\nprint(len(ss))\nprint(len(ds))\nprint(x)\n"
        ),
        "3\n3\n1\n1\n4611686018427387904\n",
    );
}

/// The two non-tuple `Subscript` shapes at a release site, pinning that a
/// *list* element is classified owning while a *tuple* element is borrowed.
/// Both are smallints at runtime (D-141 container ingress rejects bigints),
/// so the emitted guard is false either way -- this is a compile-time
/// classification test that must still produce the right values.
#[test]
fn list_and_tuple_elements_are_classified_differently_at_a_release_site() {
    assert_runs_and_prints(
        "subscript_operands",
        "xs = [10, 20]\nt = (30, 40)\nprint(xs[0] + 1)\nprint(t[0] + 1)\nprint(xs[1] + t[1])\n",
        "11\n31\n60\n",
    );
}

/// Issue [#633](https://github.com/rotnov/pycc/issues/633) direction A, the
/// pre-existing use-after-free Part 2 fixes: a tuple field holds a bigint
/// word it never retained, and reading it into an `int` local made that
/// local a second owner of a single reference. Overwriting the local then
/// released the word `b` still held, so the next allocation reused the
/// freed object and `print(b)` produced `c`'s value.
///
/// The more obvious `t = (x + x, 1); y = t[0] + t[0]` shape does *not*
/// reproduce it -- verified while planning -- so the fixture is pinned in
/// exactly this shape.
///
/// The mirror direction (overwriting `b` itself before reading `t[0]`) is
/// D-182's #633 direction B, closed by giving the tuple field its own
/// ingress reference and pinned by
/// `a_bigint_stored_in_a_tuple_survives_overwriting_its_supplier` below.
#[test]
fn a_bigint_read_out_of_a_tuple_is_not_freed_by_overwriting_the_reader() {
    assert_runs_and_prints(
        "tuple_egress_alias",
        &format!(
            "b: int = {PROMOTED}\nt = (b, 1)\ny: int = t[0]\ny = 0\n\
             c: int = 4611686018427387905\nprint(b)\nprint(c)\n"
        ),
        "4611686018427387904\n4611686018427387905\n",
    );
}

/// Issue [#633](https://github.com/rotnov/pycc/issues/633) direction B, the
/// mirror of the fixture above and the one D-182 closes: the tuple field
/// held a bigint word it never retained, so overwriting `b` -- the name
/// that supplied the element -- released the object the field still points
/// at. The next allocation (`c`) reused it and `print(t[0])` produced `c`'s
/// value. Verified failing at the base commit before D-182's ingress retain
/// was added.
#[test]
fn a_bigint_stored_in_a_tuple_survives_overwriting_its_supplier() {
    assert_runs_and_prints(
        "tuple_ingress_supplier_overwritten",
        &format!(
            "b: int = {PROMOTED}\nt = (b, 1)\nb = 0\n\
             c: int = 4611686018427387905\nprint(t[0])\n"
        ),
        "4611686018427387904\n",
    );
}

/// The whole-tuple-copy variant of the same direction. `t2 = t` copies the
/// aggregate field-by-field without going through `MirExpr::TupleLiteral`,
/// so the copy takes no reference of its own; the fixture pins that this is
/// still sound, because the *original* tuple's D-182 ingress reference is
/// never released and therefore keeps the word alive for both.
#[test]
fn a_copied_tuple_still_holds_a_live_bigint_after_its_supplier_is_overwritten() {
    assert_runs_and_prints(
        "tuple_ingress_whole_copy",
        &format!(
            "b: int = {PROMOTED}\nt = (b, 1)\nt2 = t\nb = 0\n\
             c: int = 4611686018427387905\nprint(t2[0])\n"
        ),
        "4611686018427387904\n",
    );
}

/// The loop shape, where the supplier is rebound on every trip. At the base
/// commit this did not merely print a wrong value: it *hung* under
/// `pycc run`, which is what genuine use-after-free looks like when the
/// freed object is walked as a bigint. Correctness is all that is asserted
/// -- deliberately no peak-RSS ratio gate, because D-182 knowingly accepts
/// a trip-count-linear leak for exactly this shape as the price of closing
/// the use-after-free.
#[test]
fn a_bigint_tuple_rebuilt_in_a_loop_survives_its_supplier_being_rebound() {
    assert_runs_and_prints(
        "tuple_ingress_loop_rebind",
        &format!(
            "b: int = {PROMOTED}\nt = (0, 1)\nfor i in range(3):\n    b = b + 1\n\
                 t = (b, 1)\nb = 0\nc: int = 4611686018427387905\nprint(t[0])\n"
        ),
        "4611686018427387907\n",
    );
}

// ---------------------------------------------------------------------------
// `Optional[int]` wrapping (D-197, #763, Part 1 of #747; #770 review round).
//
// `MirExpr::OptionalWrap`'s codegen arm (`emit_expr` in `pycc_codegen::lib`)
// called `coerce_scalar_to_type` directly on a borrowed `int` payload with no
// retain anywhere in the pipeline: `MirStmt::Assign`'s own
// `retain_if_int_duplicate` call runs against the *already-wrapped*
// `Scalar::Optional` result and can never match its `Scalar::Int` guard, and
// `int_value_is_a_duplicate_reference` classifies `OptionalWrap` itself as
// unconditionally owning by design (see its own doc comment), so nothing in
// the pre-fix pipeline ever looked inside the wrapper. Confirmed directly at
// the emitted-IR level (`bigint_rc::tests::
// an_optional_wrap_of_a_borrowed_int_name_retains_the_duplicate_reference`
// and its release-side sibling immediately below it in `bigint_rc.rs`): the
// pre-fix fixture emitted zero `pycc_rt_bigint_retain` calls for `x: int |
// None = n`, while `n`'s own later reassignment still emitted its
// unconditional `pycc_rt_bigint_release` -- an unretained second owner whose
// supplier's release can free the object out from under it. `truthy`'s own
// `Scalar::Optional` arm (`lib.rs`) then unconditionally calls
// `pycc_rt_int_truthy` on the payload word for `if x:`/`while x:`, which
// dereferences a heap-bigint payload via `bigint_ref` -- so the freed object
// is genuinely read back, not merely retained-but-unread.
//
// The value-correctness test below cannot *prove* the absence of a
// use-after-free on every allocator (a freed-then-reused block can still
// look like a plausible bigint), which is exactly why the IR-level tests in
// `bigint_rc.rs` are the authoritative regression coverage for this fix, per
// this module's own established convention (see its file-level doc comment:
// "these are in-crate rather than in `tests/` on purpose"). The peak-RSS
// test after it proves the fix's other, directly observable half: without a
// matching release, the retain this fix adds would leak once per
// reassignment of an `Optional[int]` slot.

#[test]
fn an_optional_int_wrapping_a_borrowed_bigint_survives_its_supplier_being_reassigned() {
    // The exact #770 review repro: `n` supplies `x`'s payload by reference:
    // reassigning `n` afterwards must not affect `x`'s own reference. CPython
    // oracle: `n`'s value (nonzero) is still what `x` reflects, so `if x:`
    // takes the truthy branch regardless of what `n` is reassigned to.
    assert_runs_and_prints(
        "optional_wrap_borrowed_bigint",
        &format!(
            "n = {PROMOTED}\nx: int | None = n\nn = 0\nif x:\n    print(1)\nelse:\n    print(0)\n"
        ),
        "1\n",
    );
}

#[cfg(unix)]
mod peak_rss {
    use super::{PROMOTED, pycc_bin, write_case};
    use std::process::Command;

    /// Spawns `command`, waits for it via `wait4`, and returns the child's
    /// `ru_maxrss`. Follows `tests/nbody_bench.rs::time_command`'s reaping
    /// pattern (duplicated rather than shared -- this repository's
    /// integration tests each carry their own helpers, see that file's
    /// `oracle_python_bin` doc comment), but reads the resident-set high
    /// water mark instead of the CPU times.
    #[allow(clippy::zombie_processes)]
    fn peak_rss(mut command: Command) -> libc::c_long {
        let child = command.spawn().expect("command must spawn");
        let pid = child.id() as libc::pid_t;
        let mut status = 0;
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        let waited = loop {
            // SAFETY: `pid` identifies the live child spawned immediately
            // above; both output pointers are valid for writes for the
            // duration of the call, and a successful `wait4` initializes the
            // complete `rusage` value.
            let result = unsafe { libc::wait4(pid, &mut status, 0, usage.as_mut_ptr()) };
            if result != -1 {
                break result;
            }
            let error = std::io::Error::last_os_error();
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::Interrupted,
                "wait failed for {command:?}: {error}"
            );
        };
        assert_eq!(waited, pid, "wait4 reaped the wrong child for {command:?}");
        // SAFETY: the successful `wait4` above initialized `usage`.
        let usage = unsafe { usage.assume_init() };
        assert!(
            libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
            "command failed: {command:?}"
        );
        // `ru_maxrss` is `i64` on macOS/BSD and `i64` on 64-bit Linux, but
        // the field is the *peak resident set*, in bytes on macOS/BSD and in
        // kilobytes on Linux -- which is why every assertion below is a
        // ratio of two readings from the same platform, never an absolute.
        usage.ru_maxrss
    }

    /// Builds `source` with `pycc build` and returns the produced binary's
    /// own peak RSS -- never `pycc run`'s, whose footprint is dominated by
    /// LLVM and would swamp the signal.
    fn built_program_peak_rss(case: &str, source: &str) -> libc::c_long {
        let (dir, src) = write_case(case, source);
        let bin = dir.join(format!("{case}_rss"));
        let build = Command::new(pycc_bin())
            .arg("build")
            .arg(&src)
            .arg("-o")
            .arg(&bin)
            .status()
            .expect("pycc build must spawn");
        assert!(build.success(), "pycc build of {case} failed");
        peak_rss(Command::new(&bin))
    }

    /// The #146 repro, reduced to Part 1's own scope: each iteration binds a
    /// freshly allocated `BigIntObj` to the same named local, dropping the
    /// previous one on the floor.
    ///
    /// `x = x + 1` rather than `x = <bigint literal> + i`, historically
    /// because a bigint *literal* is materialized per evaluation
    /// (`int_const::emit_int_constant`) and the literal form therefore
    /// leaked one extra unbound temporary per iteration on top of the
    /// named-storage leak this particular test is about. Part 2 (#625,
    /// D-181) closed that temporary leak, and
    /// `a_bigint_literal_operand_does_not_grow_with_the_iteration_count`
    /// below is the form that measures it. This one is left in its original
    /// shape so it keeps measuring Part 1's own site family in isolation.
    fn repro(iterations: u32) -> String {
        format!("x: int = {PROMOTED}\nfor i in range({iterations}):\n    x = x + 1\nprint(x)\n")
    }

    #[test]
    fn overwriting_a_named_bigint_local_does_not_grow_with_the_iteration_count() {
        let single = built_program_peak_rss("rss_1x", &repro(500_000));
        let double = built_program_peak_rss("rss_2x", &repro(1_000_000));
        let ratio = double as f64 / single as f64;
        assert!(
            ratio < 1.35,
            "peak RSS must not scale with the loop trip count: \
             500k iterations={single} 1M iterations={double} ratio={ratio:.4} \
             (a per-iteration `BigIntObj` leak reads as ratio ~2.0)"
        );
    }

    /// The same measurement for the bigint-domain `range` #147 introduced,
    /// where the leaked object is the induction variable itself rather than
    /// an assignment target.
    #[test]
    fn a_bigint_domain_range_loop_does_not_grow_with_the_iteration_count() {
        let start: u128 = 4_611_686_018_427_387_904;
        let single = built_program_peak_rss(
            "rss_range_1x",
            &format!(
                "for i in range({start}, {}):\n    pass\nprint(0)\n",
                start + 500_000
            ),
        );
        let double = built_program_peak_rss(
            "rss_range_2x",
            &format!(
                "for i in range({start}, {}):\n    pass\nprint(0)\n",
                start + 1_000_000
            ),
        );
        let ratio = double as f64 / single as f64;
        assert!(
            ratio < 1.35,
            "peak RSS must not scale with the loop trip count: \
             500k iterations={single} 1M iterations={double} ratio={ratio:.4} \
             (a per-iteration `BigIntObj` leak reads as ratio ~2.0)"
        );
    }

    /// #627/D-187: a bigint parked in an *instance attribute* slot,
    /// overwritten by a `bool` on every iteration. Each iteration puts a
    /// freshly allocated `BigIntObj` in the slot and then replaces it with
    /// the encoded `True` word; if the `bool` store skips the slot release
    /// -- which it did before `pycc_mir` widened the value through
    /// `MirExpr::IntBoundary` -- one `BigIntObj` is stranded per iteration.
    /// The release is invisible in program output, which is why this is a
    /// ratio measurement rather than an assertion on what the program
    /// prints.
    fn bool_over_bigint_attribute_repro(iterations: u32) -> String {
        [
            "class Holder:",
            "    def __init__(self) -> None:",
            "        self.n = 0",
            "",
            "",
            "h = Holder()",
            &format!("for i in range({iterations}):"),
            &format!("    h.n = {PROMOTED} + i"),
            "    h.n = True",
            "print(h.n)",
            "",
        ]
        .join("\n")
    }

    #[test]
    fn a_bool_overwriting_a_bigint_attribute_does_not_grow_with_the_iteration_count() {
        let single = built_program_peak_rss(
            "rss_attr_bool_1x",
            &bool_over_bigint_attribute_repro(500_000),
        );
        let double = built_program_peak_rss(
            "rss_attr_bool_2x",
            &bool_over_bigint_attribute_repro(1_000_000),
        );
        let ratio = double as f64 / single as f64;
        assert!(
            ratio < 1.35,
            "peak RSS must not scale with the loop trip count: \
             500k iterations={single} 1M iterations={double} ratio={ratio:.4} \
             (one `BigIntObj` stranded per bool-over-bigint attribute store \
             reads as ratio ~2.0)"
        );
    }

    /// #146 Part 2 (D-181): the nested arithmetic temporary. `(x + 1)` is
    /// an unbound `int` value whose single consumer is the outer `+`, and
    /// before Part 2 nothing ever retired its birth reference. Measured at
    /// the plan's own baseline this form grew with ratio 1.925 against the
    /// `< 1.35` gate below.
    fn nested_temporary_repro(iterations: u32) -> String {
        format!(
            "x: int = {PROMOTED}\ny: int = 0\nfor i in range({iterations}):\n    \
             y = (x + 1) + 2\nprint(y)\n"
        )
    }

    #[test]
    fn a_nested_bigint_arithmetic_temporary_does_not_grow_with_the_iteration_count() {
        let single = built_program_peak_rss("rss_tmp_1x", &nested_temporary_repro(500_000));
        let double = built_program_peak_rss("rss_tmp_2x", &nested_temporary_repro(1_000_000));
        let ratio = double as f64 / single as f64;
        assert!(
            ratio < 1.35,
            "peak RSS must not scale with the loop trip count: \
             500k iterations={single} 1M iterations={double} ratio={ratio:.4} \
             (a per-iteration `BigIntObj` leak reads as ratio ~2.0)"
        );
    }

    /// The same shape with a *named* operand on both sides rather than a
    /// folded literal, so the leaked temporary is produced by two
    /// borrowed reads rather than one borrowed read and one immediate.
    /// Measured at 1.926 before Part 2.
    fn aliased_temporary_repro(iterations: u32) -> String {
        format!(
            "x: int = {PROMOTED}\ny: int = 0\nfor i in range({iterations}):\n    \
             y = (x + x) + x\nprint(y)\n"
        )
    }

    #[test]
    fn an_aliased_bigint_arithmetic_temporary_does_not_grow_with_the_iteration_count() {
        let single = built_program_peak_rss("rss_alias_1x", &aliased_temporary_repro(500_000));
        let double = built_program_peak_rss("rss_alias_2x", &aliased_temporary_repro(1_000_000));
        let ratio = double as f64 / single as f64;
        assert!(
            ratio < 1.35,
            "peak RSS must not scale with the loop trip count: \
             500k iterations={single} 1M iterations={double} ratio={ratio:.4} \
             (a per-iteration `BigIntObj` leak reads as ratio ~2.0)"
        );
    }

    /// The third acceptance form, and the only one that exercises the
    /// bigint *literal* allocation specifically: `{PROMOTED}` is outside
    /// D-061's tagged smallint range, so `int_const::emit_int_constant`
    /// cannot fold it to an immediate and emits a per-evaluation
    /// `pycc_rt_int_from_i64` call instead. Neither form above reaches that
    /// path -- their `1`/`2` literals are folded immediates.
    fn literal_operand_repro(iterations: u32) -> String {
        format!("y: int = 0\nfor i in range({iterations}):\n    y = {PROMOTED} + i\nprint(y)\n")
    }

    #[test]
    fn a_bigint_literal_operand_does_not_grow_with_the_iteration_count() {
        let single = built_program_peak_rss("rss_lit_1x", &literal_operand_repro(500_000));
        let double = built_program_peak_rss("rss_lit_2x", &literal_operand_repro(1_000_000));
        let ratio = double as f64 / single as f64;
        assert!(
            ratio < 1.35,
            "peak RSS must not scale with the loop trip count: \
             500k iterations={single} 1M iterations={double} ratio={ratio:.4} \
             (a per-iteration `BigIntObj` leak reads as ratio ~2.0)"
        );
    }

    /// #770 review round (D-197 follow-up): the release-side half of the
    /// `Optional[int]`-wrapping fix, proved the same way Part 1's own
    /// `repro` above proves its release. Before this fix, `emit_assign`
    /// released a previous slot payload only for a `Ty::Int` slot -- never
    /// for `Ty::Optional(Int)` -- so a bigint retained into `x` (by the
    /// fix directly above this module) on one trip through the loop was
    /// never released on the next trip's overwrite, leaking one `BigIntObj`
    /// per iteration exactly like Part 1's original named-storage leak.
    fn optional_wrap_reassignment_repro(iterations: u32) -> String {
        format!(
            "y: int = {PROMOTED}\nx: int | None = None\nfor i in range({iterations}):\n    \
             y = y + 1\n    x = y\nif x:\n    print(1)\nelse:\n    print(0)\n"
        )
    }

    #[test]
    fn reassigning_an_optional_int_local_in_a_loop_does_not_grow_with_the_iteration_count() {
        let single = built_program_peak_rss(
            "rss_optional_wrap_1x",
            &optional_wrap_reassignment_repro(500_000),
        );
        let double = built_program_peak_rss(
            "rss_optional_wrap_2x",
            &optional_wrap_reassignment_repro(1_000_000),
        );
        let ratio = double as f64 / single as f64;
        assert!(
            ratio < 1.35,
            "peak RSS must not scale with the loop trip count: \
             500k iterations={single} 1M iterations={double} ratio={ratio:.4} \
             (a per-iteration `BigIntObj` leak reads as ratio ~2.0)"
        );
    }
}
