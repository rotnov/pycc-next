//! Foreign (CPython-object) import bindings in the type environment
//! (Part 1 of #1026, PR 1c of #1080).
//!
//! `import numpy` binds `numpy` to [`Ty::Object`], an opaque `PyObject *`
//! whose shape pycc knows nothing about. Since #1278 an unaliased
//! top-level `from itertools import product, chain` binds each imported
//! name the same way -- `product` is the CPython object
//! `itertools.product`, typed [`Ty::Object`] like any other foreign
//! binding, so every refusal below applies to it unchanged. Every refusal of an operation on
//! such a value is `I0404`.
//!
//! **Part 2 of #1026 (#1081) moved the refusal from the producer to the
//! consumers, and this paragraph replaces the containment invariant Part 1
//! rested on.** Part 1 refused the *read* of a foreign binding, which
//! refused every derived operation at once without a refusal per
//! operation. That works only while no operation is supported. Part 2
//! supports `numpy.pi`, and `infer_expr_in` is context-free -- it cannot
//! see whether the read it is answering feeds an attribute load or a
//! `print` -- so a conditional relaxation of the read is not expressible.
//! The read is therefore admitted *in a module body* and each **consuming**
//! site refuses on its own.
//!
//! Two consequences follow, and both are load-bearing:
//!
//! 1. A `Ty::Object` value no longer has a name. The diagnostic builder
//!    below names the **operation** instead ([`object_operation_unsupported`]),
//!    because the consumer knows what it was about to do while the value
//!    it holds is an anonymous temporary.
//! 2. There are now several producer shapes, not one: `o.attr`
//!    (`HirExpr::AttrGet` over a `Ty::Object` base), a call to an
//!    unannotated private helper whose inferred return is `Ty::Object`
//!    (`constraints.rs`'s `AttrGet` term), `o.method(...)` (PR 2b of
//!    #1081), `o[k]` (PR 3b of #1082), and the loop variable of
//!    `for x in <object>:` (`HirStmt::ForObject`, PR 3c of #1082, which
//!    binds `x` to `Ty::Object` directly rather than through
//!    `check_assignment`). Every refusal must
//!    therefore key on the **type**, never on the producing expression
//!    shape, and the list is expected to keep growing.
//!
//! **PR 2a of #1081 bounded the admitted read by position; #1316 widened
//! the bound to function bodies.** A foreign object may be read only where
//! the compiler can report a failed lookup and a read before the `import`:
//!
//! * a module body, where a read placed *above* the `import` is an
//!   ordinary unbound-name `T0021` -- see [`bind_foreign_objects_at`] --
//!   and a failed operation stops the module exec;
//! * a function body, method body or unannotated helper, for any
//!   definitely-bound `Ty::Object` name: a module-level foreign import
//!   (#1316), a module-level name a `x = <object>` statement bound (#1325),
//!   a function local, or an unannotated helper's solver-inferred `object`
//!   parameter (Part 1 of #1333). D-041 checks a body against the module
//!   environment as it stands after all top-level code, so it cannot see
//!   whether the call site precedes the module-level binding;
//!   `pycc_codegen` answers that read at run time with the `NameError`
//!   CPython raises, and bridges a failed operation into pycc's pending
//!   exception so the body's own `try` can catch it (`pycc_codegen`'s
//!   `foreign_fail.rs`).
//!
//! **Part 1 of #1333 admitted holding an object beyond one expression in a
//! function body**: binding it to a local (`check_assignment`), returning
//! it, and passing it to an ordinary pycc-compiled function. No reference
//! is released on scope exit -- the #1092 leak-only rule extends to function
//! locals unchanged (`docs/RUNTIME.md`). Passing one to a *generic* function
//! stays refused ([`reject_object_arguments`]), and so does a `for` over an
//! object inside a function body (Part 2 of #1333, #1363). An unannotated
//! helper's parameter used as a method-call base or a callee is still a
//! solver variable when its body is walked, so it reports `T0021` (Part 3 of
//! #1333, #1364).
//!
//! **#1325 admitted binding an object to a name in a module body.** At
//! module scope `x = product("ab")` stores the new reference into an
//! ordinary module global, which since Part 1 of #1499 owns it and releases
//! the old reference on rebind (`pycc_codegen`'s `object_slot.rs`); Part 1
//! of #1333 removed `check_assignment`'s
//! remaining function-body refusal. `for t in x:` over such a name is the bare-name form
//! of `HirStmt::ForObject`: `check_stmt`'s `ForList` arm routes a
//! definitely-bound `object` name to the same checks ([`for_loop`]).
//!
//! `docs/TYPE_SYSTEM.md` carries the user-facing statement of both.
//!
//! **PR 2b of #1081 added the second supported operation: a method call.**
//! `expr::infer_expr_in`'s `HirExpr::MethodCall` arm now answers
//! [`Ty::Object`] for a `Ty::Object` base, where it previously fell through
//! to `class::resolve_method_call`'s "not a class instance" `T0043`. The
//! branch admits only *positional* arguments (`HirExpr::MethodCall` carries
//! no keyword arguments at all) whose types are `int`, `float`, `bool` or
//! `str` -- the four scalars the shim has a `pycc_ext_obj_pack_*` helper
//! for -- and refuses every other argument type with
//! [`object_operation_unsupported`]; Part 2a of #1371 widened the admitted
//! set with a second `Ty::Object` (packed by `pycc_ext_obj_pack_object`) and,
//! since #1435, an instance of a regular pycc class
//! (`pycc_ext_obj_pack_instance`; see [`check_object_call_args`]). The
//! call inherits the positional bound unchanged: the base is read through
//! the same `HirExpr::Name` arm. Part 8 of #1371 admits `None` and keyword
//! arguments (`keyword_call`).
//!
//! Four method names reach this arm through a second node. `pycc_hir`'s
//! container fast paths claim `append`, `pop`, `get` and `add` while
//! lowering, from their spelling alone. Issue #1095: in a module that can
//! hold a CPython object -- an `ext` module (D-258) or one that has bound a
//! foreign import (D-244 rule 3) -- such a call is a
//! `HirExpr::ReceiverDispatchedCall` keeping both readings (the node #1188
//! introduced for user classes), and `pycc_hir::receiver_takes_method_path`
//! sends a `Ty::Object` receiver to the method reading, which is this arm.
//! So `gc.get(1, 2)`, `gc.get(1)` and `value_stack.append(x)` are the
//! foreign method call whatever their arity, while a native `list`, `dict`
//! or `set` receiver keeps the container reading and its diagnostics.
//! `docs/TYPE_SYSTEM.md`'s `object` row owns that statement.
//!
//! **PR 3a of #1082 (Part 3 of #1026) added `len` and truth testing, and
//! deleted a whole class of refusal.** `len(o)` type-checks to `Ty::Int`
//! (`expr.rs`'s and `constraints.rs`'s `len` guards both admit
//! [`Ty::Object`] now), and an `if`/`while` test or a comprehension guard
//! places no constraint on its operand at all -- `pycc_codegen`'s `truthy`
//! grew a `Scalar::Object` arm that calls the shim's `pycc_ext_obj_truthy`,
//! so the ten `reject_object_condition` sites that existed only to keep an
//! object away from a codegen panic are gone. Nothing else about the
//! positional bound changes: both operations read their operand through the
//! same `HirExpr::Name` arm.
//!
//! `not o` was *not* part of this: `unop.rs`'s `Not` arm answered `T0021`
//! for an object operand until Part 10 of #1371 admitted it as a native
//! `bool` over the same truth test.
//!
//! **PR 3b of #1082 added another producer shape: a subscript load.**
//! `o[k]` type-checks to [`Ty::Object`] (`expr.rs`'s `HirExpr::Subscript`
//! arm has a `Ty::Object` base arm ahead of the `T0033` catch-all, and
//! `constraints.rs`'s own `Subscript` arm lifts the term exactly as its
//! `AttrGet` arm does), so a key is now a fourth place a foreign value can
//! be consumed. The admitted keys are the same four scalars a method call's
//! arguments are -- `int`, `float`, `bool`, `str`, the ones with a
//! `pycc_ext_obj_pack_*` helper -- and every other key type is refused with
//! [`object_operation_unsupported`] naming the key's type. Part 2a of #1371
//! widened the admitted keys with a second [`Ty::Object`].
//!
//! Only the **load** is admitted. `o[k] = v` stays refused: it is a
//! different HIR shape, which `pycc_hir` rejects with `C0001` ("only
//! assigning to a bare-name subscript target") before this crate sees it.
//! `x = o[k]` is admitted in a module body since #1325 and in a function
//! body since Part 1 of #1333. A *slice* (`o[a:b]`) is a third shape
//! again, admitted as a load since Part 2b of #1371 by [`slice`]; every
//! non-object base keeps `expr.rs`'s `HirExpr::Slice` `T0033`. The
//! positional bound is inherited unchanged.
//!
//! **PR 4a of #1083 (Part 4 of #1026) added two conversions *out* of the
//! opaque type: `float(o)` and `bool(o)`.** Both are handled in `expr.rs`'s
//! `HirExpr::Call` arm (and mirrored in `constraints.rs`) rather than here:
//! `float`'s existing argument gate gains [`Ty::Object`], and `bool` gets a
//! new arm admitted for [`Ty::Object`] **only**, so `bool(1)` keeps its
//! `C0001` verbatim. Both arms carry `float`'s user-defined-function guard,
//! because a `def float(...)`/`def bool(...)` is a valid working program
//! today and must keep winning.
//!
//! These are the first operations that produce a *pycc-native* value from an
//! object, so they are also the first that leave nothing behind: the shim
//! releases its own CPython temporary on every exit and no new reference
//! escapes, which is why Part 4 does not grow #1092. They run CPython's own
//! conversion protocol (`PyNumber_Float`, `PyObject_IsTrue`), which is *not*
//! a D-244 rule-7 violation: rule 7 closes the boundary at the thunk export
//! seam, where a value crosses implicitly and the annotation is the whole
//! contract, whereas `float(o)` in user source is an explicit conversion
//! request that names its destination type. `docs/TYPE_SYSTEM.md`'s `object`
//! row carries the user-facing statement.
//!
//! The residual incoherence is stated rather than hidden: `bool(o)` compiles
//! while `bool(1)` is still `C0001`, because Part 4 relaxes exactly the
//! object case and leaves the general builtin-conversion story to
//! #1017/#1018. The positional bound is inherited unchanged. (Part 4 left
//! `print(o)` itself `I0404` while `print(float(o))` type-checked; #1340
//! admits `print(o)` and `f"{o}"` too -- see below.)
//!
//! **PR 4b of #1083 adds the other two conversions out of the opaque type:
//! `int(o)` and `str(o)`.** Both are new `expr.rs` arms (mirrored in
//! `constraints.rs`) on the `bool` arm's exact shape -- [`Ty::Object`]
//! **only**, both the user-defined-function and the user-defined-class guard
//! on the whole arm, since neither arm admits anything else. `int(o)` runs
//! `PyNumber_Long` and D-141-encodes the result, refusing a value outside the
//! inline-integer range `[-2**62, 2**62-1]` with `OverflowError` (#1040, no
//! bigint path); `str(o)` runs `PyObject_Str` and copies the UTF-8 out with
//! `pycc_rt_str_from_literal`, producing exactly what a `str` literal
//! produces. Both release their CPython temporary on every exit, so Part 4
//! still adds nothing to #1092.
//!
//! The same asymmetry is inherited and stated: `str(o)` compiles while
//! `str(1)` keeps its `C0001`.
//!
//! **#1340 admits rendering the object itself: `print(o)` and `f"{o}"`.**
//! `string_conversion.rs`'s `Ty::Object` refusal is gone, and
//! `pycc_codegen` renders the operand through the shim as CPython does --
//! `print` calls `str()` (`pycc_ext_obj_to_str`) in its write phase, after
//! every argument is evaluated, and an f-string part calls
//! `format(o, '')` (`pycc_ext_obj_format`, the operand's `__format__`) as
//! soon as it is evaluated. Conversion flags and format specs (`f"{o!r}"`,
//! `f"{o:>8}"`) stay refused in `pycc_hir` for every operand type.
//!
//! **PR 4c of #1083 admits the opaque type at one *annotated assignment*.**
//! `x: tuple[float, float, float] = <object>` in a module body -- and more
//! generally any fixed-arity annotation whose every element is `float` --
//! now compiles. This is the first Part 4 operation that is not a call:
//! `check_stmt`'s module-level `HirStmt::AnnAssign` arm tests
//! [`is_object_float_tuple_annotation`] *ahead of* `is_assignable_env`, so
//! the relaxation is a branch in that one arm rather than a widening of
//! `is_assignable`, which would have admitted the pair everywhere a value
//! flows into a declared type.
//!
//! **Strict container, converting elements.** The shim checks
//! `PyTuple_Check` with an exact-arity test and then runs `PyNumber_Float`
//! on each item; the runtime object's items are never type-checked by pycc,
//! and a bad item fails at run time with whatever CPython raises. The two
//! halves answer two different questions -- D-115/D-116 leave no shape for
//! a differently-sized sequence, while `float` is a type the author wrote,
//! which makes converting to it the same explicit-conversion case PR 4a
//! records.
//!
//! Everything else stays refused, and the refusals are two different
//! diagnostics: a *mixed* annotation such as `tuple[float, int]` is the
//! ordinary `T0025` this arm already produced, while PEP 585's variadic
//! `tuple[float, ...]` never reaches this crate -- `pycc_hir` refuses the
//! `...` type argument with `T0053` while lowering the annotation. The
//! in-function arm gains no branch at all: since #1316 the read of a
//! module-level foreign name is admitted there, and the ordinary
//! assignability check refuses the `object` value against the declared
//! tuple with `T0025`, as PR 4c's own test pins.
//!
//! **#1313 added a direct call of an `object`-typed name** (`product("ab")`
//! after `from itertools import product`, or a call of a `for` loop target
//! bound to an object). `expr::infer_expr_in`'s
//! `HirExpr::Call` arm answers [`Ty::Object`] for a callee bound to
//! [`Ty::Object`] in a module body -- and, since #1316 and Part 1 of #1333,
//! in a function body too -- under the
//! same argument rule as a method call ([`check_object_call_args`]: a
//! scalar, `None`, object or (since #1435) class-instance argument,
//! positional or, since Part 8 of #1371,
//! keyword -- `keyword_call`); the
//! constraint solver's own `Call` arm answers the same term. Part 2 kept
//! the call refused because admitting `f(2.0)` also admits `numpy(1)`,
//! which CPython answers with `TypeError: 'module' object is not
//! callable`. That is now the intended reading: the call is compiled and
//! the host raises exactly that `TypeError`, on the same failure edge
//! every other object operation uses: catchable by an enclosing `try` in a
//! function body (#1316) and in the module body (Part 1 of #1096).
//!
//! [`reject_object_read`] serves the one site that keys on a *named*
//! binding rather than on a consumed value: `lookup_bound_name` (D-105's
//! `ForList`/`ListAppend` HIR shape carries its list as a plain `String`, so
//! it never becomes a `HirExpr::Name`). Part 1 of #1333 removed the other
//! two -- the in-function read and the in-function call of an `object`
//! value -- because both are admitted now.

