//! Extraction of a user function call's result value into a [`Scalar`].
//!
//! Narrow carve out of `lib.rs` under AGENTS.md's decomposability rule: this
//! is exactly the unit #925 touches, and nothing else is relocated.

use super::*;
use inkwell::values::CallSiteValue;
use pycc_mir::Ty;

/// Maps a completed call site to the [`Scalar`] its declared return type
/// carries.
///
/// The match is exhaustive with no catch-all arm, so every `Ty` variant is
/// accounted for here explicitly and a variant added later fails to compile
/// rather than reaching a runtime panic.
pub(super) fn call_result_scalar<'ctx>(
    context: &'ctx Context,
    call_site: CallSiteValue<'ctx>,
    ty: &Ty,
) -> Scalar<'ctx> {
    match ty {
        Ty::Int => Scalar::Int(
            call_site
                .try_as_basic_value()
                .expect_basic("this function is declared to return int")
                .into_int_value(),
        ),
        Ty::Bool => Scalar::Bool(
            call_site
                .try_as_basic_value()
                .expect_basic("this function is declared to return bool")
                .into_int_value(),
        ),
        Ty::Float => Scalar::Float(
            call_site
                .try_as_basic_value()
                .expect_basic("this function is declared to return float")
                .into_float_value(),
        ),
        Ty::Str => Scalar::Str(
            call_site
                .try_as_basic_value()
                .expect_basic("this function is declared to return str")
                .into_pointer_value(),
        ),
        Ty::None => {
            // A `None`-returning call's LLVM function returns
            // `void`, so there is no value to extract. Preserve the
            // call's side effects and materialize the canonical zero
            // carrier used when that unit value crosses a parameter
            // or storage boundary. The surrounding MIR type keeps it
            // distinct from a real `False` value.
            Scalar::Bool(context.i8_type().const_int(0, false))
        }
        // A class-typed return annotation is reachable from real
        // source: `pycc_hir::func::annotation_to_ty` resolves
        // `Self` (PEP 673), a self-referential class name and any
        // known class name alike to `Ty::Instance` (#380/#387), and
        // `-> Self` is conformance-tested against CPython 3.14
        // through `tests/fixtures/pep_0673_self.py`. Extracting the
        // result needs nothing beyond `Str`'s own shape:
        // `ty_to_basic_type` gives an `Instance`-returning
        // function's LLVM signature the same pointer return type a
        // `str`-returning one gets.
        Ty::Instance(_) => Scalar::Instance(
            call_site
                .try_as_basic_value()
                .expect_basic("this function is declared to return an instance")
                .into_pointer_value(),
        ),
        // `Optional[int]` (D-197, #763, Part 1 of #747): unlike
        // `List`/`Dict`/`Set`/`Tuple` below, an `Optional`-returning
        // function IS reachable from real, type-checked source --
        // `pycc_hir::func::annotation_to_ty`'s own `T | None` arm
        // accepts `-> int | None` as a return annotation, so a call
        // to such a function (`y = g(x)`, or `g(x)` used directly
        // as a narrowing condition's operand) must extract its
        // struct-by-value result rather than falling through to the
        // generic panic below. `ty_to_basic_type`'s own
        // `Ty::Optional` arm already gave this function's LLVM
        // signature the matching `{ inner, i8 }` struct return
        // type, mirroring `Tuple`'s own by-value extraction one
        // arm up in kind (D-115).
        Ty::Optional(_) => Scalar::Optional(
            call_site
                .try_as_basic_value()
                .expect_basic("this function is declared to return Optional[int]")
                .into_struct_value(),
        ),
        // The container families (#925, Part 2 of #918; `frozenset[int]`
        // shares `set[int]`'s arm since Part 1 of #1319). All of
        // them were reachable only through the panic below until this
        // change: D-228 (issue #918, Part 1) lowered a parameterized
        // container annotation in every position *except* return, and
        // #925 removed that last exclusion, so a container-returning
        // function's call result now has to be extracted here.
        //
        // `ty_to_basic_type` already gives each of these the right LLVM
        // return type, so nothing about the callee's signature changes:
        // `List`/`Dict`/`Set` are heap-allocated runtime objects always
        // referenced by pointer (D-105/D-121/D-122), extracted exactly
        // like `Str`/`Instance` above; `Tuple` is D-115's by-value SSA
        // struct, extracted like `Optional` above.
        //
        // No ownership work accompanies these arms. Codegen declares no
        // container refcount entry point and calls none (D-107 for
        // `list`, D-124 for `dict`/`set`), so a container's backing
        // allocation leaks for the process's lifetime and returning its
        // pointer can neither dangle nor double-free. Returning a
        // container is a genuine pointer transfer -- the callee's value
        // and the caller's binding name the same allocation -- which
        // adds no new free site and no new release obligation.
        Ty::List(_) => Scalar::List(
            call_site
                .try_as_basic_value()
                .expect_basic("this function is declared to return list")
                .into_pointer_value(),
        ),
        Ty::Dict(_) => Scalar::Dict(
            call_site
                .try_as_basic_value()
                .expect_basic("this function is declared to return dict")
                .into_pointer_value(),
        ),
        Ty::Set(_) | Ty::FrozenSet(_) => Scalar::Set(
            call_site
                .try_as_basic_value()
                .expect_basic("this function is declared to return set")
                .into_pointer_value(),
        ),
        Ty::Tuple(_) => Scalar::Tuple(
            call_site
                .try_as_basic_value()
                .expect_basic("this function is declared to return tuple")
                .into_struct_value(),
        ),
        // Exhaustive by construction: no catch-all. Every `Ty` variant
        // above yields a real `Scalar`, and these three are the only
        // ones left. Each is unreached today, but for two different
        // reasons, and the distinction matters.
        //
        // `Ty::Infer` and `Ty::Param` cannot be produced by real,
        // type-checked source at all -- `Ty::Infer` is an HIR-only
        // placeholder that never survives to MIR (an unresolvable helper
        // is `T0021` at type-check time) and `Ty::Param` is substituted
        // at each call site before codegen (D-134). For those two this
        // arm really is a defensive backstop for malformed, hand-built
        // MIR.
        //
        // `Ty::Protocol` joined them with #934: a function annotated to
        // return a `Protocol` subclass used to be accepted by the front
        // end and then abort inside `pycc_mir` (or here, for an unused
        // result), so this arm was a genuine reachability gap rather than
        // a backstop. `pycc_hir::func::lower_return_annotation` now
        // rejects a protocol class in return position with `C0001`, so no
        // type-checked source can produce a protocol-typed call result any
        // more; like the other two, this arm now guards hand-built MIR
        // only. If a later slice lifts that gate (D-166 gives a protocol
        // no runtime representation, so it would have to bind a concrete
        // type first), this arm needs a real implementation, not a panic.
        //
        // All three are covered by one `#[should_panic]` unit test per
        // variant in `pycc_codegen` rather than by any `.py` input.
        //
        // Listing the variants instead of writing `other =>` is the
        // point: a `Ty` variant added later is a compile error here
        // rather than a runtime panic discovered by a user, which is
        // what makes "only these three remain unhandled" a mechanically
        // checked claim instead of a comment.
        //
        // Part 2 of #1026 moved `Ty::Object` out of that panic group and
        // gave it the real extraction Part 1's comment said it would need.
        // It is no longer unreachable: an *unannotated* private helper
        // whose body returns a foreign attribute infers a `Ty::Object`
        // return, and since Part 1 of #1367 a return annotated with a class
        // a foreign import binds declares one, so a call to either is a
        // `Ty::Object`-typed call result. The value is an opaque `PyObject *` and stays one --
        // `Scalar::Object` is the pointer and nothing here inspects it.
        Ty::Object => Scalar::Object(
            call_site
                .try_as_basic_value()
                .expect_basic("this function is declared to return a CPython object")
                .into_pointer_value(),
        ),
        ty @ (Ty::Infer | Ty::Param(_) | Ty::Protocol(_) | Ty::MemoryView) => {
            panic!(
                "pycc_codegen: a `{}`-typed call result is not supported yet",
                ty.name()
            )
        }
    }
}
