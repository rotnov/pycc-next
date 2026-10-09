//! The artifact-owned buffer producer and the refusals that bound it
//! (Part 2a of #1142, issue #1165).
//!
//! Until this module existed, `Ty::MemoryView` had exactly one source: a
//! parameter of a `pycc build --ext` export, borrowed from the host for the
//! duration of one call. `crate::expr::reject_memoryview_read`'s own doc
//! comment states the consequence that fact bought -- "`memoryview` has no
//! literal and no producing expression, so ... [every other position] is
//! reachable only through that read and is refused by it at once".
//!
//! This module adds the second source: `ndarray(n)` / `NDArray(n)` in
//! expression position, yielding a zero-filled one-dimensional `float64`
//! buffer of `n` elements that the artifact itself allocates and frees. With
//! two sources, "which refusal applies to this name" stops being a question
//! about the *type* and becomes a question about the *binding*, which is why
//! [`crate::Environment`] and `crate::constraints::ConstraintEnvironment`
//! both carry an `owned_buffers` set and
//! [`crate::expr::reject_memoryview_read`] takes a provenance flag.
//!
//! # The producer's admitted position
//!
//! Exactly one: the entire right-hand side of an assignment -- bare or
//! annotated -- to a simple local name, inside a function body. Every other
//! syntactic position is refused by [`producer_position_unsupported`], and
//! module scope by [`producer_at_module_scope`].
//!
//! That single-position rule is what makes Part 2a's free-at-function-exit
//! lifetime sound: the value cannot be aliased, stored, returned, passed or
//! left on the floor, so the allocating frame is provably the only owner and
//! `crates/pycc_codegen/src/call_result.rs`'s `Ty::MemoryView` panic stays
//! unreachable by construction rather than by luck.
//!
//! # Where the refusal lives, and why it is one message
//!
//! Both type walkers refuse the producer spelling from their own `Call` arm
//! and both admit it only from their own assignment arm. Refusing in the
//! `Call` arm is what reaches the positions a `Name`-keyed refusal
//! structurally cannot: `crate::check_stmt_in_function`'s `HirStmt::ExprStmt`
//! arm discards the inferred type, so a bare `ndarray(4)` statement would
//! otherwise type-check, lower to a `MirStmt::ExprStmt` that
//! `collect_stmt_bindings` never gives a slot, and leak one allocation per
//! call in a long-lived host process.
//!
//! The plan for #1165 asked for a distinct message per refused position
//! (call argument, container literal, return value, comprehension, subscript
//! base, bare statement). One named producer-position diagnostic raised from
//! the `Call` arm is used instead: the plan's actual objection is that those
//! positions are refused *incidentally* today (`f(ndarray(3))` falls out as a
//! parameter-type mismatch), and a named refusal raised before the value can
//! reach any of them removes the incidentality for all of them at once,
//! without six messages that can drift apart. Each position keeps its own
//! pin in the test matrix.
//!
//! # Spelling, and whose binding wins
//!
//! `ndarray` and `NDArray` -- the two spellings #1129 and #1134 already gave
//! the *annotation*, now callable. `memoryview` is deliberately **not** a
//! producer spelling: `memoryview(5)` is a `TypeError` in CPython while
//! `numpy.ndarray(5)` is a real constructor, so admitting it would mean
//! something CPython refuses outright rather than something it does
//! differently. D-244's #1134 "one type, three spellings" is therefore
//! qualified rather than broken -- it still holds for every rule stated
//! about the *type*, and the call position is the first rule stated about a
//! *spelling*.
//!
//! D-244's #1129 statement (h) governs the call position identically to the
//! annotation position: a program's own `class ndarray` or `def NDArray`
//! wins. In the check phase that falls out of placement -- the interception
//! sits inside `infer_expr_in`'s `lookup_function` `else` block, after the
//! class and generic lookups have already run. The solver has no class
//! table, so `ConstraintEnvironment::shadowed_producers` carries the same
//! fact explicitly; see its own doc comment.

use pycc_diag::{Diagnostic, Span};
use pycc_hir::Ty;

/// The two callable spellings of the buffer producer.
///
/// Deliberately *not* registered in `crate::is_known_callable_builtin` or in
/// `pycc_hir::typecheck::is_builtin_type_name`: either registration would
/// make `isinstance(x, ndarray)` an accept, against D-244's #1129
/// statement (e).
pub(crate) const PRODUCER_SPELLINGS: [&str; 2] = ["ndarray", "NDArray"];