use crate::Environment;
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{ForeignImportSite, FromImport, HirExpr, HirStmt, ImportBinding, Ty};

/// Whether a declared annotation is a fixed-arity tuple whose every
/// element is `float` -- the one annotation a [`Ty::Object`] initializer
/// may be assigned to (Part 4 of #1026, PR 4c of #1083).
///
/// This is the **canonical statement of that admission rule**. Two mirrors
/// restate it and must not drift: `pycc_mir::stmt`'s
/// `float_tuple_annotation_arity`, which decides whether the lowering emits
/// `MirExpr::ObjUnpackFloatTuple`, and the shim helper's own exact-arity
/// `PyTuple_Check`, which enforces the container half at run time. Neither
/// can share this function -- `pycc_mir` does not depend on this crate, and
/// the shim is C.
///
/// **Fixed arity only.** PEP 585's variadic `tuple[float, ...]` is not
/// admitted and does not reach here at all: `pycc_hir` refuses the `...`
/// type argument with `T0053` while lowering the annotation, which is a
/// different diagnostic from the `T0025` a *mixed* annotation such as
/// `tuple[float, int]` still gets from the caller below. Both stay refused.
///
/// The emptiness test guards `tuple[()]`, which `pycc_hir` also refuses
/// with `T0053` before this crate runs; it is kept because the rule is
/// "arity at least one", and because codegen's out-slot is an
/// `[arity x double]` array that a zero arity would make degenerate.
pub(crate) fn is_object_float_tuple_annotation(ty: &Ty) -> bool {
    matches!(ty, Ty::Tuple(elems) if !elems.is_empty() && elems.iter().all(|elem| matches!(elem, Ty::Float)))
}

/// The `help` of a `T0022` or `T0025` whose value is a CPython object and
/// whose declared slot is one of the four scalars an explicit conversion
/// produces (`bool`, `int`, `float`, `str`), or `None` for every other pair.
///
/// #1419 settled that such a slot keeps refusing the object rather than
/// converting it implicitly (D-258's 2026-10-05 #1419 amendment). The
/// motivating value is a rich comparison with an object operand, typed
/// `object` because `__eq__` may return anything; a `-> bool` return of it
/// stays the ordinary mismatch, and this help names the explicit way out
/// (`bool(a == b)`), which D-258 rule 5 requires.
pub(crate) fn object_into_scalar_help(actual: &Ty, declared: &Ty) -> Option<String> {
    (matches!(actual, Ty::Object) && matches!(declared, Ty::Bool | Ty::Int | Ty::Float | Ty::Str))
        .then(|| {
            let name = declared.name();
            format!(
                "a CPython object reaches `{name}` slots only through an explicit conversion: wrap the value in `{name}(...)`"
            )
        })
}

/// The diagnostic every unsupported operation on a CPython object gets.
///
/// `operation` is a noun phrase naming what the *consumer* was about to
/// do, written so the message reads as a sentence: "matching on a CPython
/// object is not supported yet". Part 1 interpolated the binding's local
/// name here instead; Part 2's refusals sit at consuming sites that hold
/// an anonymous temporary and have no name to report (see the module doc).
pub(crate) fn object_operation_unsupported(operation: &str) -> Diagnostic {
    Diagnostic::error(
        "I0404",
        format!(
            "{operation} is not supported yet -- pycc models a CPython object as an opaque \
             value and implements attribute access, store and deletion, scalar-, `None`-, \
             object- or class-instance-argument method calls and direct calls with \
             positional or keyword arguments \
             (including a call of a subscript result), `len`, truth \
             testing and `not`, `type`, a \
             scalar-, object- or class-instance-key subscript load, a slice load or deletion \
             with scalar or object bounds, a rich comparison with an object, scalar or \
             class-instance operand, an identity test against an object or `None`, \
             a membership test of a scalar or object item in an object, \
             a list display of scalar, object or class-instance elements bound to an object \
             slot, \
             `and`/`or` with an object or scalar operand, \
             `isinstance` against a foreign class, a plain pycc class or \
             `int`/`float`/`bool`/`str`/`list`/`dict`/`tuple`, `for` iteration, a list \
             or set comprehension over it unless it is a bare name, binding the \
             value to a name, returning it from and passing it to a pycc \
             function, boxing a scalar, `None` or class-instance value into an \
             `object` binding, parameter, attribute or return value, printing it \
             and f-string \
             interpolation, the `float`, \
             `bool`, `int` and `str` conversions and an annotated \
             module-level assignment to a fixed-arity all-`float` `tuple` \
             on it and nothing else"
        ),
        Span::new(0, 0),
    )
}