/// Whether `callee` is spelled like the buffer producer.
///
/// A spelling match alone decides nothing: the caller must still apply
/// statement (h) (the program's own binding wins) and the arity rule before
/// treating the call as a producer.
pub(crate) fn is_producer_spelling(callee: &str) -> bool {
    PRODUCER_SPELLINGS.contains(&callee)
}

/// `Err(C0001)` for a buffer producer anywhere but an assignment's
/// right-hand side.
///
/// See this module's own doc comment for why one message covers every
/// refused position.
pub(crate) fn producer_position_unsupported(callee: &str) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!(
            "`{callee}(n)` here is valid Python but not implemented yet; a buffer \
             producer is admitted only as the whole right-hand side of an assignment \
             to a local name inside a `pycc build --ext` export, as in `a = {callee}(n)`"
        ),
        Span::new(0, 0),
    )
}

/// `Err(C0001)` for a buffer producer at module scope.
///
/// Its own refusal rather than a case of [`producer_position_unsupported`]
/// because the ground is different, and Part 2b will lift exactly one of the
/// two. A module-level frame gets no epilogue at all
/// (`crates/pycc_codegen/src/lib.rs`'s owned-slot frame is skipped for
/// `main` / `__pycc_ext_exec`), and the "cannot escape its function"
/// invariant that makes free-at-exit sound says nothing about a value that
/// was never in a function.
pub(crate) fn producer_at_module_scope(callee: &str) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!(
            "`{callee}(n)` at module scope is valid Python but not implemented yet; \
             artifact-owned buffer storage is freed when the allocating function \
             returns, and module-level code has no such frame -- allocate it inside a \
             `pycc build --ext` export instead"
        ),
        Span::new(0, 0),
    )
}

/// `Err(T0033)` when a buffer producer's length argument is not an `int`.
///
/// A `bool` is *not* such a case: it is an `int` by the representation table
/// (`docs/TYPE_SYSTEM.md`, rule 4/D-086), so `ndarray(True)` requests one
/// element. See [`producer_assignment_ty`]'s own length check, and
/// `crate::constraints::reject_non_int_producer_length` for the solver's
/// mirror of it -- whose absence made this refusal unreachable for an
/// unannotated private helper until #1165's review round 8.
pub(crate) fn producer_length_not_an_int(callee: &str, len_ty: &Ty) -> Diagnostic {
    Diagnostic::error(
        "T0033",
        format!(
            "`{callee}` expects an `int` element count, got `{}`",
            len_ty.name()
        ),
        Span::new(0, 0),
    )
    .with_help("pass an `int` length, as in `a = ndarray(4)`")
}

/// `Err(C0001)` for any use of an **artifact-owned** buffer beyond the three
/// operations Part 2a implements.
///
/// The owned counterpart of [`crate::expr::reject_memoryview_read`]'s
/// parameter arm, whose message is pinned verbatim by eight assertions and
/// stays exactly as it was. The distinction is load-bearing rather than
/// cosmetic: the two arms name different storage and different reasons, and
/// reporting the parameter message for the owned case would have left Part
/// 2b inheriting a refusal that lies about why it exists.
///
/// Both refusals have since been lifted at the *return* position only --
/// `return a` by Part 2b of #1142 (#1164), `return b` by Part 1 of #1175
/// (#1178) -- and the parameter arm's original ground, that handing back a
/// view the host lent for one call is a use-after-free, was never true of
/// the shape. [`admits_buffer_egress`] carries the corrected account and the
/// measurement behind it. Every *other* use of either name is still refused
/// here and by [`crate::expr::reject_memoryview_read`], which is what keeps
/// a buffer name from being aliased.
pub(crate) fn owned_buffer_use_unsupported(name: &str) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!(
            "using `{name}`, which is bound to buffer storage this `pycc build --ext` \
             artifact allocated, is valid Python but not implemented yet; #1165 admits \
             such a buffer only inside the function that allocated it, read one element \
             at a time with `{name}[i]` over `range(len({name}))`, and store one with \
             `{name}[i] = 1.0`"
        ),
        Span::new(0, 0),
    )
}