/// Whether a value of type `ty` can be handed to the shim as a call
/// argument or a subscript key: the four scalars -- `int`, `float`, `bool`
/// and `str` -- and, since Part 2a of #1371, a second `object`, each of
/// which has a `pycc_ext_obj_pack_*` helper in the shim
/// (`pycc_ext_obj_pack_object` takes one new reference to the operand).
///
/// The narrow operand rule a membership item, an object comprehension's
/// element and a slice bound keep; [`check_object_call_args`] and
/// [`is_object_operand`] widen it.
pub(crate) fn is_packable_operand(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::Object)
}

/// Whether a value of type `ty` can be boxed opposite a CPython object as a
/// subscript key (`expr.rs`'s `Ty::Object` arm), a rich-comparison operand
/// ([`compare::rich_compare_ty`]) or a list-display element
/// ([`list_display`]): a packable operand ([`is_packable_operand`]) or,
/// since #1470, an instance of a regular user class
/// ([`is_carriable_instance`]), which crosses as its `PyccExtInstance`
/// carrier (`pycc_ext_obj_pack_instance`).
///
/// The carrier makes the instance a key, an operand and an element CPython
/// treats as the program's own object: since #1427 its type compares and
/// hashes through the class's `__eq__`/`__hash__` and siblings, and since
/// #1470 an embedded executable's carriers get the same slots as an
/// `--ext` module's, so the one rule holds in both artifacts. A class that
/// defines none of them compares and hashes by identity, as CPython's
/// `object` does. Each site also refuses a class method's own `cls`
/// ([`refuse_classmethod_cls`]), which this type-level test cannot see.
pub(crate) fn is_object_operand(env: &Environment, ty: &Ty) -> bool {
    is_packable_operand(ty) || is_carriable_instance(env, ty)
}

/// `Err(I0404)` naming `operation` when `expr` is a class method's own
/// `cls`, which is typed as an instance of its class but holds none (see
/// `Environment::in_classmethod`). CPython would hand over the class object,
/// which pycc has no boundary representation for.
///
/// The check is syntactic: an alias of `cls` that slips past it reaches the
/// shim's null guard and raises `SystemError` instead.
pub(crate) fn refuse_classmethod_cls(
    env: &Environment,
    expr: &HirExpr,
    operation: &str,
) -> Result<(), Diagnostic> {
    if env.in_classmethod && matches!(expr, HirExpr::Name(name) if name == "cls") {
        return Err(object_operation_unsupported(operation));
    }
    Ok(())
}

/// `Err(I0404)` unless every argument of a call on a CPython object is
/// packable ([`is_packable_operand`]), `None`, or an instance of a regular
/// user class ([`is_carriable_instance`]).
///
/// The one statement of the argument rule every object-call shape shares: a
/// method call (`o.method(args)`, PR 2b of #1081, `what` = `"method"`), a
/// direct call of an `object`-typed name (`product(args)`, #1313, `what` =
/// `"call"`) and a call of an `object`-typed subscript result
/// (`table[k](args)`, Part 2a of #1371, also `"call"`), each with positional
/// or, since Part 8 of #1371, keyword arguments -- and the value of an
/// attribute store through an object-typed name (`o.x = v`, #1457, `what` =
/// `"attribute store"`), checked as a single argument; `args` are the argument
/// expressions whose types are `arg_tys`, the keyword values for a keyword
/// call. `None` is admitted since Part 8 too, as an argument only: codegen
/// passes CPython's own `Py_None` for it. Subscript keys, comparison
/// operands and list-display elements keep the narrower
/// [`is_object_operand`] rule, which has no `None`. Anything else
/// -- a container, an enum member or an exception instance -- has no
/// boundary representation yet and is refused here rather than reaching
/// codegen, naming the first offending argument's type.
///
/// The instance arm (#1435) crosses as the instance's `PyccExtInstance`
/// carrier (`pycc_ext_obj_pack_instance`), exactly as it does in the three
/// positions [`is_object_operand`] admits since #1470.
///
/// A `@classmethod`'s own `cls` is refused by name
/// ([`refuse_classmethod_cls`]).
pub(crate) fn check_object_call_args<'a>(
    env: &Environment,
    args: impl IntoIterator<Item = &'a HirExpr>,
    arg_tys: &[Ty],
    what: &str,
) -> Result<(), Diagnostic> {
    let operation = format!("passing a class method's `cls` argument to a CPython object's {what}");
    for arg in args {
        refuse_classmethod_cls(env, arg, &operation)?;
    }
    for arg_ty in arg_tys {
        if !is_packable_operand(arg_ty)
            && !matches!(arg_ty, Ty::None)
            && !is_carriable_instance(env, arg_ty)
        {
            return Err(object_operation_unsupported(&format!(
                "passing a `{}` argument to a CPython object's {what}",
                arg_ty.name()
            )));
        }
    }
    Ok(())
}