/// `Err(C0001)` for assigning to a name bound to a buffer **parameter**.
///
/// Refused in Part 2a for two independent reasons, either of which is
/// sufficient. The memory-safety one: `crates/pycc_codegen/src/lib.rs`
/// pushes parameter slots into the frame's owned-slot list on the `str`
/// precedent, so a rebound buffer parameter would make the epilogue `free`
/// a `PyccExtBufferView *` the host owns. The behavioral one:
/// `pycc_hir::body_stores_into` is a syntactic walk matching
/// `HirStmt::DictSet` targets against *parameter names*, so a store into the
/// rebound local would make `src/ext_build.rs` request `PyBUF_WRITABLE` on
/// the host's buffer for a store the body never performs on it.
///
/// Refusing this shape is also what makes the flat, flow-insensitive
/// provenance model sound: no name can be parameter-bound for the first half
/// of a function and artifact-owned for the second.
pub(crate) fn buffer_parameter_rebinding(name: &str) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!(
            "assigning to `{name}`, a buffer parameter of a `pycc build --ext` export, \
             is valid Python but not implemented yet; the wrapper borrows that buffer \
             from the host for exactly one call -- bind the new buffer to a different \
             local name instead"
        ),
        Span::new(0, 0),
    )
}

/// The check phase's one admitting seam for the buffer producer.
///
/// `None` means "`value` is not a buffer producer here" and the caller falls
/// through to ordinary inference; `Some(Ok(Ty::MemoryView))` means the
/// caller must bind `target` as artifact-owned storage; `Some(Err(..))` is a
/// refusal that belongs to the producer rather than to the assignment.
///
/// Called from exactly the four assignment arms that admit it -- module and
/// function scope, plain and annotated -- so "the producer's admitted
/// position" is one call site per arm rather than a property scattered
/// across the expression walk. Everything this function declines is refused
/// by `infer_expr_in`'s own `Call` arm.
///
/// Statement (h) is applied here explicitly, because unlike the `Call` arm
/// this seam runs *before* the class, generic and function lookups: a
/// program's own `class ndarray`, `def ndarray`, or module-level `ndarray =
/// ...` all make this return `None`, leaving the ordinary walk to resolve
/// the call as the program meant it.
pub(crate) fn producer_assignment_ty(
    env: &crate::Environment,
    local_names: &[&str],
    value: &pycc_hir::HirExpr,
) -> Option<Result<Ty, Diagnostic>> {
    let pycc_hir::HirExpr::Call { callee, args } = value else {
        return None;
    };
    if !is_producer_spelling(callee) {
        return None;
    }
    // D-244 #1129 statement (h): the program's own binding wins, in call
    // position exactly as in annotation position.
    //
    // `local_names` is the fourth arm rather than a redundant one. It is a
    // whole-body pre-pass (`crate::function_local_names` = the parameters
    // plus every assignment, `for`, comprehension, `match` capture and
    // `except ... as` target `collect_local_names` reaches at any depth), so
    // it answers CPython's own scoping question: a function that binds the
    // spelling *anywhere* in its body makes it local *throughout*, and a use
    // before that binding is an `UnboundLocalError` rather than a producer.
    // Without this arm `a = ndarray(4)` followed by a later `ndarray = 1` in
    // the same body allocated, because `env.bindings` never holds a local
    // that has not been bound yet at this point in the walk. The solver's
    // mirror is `crate::constraints::resolved_producer_call`; both then let
    // the ordinary `Call` walk report the program's own error, which for a
    // syntactic local is the solver's `unbound_local` (`T0021`).
    //
    // `std_module_aliases` is the fifth arm for the same reason, and it is
    // the one arm that was a *miscompile* rather than a misdirected
    // message: `import math as ndarray` binds the spelling to the `math`
    // module, but a stdlib module alias is recorded only in that table --
    // never in `env.bindings` -- so without this arm `a = ndarray(4)`
    // allocated a buffer for a program whose own binding makes the call
    // CPython's `TypeError: 'module' object is not callable`. Declining
    // here hands it to the ordinary `Call` walk, which reports `T0021`
    // exactly as it does for an unaliased spelling. `docs/TYPE_SYSTEM.md`'s
    // `memoryview` row is the canonical enumeration of the binding kinds
    // statement (h) covers.
    if env.lookup_class(callee).is_some()
        || env.lookup_generic(callee).is_some()
        || env.lookup_function(callee).is_some()
        || env.bindings.contains_key(callee.as_str())
        || crate::is_local(local_names, callee)
        || env
            .std_module_aliases
            .iter()
            .any(|(alias, _)| alias == callee)
    {
        return None;
    }
    // A wrong arity is not a producer at all: it falls through to the `Call`
    // arm, whose refusal names the admitted shape (`a = ndarray(n)`).
    if args.len() != 1 {
        return None;
    }
    if !env.in_function_body {
        return Some(Err(producer_at_module_scope(callee)));
    }
    let len_ty = match crate::expr::infer_expr_in(env, local_names, &args[0]) {
        Ok(ty) => ty,
        Err(diagnostic) => return Some(Err(diagnostic)),
    };
    // `bool` is admitted alongside `int`, exactly as it is for a buffer
    // *index* (`docs/TYPE_SYSTEM.md`'s `memoryview` row, rule 4/D-086):
    // `crate::is_assignable`'s own `from == Ty::Bool && to == Ty::Int` clause
    // encodes that representation-table subtyping, and codegen already
    // decodes the value -- `MirExpr::BufferAlloc`'s arm routes the length
    // through `to_numeric_encoded_int`, whose `Scalar::Bool` arm zero-extends
    // the `i8` to `i64` and re-tags it before the shared checked untag, so
    // `ndarray(True)` reaches the allocator as the element count `1`.
    //
    // Written as an explicit match rather than `is_assignable(len_ty,
    // Ty::Int)` because that helper also admits a `Ty::Param`, which is not a
    // length; the `float(...)` arm in `crate::expr` spells its own
    // `Int | Float | Bool` admission out for the same reason.
    if !matches!(len_ty, Ty::Int | Ty::Bool) {
        return Some(Err(producer_length_not_an_int(callee, &len_ty)));
    }
    Some(Ok(Ty::MemoryView))
}