/// Whether `ty` is an instance of a regular user class, which crosses into
/// CPython as a call argument boxed in a `PyccExtInstance` carrier (#1435).
///
/// A regular class is one the program defines that is neither an enum nor
/// an exception class. An enum member is allocated without the layout
/// descriptor the carrier names its type from, and an exception instance
/// has its own boundary (the D-244 exception bridge) that a carrier would
/// contradict; both stay refused.
///
/// An exception class is recognized by its MRO reaching a builtin exception
/// class -- the rule `HirClassDef::exception_type_tag` is assigned by --
/// read directly, because the tag is only assigned when the program is
/// finalized and this check also runs on a module checked on its own. The
/// tag is consulted as well, for the bridged foreign exception base (#1316)
/// whose MRO names no builtin.
fn is_carriable_instance(env: &Environment, ty: &Ty) -> bool {
    let Ty::Instance(name) = ty else {
        return false;
    };
    env.lookup_class(name).is_some_and(|class| {
        !class.is_enum
            && class.exception_type_tag.is_none()
            && !class
                .mro
                .iter()
                .any(|entry| pycc_hir::is_builtin_exception_class(entry))
    })
}

/// `Err(I0404)` when `ty` is the opaque object type, `Ok(())` otherwise.
///
/// The guard for the one site that keys on a *named* binding rather than
/// on a consumed value -- `lookup_bound_name` (module doc). A
/// consuming site calls [`object_operation_unsupported`] directly instead,
/// because it knows the operation and its operand has no name.
pub(crate) fn reject_object_read(name: &str, ty: &Ty) -> Result<(), Diagnostic> {
    if matches!(ty, Ty::Object) {
        return Err(object_operation_unsupported(&format!(
            "using `{name}`, which is bound to a CPython object, in this position"
        )));
    }
    Ok(())
}

/// `Err(I0404)` when `ty` is the opaque object type, naming `operation`.
///
/// The consumer-side counterpart of [`reject_object_read`]: one helper so
/// the remaining consuming sites -- `check_match` (`lib.rs`),
/// `check_isinstance` (`class.rs`) and [`reject_object_arguments`] --
/// cannot drift into several spellings of the same rule.
///
/// Part 3 of #1026 (PR 3a of #1082) removed the largest group of callers:
/// the ten `if`/`while`/comprehension-guard condition sites, which refused
/// `Ty::Object` only because `pycc_codegen`'s `truthy` had no object arm.
/// It has one now, so a condition no longer consults this helper at all.
pub(crate) fn reject_object_operand(ty: &Ty, operation: &str) -> Result<(), Diagnostic> {
    if matches!(ty, Ty::Object) {
        return Err(object_operation_unsupported(operation));
    }
    Ok(())
}

/// Refuses a CPython object as an argument to a generic function (#1316).
///
/// A generic function calls this over every argument before substitution:
/// monomorphization only instantiates `int`, `float`, `bool` or `str`, so no
/// instance could take the object. Since Part 1 of #1333 an *ordinary*
/// function admits one: an unannotated private helper's solver-inferred
/// `object` parameter accepts it, and since Part 1 of #1367 so does any
/// parameter -- of a function, a method or a constructor -- annotated with a
/// class a foreign import binds (`docs/TYPE_SYSTEM.md`'s `object` row;
/// outside an `--ext` module `object` itself stays unspellable, while under
/// D-258 an `Any`/`object` parameter of an ext build is the same object).
/// Every other declared parameter
/// refuses an `object` argument through the ordinary mismatch.
pub(crate) fn reject_object_arguments(arg_tys: &[Ty]) -> Result<(), Diagnostic> {
    arg_tys
        .iter()
        .try_for_each(|ty| reject_object_operand(ty, PASSING_TO_A_FUNCTION))
}

/// The `I0404` operation phrase for a CPython object passed to a generic
/// function (see [`reject_object_arguments`]).
pub(crate) const PASSING_TO_A_FUNCTION: &str = "passing a CPython object to a generic function";

/// The local names a module's import table binds to a CPython object, in
/// source order.
pub(crate) fn foreign_object_names(imports: &[ImportBinding]) -> Vec<&str> {
    imports
        .iter()
        .filter_map(|binding| match binding {
            ImportBinding::Foreign { local_name, .. } => Some(local_name.as_str()),
            ImportBinding::Module { .. }
            | ImportBinding::Symbol { .. }
            | ImportBinding::Project { .. } => None,
        })
        .collect()
}

/// Seeds every foreign import as a definitely-bound `Ty::Object` global,
/// with no regard for the import's own position.
///
/// **The check phase no longer uses this.** Part 1 of #1026 seeded here
/// before the source-order top-level pass (D-041 pass 2), which made a read
/// placed *above* the `import` `I0404` rather than the `T0021` CPython's
/// `NameError` would justify -- sound only while Part 1 refused the read
/// unconditionally. Part 2 admitted the read, and the seed then admitted
/// the whole program: it lowered, built, and trapped at run time
/// (`llvm.trap`, rc 133) on the global-initialization failure edge. PR 2a
/// of #1081 removed the seed from `crate::module`, so such a read is now an
/// ordinary unbound-name `T0021` -- which is also the closer answer.
/// [`bind_foreign_objects_at`] does the positional binding the checker
/// keeps, and `docs/TYPE_SYSTEM.md` records the outcome.
///
/// The one remaining caller is [`crate::empty_container`]'s pre-pass, which
/// wants the position-blind form on purpose: it reports no diagnostic, so a
/// wider module scope than the checker's is the safe direction for it (that
/// call site carries the full argument).
///
/// D-040's sticky-representation rule is unaffected: a later `numpy = 3` in
/// the same module is `T0023`, because the name's recorded representation
/// is `object` and `int` is not assignable to it.
pub(crate) fn bind_foreign_objects(env: &mut Environment, imports: &[ImportBinding]) {
    for name in foreign_object_names(imports) {
        env.bind(name.to_string(), Ty::Object);
    }
}