/// The public spelling predicate, for the one consumer outside this crate.
///
/// `src/memoryview_mode.rs`'s native-mode body gate has to recognize the
/// producer *before* the type check runs (it is a mode refusal, and neither
/// `pycc_hir` nor this crate carries artifact-mode awareness), so it cannot
/// ask the typed result and must match the spelling itself. Exporting the
/// predicate keeps `PRODUCER_SPELLINGS` the one place the two spellings
/// are written down for the checker side of the compiler; `pycc_mir`'s own
/// lowering guard names them literally and cross-references this constant,
/// because `pycc_mir` does not depend on this crate.
pub fn is_buffer_producer_spelling(callee: &str) -> bool {
    is_producer_spelling(callee)
}

/// The producer spellings `params`/`body` bind as function-local names, for
/// the same one consumer outside this crate as
/// [`is_buffer_producer_spelling`].
///
/// `src/memoryview_mode.rs`'s native-mode body gate has to reproduce
/// [`producer_assignment_ty`]'s statement-(h) decline *exactly*, and that
/// decline's fourth arm is `crate::is_local` over
/// `crate::function_local_names`. Exporting the composition of those two
/// private helpers -- rather than letting the gate re-derive locality with a
/// walk of its own -- is what makes the gate's skip sound by construction:
/// the gate skips a spelling only where this returns it, this returns
/// exactly what the checker's fourth arm tests, and where that arm declines
/// the checker never binds `Ty::MemoryView`, so no allocation the gate
/// stopped refusing can reach a native artifact. A hand-rolled locality test
/// could diverge, and in the one direction that matters.
///
/// Filtered to the producer spellings so the export stays as narrow as
/// [`is_buffer_producer_spelling`]: `function_local_names` itself and the
/// whole local-name vocabulary stay crate-private.
pub fn function_local_producer_spellings<'a>(
    params: &'a [(String, Ty)],
    body: &'a [pycc_hir::HirStmt],
) -> Vec<&'a str> {
    crate::function_local_names(params, body)
        .into_iter()
        .filter(|name| is_producer_spelling(name))
        .collect()
}