/// Applies every foreign import recorded at `position` in the item list,
/// at the point the source-order pass reaches that position.
///
/// A `ForeignImportSite::Item` index is the item count at the moment the
/// `import` lowered, so the statement runs immediately *before* item
/// `position`; a trailing import records the item count itself, which the
/// caller applies once the loop is done. `Environment::bind` clears the
/// name's `def_rebound` mark, which is precisely what makes a later call
/// reach the `I0404` refusal instead of the stale function pointer -- and,
/// symmetrically, a `def` *below* the import re-marks the name and keeps
/// working, matching CPython's own last-binding-wins order.
///
/// A [`ForeignImportSite::Block`] import is never bound here: its own
/// `HirStmt::ForeignImport` statement binds it where it runs (#1291).
pub(crate) fn bind_foreign_objects_at(
    env: &mut Environment,
    imports: &[ImportBinding],
    position: usize,
) {
    for binding in imports {
        if let ImportBinding::Foreign {
            local_name,
            site: ForeignImportSite::Item(index),
            ..
        } = binding
            && *index == position
        {
            env.bind(local_name.clone(), Ty::Object);
        }
    }
}

/// Binds each local name of a `HirStmt::ForeignImport` (a foreign `import`
/// or `from ... import` nested in a module-level `if`/`try` block, #1291,
/// #1383) to `Ty::Object` at the statement's own position. The enclosing
/// `if`/`try` join then decides whether a read after the block is
/// definitely assigned.
pub(crate) fn bind_block_import(
    env: &mut Environment,
    bindings: &[(String, String, Option<FromImport>)],
) {
    for (local_name, _, _) in bindings {
        env.bind(local_name.clone(), Ty::Object);
    }
}

/// Seeds `handler_env` for an `except ImportError` fallback (#1485): each
/// name a foreign import written directly in the `try` body binds, and the
/// handler body rebinds, is possibly bound as `object` on entry to the
/// handler -- the import may have run before the failure, and a fallback
/// rebinding stores into the same `object` slot. `check_assignment` then
/// boxes the handler's `name = None` (D-258's #1475 amendment) rather than
/// typing it as a fresh `None` binding the `try` join would refuse. A
/// handler that only reads the name is not seeded, so its read stays the
/// unbound `T0021`; a name already bound before the `try` keeps its type.
/// HIR admits only the fallback shapes (`crate::import::fallback_groups` in
/// `pycc_hir`); every other rebinding of a foreign import is refused there.
pub(crate) fn seed_fallback_rebindings(
    handler_env: &mut Environment,
    body: &[HirStmt],
    handler_body: &[HirStmt],
) {
    let rebound = pycc_hir::killed_names(handler_body);
    for stmt in body {
        if let HirStmt::ForeignImport { bindings, .. } = stmt {
            for (local, _, _) in bindings {
                if rebound.contains(local) && handler_env.lookup_any(local).is_none() {
                    handler_env.bind_maybe(local.clone(), Ty::Object);
                }
            }
        }
    }
}

pub(crate) mod attr_store;
pub(crate) mod compare;
pub(crate) mod for_loop;
pub(crate) mod keyword_call;
pub(crate) mod list_display;
pub(crate) mod slice;
pub(crate) mod subscript_call;
pub(crate) mod type_call;

#[cfg(test)]
mod binding_tests;
#[cfg(test)]
mod call_tests;
#[cfg(test)]
mod container_names_tests;
#[cfg(test)]
mod fallback_tests;
#[cfg(test)]
mod function_local_tests;
#[cfg(test)]
mod in_function_tests;
#[cfg(test)]
mod instance_arg_tests;
#[cfg(test)]
mod keyword_call_tests;
#[cfg(test)]
mod raise_tests;
#[cfg(test)]
mod subscript_call_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod type_call_tests;