/// The producer spellings a module's own import table binds, for the same
/// one consumer outside this crate as [`function_local_producer_spellings`].
///
/// `import math as ndarray` binds the spelling to the `math` module, so
/// D-244 #1129 statement (h) makes it the program's own meaning and
/// [`producer_assignment_ty`] declines the call. The check phase reads that
/// fact from `crate::Environment`'s `std_module_aliases`, which
/// `crate::std_receiver::bind_std_module_aliases` fills;
/// `src/memoryview_mode.rs`'s native-mode body gate walks the HIR instead
/// and has no environment to ask, so this export gives it the same
/// computation rather than a second walk that agrees today.
///
/// The module-alias arm is deliberately *exactly* `bind_std_module_aliases`'
/// predicate -- an `ImportBinding::Module` whose `local_name` is not the
/// module's canonical spelling. Membership in that gate's shadow set means
/// "skip the refusal", so a set wider than the checker's decline set is
/// under-refusal: an artifact-owned allocation reaching a native executable.
///
/// `ImportBinding::Foreign` (`import ndarray`, `from x import ndarray`) is
/// the second arm (Part 1 of #1333). It used to need no entry, because a
/// foreign name bound in a function body was refused upstream with
/// `I0404` before the native gate ran. Part 1 of #1333 admits that binding,
/// so `a = ndarray(n)` over a foreign import now type-checks as an
/// ordinary CPython call, and without this arm the gate reported `I0405`
/// ("rebuild with `--ext`") for a program with no buffer allocation at all.
/// The arm stays inside the checker's decline set: the check phase binds
/// every foreign import in `Environment::bindings` (via
/// `crate::foreign::bind_foreign_objects_at`), which is
/// `producer_assignment_ty`'s third arm, and the solver's
/// `constraints::resolved_producer_call` declines on its `foreign_objects`
/// table, which is seeded from the same
/// `crate::foreign::foreign_object_names` this arm reads. A buffer-carrier
/// from-import (`from numpy import ndarray`, `from numpy.typing import
/// NDArray`, #1380) is *not* in this set: it binds a hidden local name that
/// is never a producer spelling, and `pycc_hir`'s `import::carrier` refuses
/// every producer call (and every other non-annotation use of the
/// spelling) in such a module before this crate runs.
/// `ImportBinding::Symbol` (`from math import sqrt as
/// ndarray`) stays out: it is refused upstream with `C0001`.
pub fn imported_producer_spellings(imports: &[pycc_hir::ImportBinding]) -> Vec<&str> {
    imports
        .iter()
        .filter_map(|binding| match binding {
            pycc_hir::ImportBinding::Module { local_name, module }
                if local_name != pycc_std::module_name(*module) =>
            {
                Some(local_name.as_str())
            }
            _ => None,
        })
        .chain(crate::foreign::foreign_object_names(imports))
        .filter(|name| is_producer_spelling(name))
        .collect()
}

/// The name an admitted buffer **egress** returns, or `None` when this
/// `return` is not the one shape Part 2b of #1142 (#1164) admits.
///
/// The shape is deliberately exact: a bare name, in a function whose
/// *declared* return type is the buffer type. Both walkers call this from
/// their own `HirStmt::Return` arm, before the operand is ever inferred, on
/// the same **interception** model `crate::expr::reject_memoryview_read`'s
/// doc comment records for `b[i]` and `len(b)` -- the refusal itself is not
/// weakened, only the set of expressions that reach it narrows by one.
///
/// Membership in the caller's own `owned_buffers` set is the caller's half
/// of the test and is deliberately *not* asked here: that set lives on two
/// different environments (the check phase's [`crate::Environment`] and the
/// solver's `ConstraintEnvironment`), so the provenance question is left
/// with each walker, which answers it in its own spelling and hands the
/// answer to [`admits_buffer_egress`].
///
/// Part 1 of #1175 corrected what that split is *for*. It used to read "and
/// that is what keeps a *parameter*-bound name -- `return b` -- falling
/// through to the parameter refusal, which is the use-after-free #1142
/// exists to forbid". Both halves of that sentence are now false. A
/// parameter-bound name at this position is **admitted**, and the
/// use-after-free it named was never a property of the shape: the wrapper's
/// `PyBuffer_Release` releases the wrapper's *own* `Py_buffer`, while the
/// returned view is built with `PyMemoryView_FromObject` on the host's own
/// argument object and therefore holds a **second, independent** buffer
/// export that the wrapper's release does not touch. Holding that export --
/// not holding a reference -- is what pins the storage; a bare `Py_INCREF`
/// really would ship a use-after-free against a resizable exporter such as
/// `array.array('d')`. See the D-244 amendment for the measurement.
///
/// A declared return type other than the buffer type declines here, so
/// `def f(n: int) -> float: a = ndarray(n); return a` keeps exactly the
/// refusal it has today.
///
/// Part 2 of #1175 (#1179) adds the second admitted shape:
/// `return b[start:stop]`, an `HirExpr::Slice` whose base is a bare name.
/// The answer therefore names the shape as well as the name -- see
/// [`AdmittedBufferReturn`]. The `step`, when one is written, is reported
/// but **not** refused here: this function is environment-free, and the
/// same syntax with a non-buffer base is an ordinary return-type mismatch
/// that the step diagnostic would misdescribe. Both refusals a slice owes
/// -- [`buffer_slice_step_unsupported`] and
/// [`buffer_slice_bound_not_an_int`] -- are raised from the walkers'
/// post-admission block, on the precedent of
/// [`buffer_return_inside_finally`].
pub(crate) fn admitted_buffer_return<'a>(
    expr: &'a pycc_hir::HirExpr,
    declared_return: Option<&Ty>,
) -> Option<(&'a str, AdmittedBufferReturn<'a>)> {
    if !matches!(declared_return, Some(Ty::MemoryView)) {
        return None;
    }
    match expr {
        pycc_hir::HirExpr::Name(name) => Some((name.as_str(), AdmittedBufferReturn::Whole)),
        pycc_hir::HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => match base.as_ref() {
            pycc_hir::HirExpr::Name(name) => Some((
                name.as_str(),
                AdmittedBufferReturn::Slice {
                    start: start.as_deref(),
                    stop: stop.as_deref(),
                    has_step: step.is_some(),
                },
            )),
            _ => None,
        },
        _ => None,
    }
}

/// The syntactic shape [`admitted_buffer_return`] matched, carried to
/// [`admits_buffer_egress`] and to the walkers' shared post-admission block.
///
/// Part 2 of #1175 (#1179) adds the second variant. The two are not
/// interchangeable at the admission gate: a sub-range is admitted for a
/// **caller-owned** parameter only, while the whole view is admitted for
/// both provenances (see [`admits_buffer_egress`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum AdmittedBufferReturn<'a> {
    /// `return b` -- the whole view, either provenance.
    Whole,
    /// `return b[start:stop]` -- a sub-range of a caller-owned view.
    ///
    /// `start` and `stop` are the unlowered bound expressions, which the
    /// admitted branch still owes a type check (D1): the interception is an
    /// early exit, so the ordinary `crate::expr`'s `HirExpr::Slice` arm --
    /// the one that raises `T0021` for a non-`int` bound -- never runs on
    /// them. `has_step` records that a `step` was written at all; the
    /// refusal it drives is [`buffer_slice_step_unsupported`].
    Slice {
        start: Option<&'a pycc_hir::HirExpr>,
        stop: Option<&'a pycc_hir::HirExpr>,
        has_step: bool,
    },
}

/// Whether a name [`admitted_buffer_return`] picked out is an admitted
/// buffer **egress**, given the provenance facts only the calling walker can
/// answer.
///
/// Two provenances are admitted, and the walkers treat them identically --
/// which is why this answers a `bool` rather than naming the provenance it
/// found. A third case, an intra-artifact call result, stays refused by
/// [`buffer_returning_call_unsupported`] and never reaches here.
///
/// * **Artifact-owned** (Part 2b of #1142, #1164) -- `a = ndarray(n)`
///   storage, handed to the host through the generated wrapper's exporter
///   type, which frees it in `tp_dealloc`. `owned` is the walker's
///   `owned_buffers` membership.
/// * **Caller-owned** (Part 1 of #1175) -- the name a `memoryview`
///   *parameter* binds. The wrapper hands the host a `memoryview` built
///   directly over the host's own argument object, so the artifact allocates
///   and frees nothing on this path.
///
/// The two are disjoint by construction: `pycc_codegen` never pushes a
/// parameter into `owned_buffer_slots`, and `buffer_parameter_rebinding`
/// keeps a parameter name from ever acquiring owned storage, so a
/// `Ty::MemoryView` name outside `owned_buffers` is a parameter and nothing
/// else. That is exactly why `buffer_bound` is asked only after `owned`.
///
/// `definitely_bound` is **not** a formality and is the one conjunct both
/// walkers share for a reason review round 3 of #1164 recorded: `owned_
/// buffers` joins as a union across control flow while a binding joins on
/// the `Definitely`/`Maybe`/unbound lattice, so `if c: a = ndarray(4)` /
/// `return a` is owned-on-some-path and unbound on another, and admitting it
/// would load a null-initialized slot. A *parameter* is always `Definitely`
/// bound, so the conjunct is free on the caller-owned arm -- but it is
/// spelled rather than assumed, because the two arms share one call site.
///
/// * **Caller-owned sub-range** (Part 2 of #1175, #1179) -- the same
///   parameter binding, returned as `b[start:stop]`. The compiled body
///   still returns the parameter's own view pointer and the bounds travel
///   out of band, so the wrapper's pointer-identity test and Part 1's
///   PEP 688 whole-window check both run unchanged before the sub-range is
///   derived host-side. This provenance is admitted for a parameter only,
///   which is what the `shape` argument decides.
///
/// The caller then owes exactly one further test, once, for every
/// provenance: [`buffer_return_inside_finally`]; a sub-range owes two more,
/// [`buffer_slice_step_unsupported`] and [`buffer_slice_bound_not_an_int`].
pub(crate) fn admits_buffer_egress(
    owned: bool,
    definitely_bound: bool,
    buffer_bound: bool,
    shape: AdmittedBufferReturn<'_>,
) -> bool {
    match shape {
        AdmittedBufferReturn::Whole => definitely_bound && (owned || buffer_bound),
        // Part 2 of #1175 (#1179). `buffer_bound` alone cannot carry this
        // arm and the three-conjunct formula above cannot either: for
        // `a = ndarray(4); return a[1:3]` the walkers compute
        // `owned = true, definitely_bound = true, buffer_bound = true`,
        // which is the *same* triple the admitted artifact-owned bare name
        // produces. Only the explicit `!owned` here keeps the artifact-owned
        // slice unadmitted, so the walk falls through to
        // `owned_buffer_use_unsupported` exactly as it does today. This is
        // the single place that gate is spelled; the walkers'
        // post-admission block must not repeat it.
        AdmittedBufferReturn::Slice { .. } => definitely_bound && buffer_bound && !owned,
    }
}

/// `Err(C0001)` for a buffer sub-range `return` that writes a `step`
/// (Part 2 of #1175, #1179).
///
/// `PyccExtBufferView` is `{ void *ptr; long long len; }` and carries no
/// stride, and D-244 statement (e)'s `PyBUF_C_CONTIGUOUS` acquire refuses a
/// non-contiguous window outright, so a non-unit step is unrepresentable at
/// this boundary. `b[::k]` cannot be decided at compile time at all.
///
/// Refusing `b[1:3:1]` -- which would be sound -- is a **deliberate
/// conservative narrowing**, not an omission: it keeps the rule syntactic
/// and keeps the refusal from depending on constant folding.
///
/// Deliberately not `crate::expr::reject_memoryview_read`'s message, whose
/// remedy ("read one element at a time") would misdescribe this case: the
/// position is admitted, only the `step` is not carried.
///
/// Raised from the walkers' post-admission block rather than from
/// [`admitted_buffer_return`], which is environment-free and whose
/// syntactic shape also matches `return lst[1:3:1]` for a `list[int]` `lst`
/// in a function declared `-> memoryview` -- an ordinary return-type
/// mismatch with no buffer boundary involved. Same placement, and for the
/// same reason, as [`buffer_return_inside_finally`].
pub(crate) fn buffer_slice_step_unsupported(name: &str) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!(
            "returning a slice of `{name}` with a `step` from a `pycc build --ext` export is \
             valid Python but not implemented yet; the compiled boundary carries a pointer and \
             a length and no stride, so only a contiguous sub-range `{name}[start:stop]` is \
             carried (#1179) -- drop the `step`"
        ),
        Span::new(0, 0),
    )
}

/// `Err(T0021)` for a buffer sub-range `return` whose `start` or `stop` is
/// not assignable to `int` (Part 2 of #1175, #1179).
///
/// Word-for-word `crate::expr`'s own `HirExpr::Slice` bound diagnostic,
/// shared from here because the admitted branch exits before that arm runs
/// (D1) and both walkers raise it from their post-admission block. A
/// separate wording would make the same program report two different
/// messages depending on whether its `return` was admitted.
pub(crate) fn buffer_slice_bound_not_an_int(label: &str, bound_ty: &Ty) -> Diagnostic {
    Diagnostic::error(
        "T0021",
        format!("slice {label} must be `int`, got `{}`", bound_ty.name()),
        Span::new(0, 0),
    )
    .with_help("use an `int` value")
}

/// `Err(C0001)` for a call whose callee returns the buffer type.
///
/// Part 2b of #1142 (#1164) admits a buffer return at the `pycc build --ext`
/// boundary -- where the generated wrapper turns the artifact's storage into
/// a real `memoryview` the host owns -- and nowhere else. An *intra-artifact*
/// call to such a function has no such wrapper: the compiled callee hands
/// back a raw `PyccExtBufferView *` with no owner, which
/// `crates/pycc_codegen/src/call_result.rs`'s `Ty::MemoryView` arm panics on.
/// This refusal is what keeps that panic unreachable from source, which is
/// #1164's own completion criterion.
pub(crate) fn buffer_returning_call_unsupported(callee: &str) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!(
            "calling `{callee}`, whose return type is a buffer, is valid Python but not \
             implemented yet; #1164 hands such a buffer to the CPython host across the \
             `pycc build --ext` boundary and admits no intra-artifact caller -- allocate \
             the buffer with `a = ndarray(n)` in the function that reads it"
        ),
        Span::new(0, 0),
    )
}

/// `Err(C0001)` for a call to a *method* whose return type is the buffer
/// type.
///
/// The method-shaped sibling of [`buffer_returning_call_unsupported`], which
/// serves the module-level `f()` route in `expr.rs`. #1174 widens the
/// `pycc build --ext` admission from a public module-level `def` to a public
/// method of a public class, so a buffer-returning method now *exists* in a
/// well-formed program, and every intra-artifact route to its return value
/// has to be refused where the module-level route already was. The reason is
/// unchanged: the compiled callee hands back a raw `PyccExtBufferView *` with
/// no owner, which `crates/pycc_codegen/src/call_result.rs`'s `Ty::MemoryView`
/// arm panics on. Only the generated `--ext` wrapper gives that pointer an
/// owner.
///
/// `class_name` is the class the *resolver* was asked about, which for a
/// `super().m()` call is the calling method's own class rather than the base
/// class that declares `m`. The renderer states where the call was written,
/// not where the callee lives; the alternative would need the resolved MRO
/// entry threaded through every exit for no reader benefit.
pub(crate) fn buffer_returning_method_call_unsupported(
    class_name: &str,
    method: &str,
) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!(
            "calling `{class_name}.{method}`, whose return type is a buffer, is valid \
             Python but not implemented yet; #1174 hands such a buffer to the CPython host \
             across the `pycc build --ext` boundary and admits no intra-artifact caller -- \
             allocate the buffer with `a = ndarray(n)` in the function that reads it"
        ),
        Span::new(0, 0),
    )
}

/// The interception every method-call resolver exit shares: refuse the call
/// when the resolved return type is the buffer type, otherwise fall through.
///
/// Placed at each exit *after* the callee's signature is known and *before*
/// `check_call_args`, matching `expr.rs`'s own ordering for the module-level
/// route, so a wrong-argument call to a buffer-returning method reports the
/// unsupported-return refusal rather than an argument mismatch the caller
/// cannot act on.
pub(crate) fn refuse_buffer_returning_method(
    class_name: &str,
    method: &str,
    return_ty: &Ty,
) -> Result<(), Diagnostic> {
    if matches!(return_ty, Ty::MemoryView) {
        return Err(buffer_returning_method_call_unsupported(class_name, method));
    }
    Ok(())
}

/// `Err(C0001)` for a buffer **egress** in a function that also contains a
/// `return` inside a `finally` clause (Part 2b of #1142, #1164 review
/// round 5).
///
/// The narrowing that makes the single pending-return record's own
/// precondition checked rather than assumed. `crates/pycc_codegen/src/lib.rs`
/// tracks a returned buffer's ownership in **one** per-frame record -- a
/// pointer slot plus an orphan flag -- which can describe exactly one
/// suspended return. A `return` lexically inside a `finally` is the only
/// shape that puts two returns in flight at once: the outer `return` is
/// suspended while its finalizer runs, the inner one overwrites the record
/// and releases the orphaned predecessor, and a finalizer that then raises
/// cancels the inner return so the outer one resumes -- handing the host a
/// `memoryview` over freed storage and segfaulting the interpreter.
///
/// Refusing the *admission* rather than growing the record is deliberate.
/// Four review rounds on this mechanism each closed one path into it; the
/// cardinality assumption underneath them is what this closes, and the
/// whole class with it. A correct multi-pending egress needs a stack of
/// records keyed by suspended-return context, which is tracked separately.
///
/// Raised only at the egress admission, so a `-> int` function with a
/// `return` inside a `finally` keeps exactly the behavior it has today:
/// this narrows what buffer egress admits, and nothing else.
///
/// pycc already refuses a *bare* `return` inside a `finally` with `L0001`
/// (PEP 765, #738). That check is syntactic and follows CPython in clearing
/// its `finally` context on loop entry, so `while True: return a` escapes
/// it; `pycc_hir::body_returns_inside_finally` is the transitive predicate
/// this refusal uses instead. `C0001` is the code because this is a
/// capability gap in an unimplemented feature, not a context violation --
/// the same ground as every other refusal in this module (D-148).
pub(crate) fn buffer_return_inside_finally(name: &str) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!(
            "returning `{name}`, which is bound to buffer storage this `pycc build --ext` \
             artifact allocated, from a function that also contains a `return` inside a \
             `finally` clause is valid Python but not implemented yet; the compiled frame tracks one \
             pending buffer return per call, and a `return` inside a `finally` can leave \
             a second one suspended (#1173) -- move the inner `return` out of the \
             `finally` clause"
        ),
        Span::new(0, 0),
    )
}
