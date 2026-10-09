//! HIR-to-MIR expression lowering (#546): `lower_expr`, the per-`HirExpr`
//! dispatch that the crate root's lowering walk and every sibling lowering
//! module drive.

use super::class::{
    class_def_of, class_source_name, lower_isinstance, lower_issubclass, mro_attrs,
    rewrite_exception_to_message, rewrite_instance_to_repr, self_expr,
};
use super::{
    HirClassDef, InstantiateExpr, MirCompElt, MirComprehension, MirContainerReceiver, MirExpr,
    MirFStringPart, binop_result_ty, lookup, mro_class_def, try_lower_enum_member_attr,
};
use crate::receiver_exact::{exact_callee, receiver_class};
use pycc_hir::{
    BinOpKind, ClassAttrValue, CmpOpKind, CompElt, ContainerReceiver, FStringPart, HirExpr, Ty,
    UnaryOpKind, declares_name_outside_class_attrs,
};
use std::collections::HashMap;

mod foreign_static;
mod instance_hash;
mod receiver_dispatch;
mod regular_method;
mod sequence;
pub(crate) mod set_ops;

use instance_hash::lower_instance_hash;
use sequence::sequence_after;

/// D-154 (Part 1 of #375): lowers a construction of the class `callee`
/// (`class_def`) from its already-lowered `args` -- the shared tail of
/// `ClassName(args)` and of `type(self)(args)` (#1411), whose class is the
/// one `self` is typed as in the body being lowered.
fn lower_instantiation(
    callee: &str,
    class_def: &HirClassDef,
    args: Vec<MirExpr>,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
) -> MirExpr {
    // #432: resolve `__init__` via the MRO -- a derived class
    // without its own `__init__` inherits the base class's
    // constructor. The MRO is ordered most-derived-first.
    //
    // #966: but the *first* `__init__` is not always the right
    // one. D-225 puts an implicit zero-argument constructor in
    // the own method table of every class that declares none,
    // so for `class C(A, B)` with `A` init-less and `B`
    // declaring `__init__`, `A`'s stub would win and `B`'s
    // constructor would never run -- leaving `B`'s slots
    // uninitialized. CPython ranks the equivalent
    // (`object.__init__`) last, so skip flagged classes on a
    // first pass. The `or_else` pass covers the all-implicit
    // MRO (`class A: pass` / `class B: pass` / `class C(A, B)`),
    // where the implicit constructor really is the one to call;
    // only a genuinely `__init__`-less MRO reaches the panic.
    let ctor_in = |skip_implicit: bool| {
        class_def.mro.iter().find_map(|mro_class| {
            let mro_def = classes.get(mro_class.as_str())?;
            if skip_implicit && mro_def.implicit_object_init {
                return None;
            }
            if mro_def.methods.iter().any(|(mn, _)| mn == "__init__") {
                Some((mro_class, format!("{mro_class}.__init__")))
            } else {
                None
            }
        })
    };
    let (owner, ctor) = ctor_in(true).or_else(|| ctor_in(false)).unwrap_or_else(|| {
        panic!(
            "pycc_mir: internal error: no `__init__` found in class `{callee}`'s \
             MRO -- pycc_hir guarantees an `__init__` for every non-enum class it \
             lowers (D-225: by inheritance or by synthesis), and a call to an enum \
             class is C0001 before MIR lowering -- pycc_hir's per-item scan reports \
             it at the call (#944) with pycc_types' guard behind it (#921)"
        )
    });
    // #1337 (D-254): an inherited constructor runs its
    // receiver-exact copy when one exists.
    let ctor = exact_callee(callee, owner, ctor, scopes, classes);
    MirExpr::Instantiate(Box::new(InstantiateExpr {
        ctor,
        class_name: class_source_name(callee, classes),
        // #432: allocate slots for all unique attributes across the
        // MRO, not just this class's own declared attributes.
        slot_names: mro_attrs(class_def, classes)
            .into_iter()
            .map(|(name, _)| name)
            .collect(),
        args,
        ty: Ty::Instance(Box::new(callee.to_string())),
    }))
}

pub(super) fn lower_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    match expr {
        HirExpr::IntLiteral(n) => MirExpr::IntLiteral(*n),
        HirExpr::FloatLiteral(f) => MirExpr::FloatLiteral(*f),
        HirExpr::BoolLiteral(b) => MirExpr::BoolLiteral(*b),
        HirExpr::StringLiteral(s) => MirExpr::StringLiteral(s.clone()),
        HirExpr::NoneLiteral => MirExpr::NoneLiteral,
        HirExpr::NotImplemented => MirExpr::NotImplemented,
        // #1021: a `[]`/`{}` whose element type `pycc_types`'
        // empty-container pre-pass resolved. The carried `Ty` crosses
        // straight into MIR, because MIR has nothing to derive it from --
        // see `MirExpr::EmptyList`'s own doc comment.
        HirExpr::EmptyList(element) => MirExpr::EmptyList(element.clone()),
        HirExpr::EmptyDict(pair) => MirExpr::EmptyDict(pair.clone()),
        // D-136: `math.pi` (a `pycc_hir`-qualified stdlib constant name --
        // real Python identifiers never contain `.`, see
        // `pycc_types::std_qualified_symbol`'s own doc comment for the
        // same invariant) is never bound in `scopes` the way an ordinary
        // assigned variable is, so it needs its own arm here rather than
        // falling into the ordinary `lookup` below, which would panic.
        HirExpr::Name(name) if name == "math.pi" => MirExpr::Name {
            name: name.clone(),
            ty: Ty::Float,
        },
        // Issue #769 (Part 2 of #747): a name read inside a narrowing-
        // eligible branch (`super::narrowed_ty`'s `$narrowed:{name}`
        // sentinel, pushed by `stmt::lower_stmt`'s `HirStmt::If` arm) is
        // wrapped in `OptionalUnwrap` so `.ty()` reports the Optional's
        // inner type for this read alone, without touching the slot's own
        // still-`Optional` declared representation looked up via `lookup`
        // just below. Checked before the plain `Name` arm so a narrowed
        // read never falls through to it.
        //
        // #1476: over an `object` slot narrowed by an `isinstance` guard the
        // read is an `ObjectUnbox` instead (`crate::object_narrow`).
        HirExpr::Name(name) if super::narrowed_ty(scopes, name).is_some() => {
            let inner = super::narrowed_ty(scopes, name).expect("just matched Some above");
            super::object_narrow::narrowed_read(name, lookup(scopes, name), inner)
        }
        HirExpr::Name(name) => MirExpr::Name {
            name: name.clone(),
            ty: lookup(scopes, name),
        },
        // #1411: `type(self)(args)` constructs the class `self` is typed as
        // in this body. pycc dispatches statically (D-006), and an inherited
        // body that constructs through its receiver is lowered once more per
        // subclass with `self` retyped (D-254), so this is the receiver's
        // run-time class, as in CPython. Resolved straight to the class
        // rather than through the `Call` callee path, which a local of the
        // same name as the class could otherwise claim.
        HirExpr::ReceiverClassCall { args } => {
            let Ty::Instance(class) = lookup(scopes, "self") else {
                panic!(
                    "pycc_mir: internal error: `type(self)(...)` outside an instance method -- \
                     pycc_types refuses it unless `self` is an instance of a class"
                )
            };
            let class_def = classes.get(class.as_str()).unwrap_or_else(|| {
                panic!("pycc_mir: internal error: `self`'s class `{class}` is not a known class")
            });
            let args = args
                .iter()
                .map(|a| lower_expr(a, scopes, classes, current_class))
                .collect();
            lower_instantiation(&class, class_def, args, scopes, classes)
        }
        HirExpr::Call { callee, args } => {
            // #1313: a direct call of any name typed `Ty::Object`. The
            // admission is by type, not provenance -- the same rule
            // `ObjAttrGet`/`ObjMethodCall` follow -- so it covers both a
            // foreign binding and a `ForObject` loop target
            // (`for f in o.attr:` then `f(1)`). That is sound for both
            // provenances. For a foreign binding, the shadowing rule
            // (`crates/pycc_hir/src/import/shadow.rs`; see the eager foreign
            // bind in `build`, `lib.rs`) keeps any other top-level binding
            // off a foreign name, except the #1485 `except ImportError`
            // fallback (`docs/RUNTIME.md`), which binds a boxed `None` or
            // another foreign object, both `object`. For a loop target, `pycc_types`'
            // `HirStmt::ForObject` arm (`crates/pycc_types/src/lib.rs`)
            // refuses a target already bound to another type with `T0023`,
            // binds it `object` through `Environment::bind`, and downgrades
            // a newly introduced target to maybe-bound after the loop, so a
            // post-loop call is refused as `T0041` before this arm. Either
            // way a name typed `object` here is one the checker admitted as
            // such. Checked
            // first, mirroring `pycc_types`' callee-first gate, so no
            // builtin interception below can claim an object-typed name
            // that happens to spell one. The probe is non-panicking (`lookup`
            // panics on an unbound name, which every ordinary function
            // callee is), and the reverse scan finds a function-local that
            // spells a foreign name first, with its own non-`object` type.
            if scopes.iter().rev().find_map(|scope| scope.get(callee)) == Some(&Ty::Object) {
                return MirExpr::ObjCall {
                    callee: Box::new(lower_expr(
                        &HirExpr::Name(callee.clone()),
                        scopes,
                        classes,
                        current_class,
                    )),
                    args: args
                        .iter()
                        .map(|a| lower_expr(a, scopes, classes, current_class))
                        .collect(),
                };
            }
            // #435: `isinstance`/`issubclass` are compile-time-evaluated
            // builtins. They must be intercepted BEFORE the generic arg
            // lowering below, because the class arguments are class names
            // (not value expressions) and would fail to lower as ordinary
            // MIR expressions. The object argument (isinstance's args[0])
            // IS lowered normally to extract its type.
            // A user-defined function named `isinstance`/`issubclass` takes
            // priority over the builtin (same pattern as `float` — the type
            // checker's identical guard ensures a user-defined version is
            // never intercepted here, but the MIR guard is defense-in-depth
            // and mirrors the `float` builtin's own MIR-side check).
            let is_user_defined = scopes
                .iter()
                .any(|s| s.contains_key(&format!("$fn:{callee}")));
            if callee == "isinstance" && !is_user_defined {
                return lower_isinstance(args, scopes, classes, current_class);
            }
            if callee == "issubclass" && !is_user_defined {
                return lower_issubclass(args, classes);
            }
            // #767: `typing.cast(T, value)` is a runtime no-op — it only
            // changes a static checker's view of `value`'s type — so the
            // whole call expression lowers to `value` alone. Eliding it here
            // means no `MirExpr::Call` for `cast` ever reaches codegen, and
            // codegen needs no `cast` case at all. `args[0]` (the target
            // type) is a bare type name, not a value expression, and is
            // deliberately never lowered.
            //
            // Indexing `args[1]` is sound because `pycc_types::class::
            // check_cast` rejects every arity other than 2 with `T0021`, and
            // both `pycc_mir::build` call sites lower an already-type-checked
            // HIR module — the same "the type checker guarantees this shape"
            // invariant the `__init__`-in-MRO panic below relies on. A
            // user-defined `def cast(...)` takes priority over the special
            // case, exactly as for `isinstance`/`issubclass`.
            if callee == "cast" && !is_user_defined {
                return lower_expr(&args[1], scopes, classes, current_class);
            }
            let args: Vec<MirExpr> = args
                .iter()
                .map(|a| {
                    let lowered = lower_expr(a, scopes, classes, current_class);
                    // #378 (PR-18): for `print(instance)`, rewrite an
                    // instance-typed argument to a `__repr__` call so the
                    // codegen's `to_str` receives a `str` scalar. Only
                    // `print` feeds non-str values to `to_str` among call
                    // expressions (f-string interpolations are handled in
                    // the `FString` arm); other calls pass instance
                    // arguments directly as opaque pointers.
                    if callee == "print" {
                        // Part 3A of #541 (#736): try the exception-message
                        // rewrite before the dataclass `__repr__` one, so a
                        // caught exception binding is rendered as its
                        // message rather than falling through to
                        // `rewrite_instance_to_repr`. The ordering matters:
                        // a user `@dataclass` declared under a builtin
                        // exception name *would* satisfy both rewrites, and
                        // this name-first order is why `pycc_types`'s
                        // string-conversion gate (#977, D-237) rejects that
                        // shape before lowering instead of letting the
                        // dataclass `__repr__` run on an exception object.
                        let lowered = rewrite_exception_to_message(&lowered, classes);
                        rewrite_instance_to_repr(&lowered, classes)
                    } else {
                        lowered
                    }
                })
                .collect();
            // Part 3 of #1026 (PR 3a of #1082): `len(o)` on a CPython object
            // becomes its own node rather than the `Call { callee: "len" }`
            // below. The two are not interchangeable at codegen -- the
            // scalar path yields a raw `i64` the caller re-tags, while the
            // shim hands back an already-encoded word and can fail -- so the
            // split happens here, where the argument's type is still known.
            // See `MirExpr::ObjLen`'s own doc comment.
            //
            // There is deliberately no `$fn:len` shadow check here.
            // `pycc_types::check` resolves `len` as the reserved builtin
            // whatever the module defines -- exactly as the scalar `len`
            // arm below records ("a hand-recognized builtin, not a
            // user-declarable `$fn:` signature", D-105 point 3) -- so a
            // module-level `def len` never changes which lowering a `len`
            // call takes, and the user's function is simply not called. A
            // shadow guard here would not honour the shadow; it would only
            // desynchronise MIR from the checker, letting `len(<object>)`
            // pass `pycc check` and then reach codegen as an ordinary
            // `Call` whose `Scalar::Object` argument trips
            // `expect_list_pointer`'s internal-error assertion. The
            // checker/CPython divergence itself -- that shadowing `len` is
            // silently ignored rather than refused -- is filed as #1098 and
            // is not this lowering's to fix.
            if callee == "len" && args.len() == 1 && matches!(args[0].ty(), Ty::Object) {
                return MirExpr::ObjLen {
                    base: Box::new(args.into_iter().next().expect("checked len() == 1")),
                };
            }
            // #1116: `len(b)` on a `pycc build --ext` export's `memoryview`
            // parameter becomes its own node too, for the same reason and
            // with the same shadow reasoning as the `ObjLen` split directly
            // above -- the scalar `len` arm below reaches
            // `expect_list_pointer`, and a `Scalar::MemoryView` is not a
            // `PyIntListObj`. See `MirExpr::BufferLen`'s own doc comment.
            if callee == "len" && args.len() == 1 && matches!(args[0].ty(), Ty::MemoryView) {
                return MirExpr::BufferLen {
                    base: Box::new(args.into_iter().next().expect("checked len() == 1")),
                };
            }
            // D-154 (Part 1 of #375): `ClassName(args)` (instantiation)
            // reuses `HirExpr::Call` -- there is no dedicated HIR shape for
            // it (`pycc_hir::class`'s own doc comment) -- so it is resolved
            // here, before the ordinary `$fn:` lookup below (which has no
            // entry for a bare class name; only its mangled methods are
            // registered). See `MirExpr::Instantiate`'s own doc comment for
            // why this needs a dedicated MIR node rather than folding into
            // `MirExpr::Call` the way `MethodCall` below does.
            if let Some(class_def) = classes.get(callee.as_str()) {
                return lower_instantiation(callee, class_def, args, scopes, classes);
            }
            // #1165 (Part 2a of #1142): `ndarray(n)` / `NDArray(n)`
            // produces artifact-owned buffer storage. Placed *after* the
            // class-instantiation lookup directly above and under the same
            // `$fn:` shadow guard `float`/`bool`/`int`/`str` carry below,
            // which together deliver D-244 #1129 statement (h) -- a
            // program's own `class ndarray` or `def NDArray` keeps its own
            // meaning, in call position exactly as in annotation position.
            // `pycc_types`' own interception mirrors this ordering, inside
            // the `env.lookup_function(callee)` fall-through.
            //
            // Deliberately not a `MirExpr::Call`: see `MirExpr::BufferAlloc`'s
            // own doc comment for why routing the producer through the
            // ordinary call path would take a `Ty::MemoryView` into
            // `pycc_codegen::call_result`'s panic.
            //
            // Only the one-argument shape is claimed, matching the checker,
            // which refuses every other arity before MIR ever runs.
            if (callee == "ndarray" || callee == "NDArray")
                && args.len() == 1
                && !scopes
                    .iter()
                    .any(|scope| scope.contains_key(&format!("$fn:{callee}")))
            {
                return MirExpr::BufferAlloc {
                    len: Box::new(args.into_iter().next().expect("checked len() == 1")),
                };
            }
            // Part 1 of #1319: the `frozenset(...)` builtin. Placed after the
            // class-instantiation lookup above and under the `$fn:` shadow
            // guard, like `ndarray` directly above, so a program's own
            // `class frozenset` or `def frozenset` keeps its meaning --
            // mirroring `pycc_types`' own interception, which sits after its
            // class and user-function lookups. `pycc_types` admits at most
            // one argument.
            if callee == "frozenset"
                && !scopes
                    .iter()
                    .any(|scope| scope.contains_key(&format!("$fn:{callee}")))
            {
                return MirExpr::FrozenSetFrom {
                    source: args.into_iter().next().map(Box::new),
                };
            }
            // #1335 (Part 1 of #1332): `hash(instance)`, under the same
            // shadow guard as `frozenset` above. `pycc_types` admitted the
            // class only when `resolve_instance_hash` gives a lowerable
            // verdict, from the same class table. The `hash` branch of the
            // type chain below still types every other argument.
            if callee == "hash"
                && let [instance] = args.as_slice()
                && let Ty::Instance(class) = instance.ty()
                && !scopes
                    .iter()
                    .any(|scope| scope.contains_key(&format!("$fn:{callee}")))
            {
                let instance = instance.clone();
                return lower_instance_hash(instance, &class, scopes, classes);
            }
            // Part 11 of #1371: `type(o)` on a CPython object, under the
            // same shadow guard as `hash` above; a user `class type` was
            // already claimed by the instantiation lookup. `pycc_types`
            // admits only this one-object-argument shape
            // (`foreign::type_call`). A narrowed `object` read is the object
            // itself (#1476): CPython reports its own class, a subclass of
            // the guard's type included, so the native unbox is dropped.
            if callee == "type"
                && let [base] = args.as_slice()
                && let base = super::object_narrow::object_operand(base.clone())
                && base.ty() == Ty::Object
                && !scopes
                    .iter()
                    .any(|scope| scope.contains_key(&format!("$fn:{callee}")))
            {
                return MirExpr::ObjType {
                    base: Box::new(base),
                };
            }
            let ty = if callee == "print" {
                Ty::None
            } else if callee == "math.sqrt" {
                // D-136: `math.sqrt` is the other hand-recognized stdlib
                // intrinsic this PR lowers to real codegen -- mirrors
                // `pycc_types`'s own `std_qualified_symbol` dispatch
                // (always `Ty::Float`, this registry's only lowered
                // function's fixed return type; not a general per-registry
                // lookup since `pycc_mir` has no dependency on `pycc_std`
                // and does not need one for this PR's exactly-one-function
                // registry).
                Ty::Float
            } else if callee == "len" {
                // `len` is a hand-recognized builtin, same as `print` above,
                // not a user-declarable `$fn:` signature -- mirrors
                // `pycc_types::collect_expr_constraints`'s own `callee ==
                // "len"` arm (D-105 point 3). Without this branch, `len(lst)`
                // falls to the `lookup` fallback below, finds no registered
                // `$fn:len`, and panics even though `pycc_types` already
                // accepts `len(lst)` as valid, `Ty::Int`-typed.
                Ty::Int
            } else if callee == "float"
                && !scopes
                    .iter()
                    .any(|scope| scope.contains_key(&format!("$fn:{callee}")))
            {
                // Mirrors `pycc_types`' own `callee == "float"` arms (both the
                // public-body and private-helper paths), including their own
                // user-defined-function-takes-priority guard -- see
                // `pycc_types::infer_expr_in`'s comment for why `float` (unlike
                // `len`/`print`) needs this. Always `Ty::Float`.
                Ty::Float
            } else if callee == "hash"
                && !scopes
                    .iter()
                    .any(|scope| scope.contains_key(&format!("$fn:{callee}")))
            {
                // #1331: mirrors `pycc_types::hash`'s arms under `float`'s
                // user-defined-function guard. A user `class hash` never
                // reaches here: the `Instantiate` lookup above claims it.
                // Always `Ty::Int`.
                Ty::Int
            } else if callee == "bool"
                && !scopes
                    .iter()
                    .any(|scope| scope.contains_key(&format!("$fn:{callee}")))
            {
                // Part 4 of #1026 (PR 4a of #1083). `pycc_types` admits the
                // `bool` builtin for a `Ty::Object` argument only, under the
                // same user-defined-function-takes-priority guard `float`
                // carries immediately above, so this arm mirrors that guard
                // exactly rather than `len`'s unguarded one -- a user
                // `def bool(...)` really does win in `pycc_types`, and a
                // mirror that disagreed would lower a user call to the
                // builtin. Always `Ty::Bool`.
                //
                // Without this branch `bool(o)` falls to the `lookup` below,
                // finds no `$fn:bool`, and panics -- a `pycc check` that exits
                // 0 followed by a `pycc build --ext` that aborts.
                Ty::Bool
            } else if (callee == "int" || callee == "str")
                && !scopes
                    .iter()
                    .any(|scope| scope.contains_key(&format!("$fn:{callee}")))
            {
                // Part 4 of #1026 (PR 4b of #1083), the `bool` arm above
                // repeated for the other two conversions and for its reasons:
                // `pycc_types` admits each for a `Ty::Object` argument only,
                // under the same user-defined-function-takes-priority guard, so
                // the mirror carries that guard rather than `len`'s unguarded
                // one -- a user `def int(...)`/`def str(...)` really does win in
                // `pycc_types`, and a mirror that disagreed would lower a user
                // call to the builtin. Always `Ty::Int`/`Ty::Str` respectively,
                // since the argument type is fixed.
                //
                // Without this branch `int(o)` falls to the `lookup` below,
                // finds no `$fn:int`, and panics.
                if callee == "int" { Ty::Int } else { Ty::Str }
            } else {
                lookup(scopes, &format!("$fn:{callee}"))
            };
            MirExpr::Call {
                callee: callee.clone(),
                args,
                ty,
            }
        }
        // #603 (Part 2 of #573): `-x`/`+x` over a non-literal operand is
        // rewritten into the equivalent binary expression rather than
        // getting a `MirExpr` variant of its own -- pycc's MIR deliberately
        // has no `USub`/`UAdd`/`Invert` unary node (see the #604 comment
        // below for `not x`'s own dedicated `MirExpr::Not` exception to
        // that rule, which also supersedes the "negated `__eq__` call"
        // comparison this comment used to draw to the dataclass `!=`
        // lowering further below -- that lowering now itself uses
        // `MirExpr::Not` rather than illustrating "no unary node exists").
        //
        // The rewrite is representation-sensitive, so it is not a single
        // uniform shape:
        //
        // * `int`/`bool` become `0 - x` / `0 + x`. `pycc_rt`'s `int_sub`
        //   and `int_add` both handle an already-promoted bigint operand
        //   and the smallint boundary, so negation inherits arbitrary
        //   precision for free. `x * -1` would *not*: `int_mul` decodes
        //   its operands inline and raises `OverflowError` on a bigint
        //   (Part C of #1038).
        // * `float` becomes `x * -1.0` / `x * 1.0`, a plain LLVM `fmul`.
        //   `0.0 - x` would be wrong for `-0.0` (it yields `+0.0`), and
        //   multiplication is exact for infinities and NaN too.
        //
        // `UAdd` is not folded away, because it is not the identity: on a
        // `bool` operand it crosses into `int` (`+True` is `1`), which the
        // `0 + x` / `x * 1.0` shapes deliver through the same typing rules
        // as `USub`. Any non-numeric operand is already rejected by
        // `pycc_types::unop::unary_result_type` and never reaches here.
        //
        // #604 (Part 3 of #573) adds `not x` and `~x`. `~x` follows the
        // exact same "rewrite into a `BinOp`, no new MIR node" strategy as
        // `USub`/`UAdd`: `~x == -x - 1` for every `int`/`bool` operand (the
        // only operands `pycc_types::unop::unary_result_type` accepts for
        // `Invert`), so it reuses the identical `0 - x` shape and its
        // bigint-precision guarantee, then subtracts a further literal
        // `1` -- both legs going through `int_sub`. `not x` genuinely has
        // no binary-expression equivalent (its truthiness rule spans
        // types `BinOp`'s own typing has no notion of), so it is the one
        // case that *does* need a dedicated `MirExpr::Not` node, built
        // directly on `pycc_codegen`'s existing `truthy` helper (the same
        // one `if`/`while` conditions already call).
        HirExpr::UnaryOp { op, operand } => {
            let operand = lower_expr(operand, scopes, classes, current_class);
            match op {
                UnaryOpKind::USub | UnaryOpKind::UAdd => {
                    let (bin_op, left, right) = if operand.ty() == Ty::Float {
                        let factor = if matches!(op, UnaryOpKind::USub) {
                            -1.0
                        } else {
                            1.0
                        };
                        (BinOpKind::Mul, operand, MirExpr::FloatLiteral(factor))
                    } else {
                        let bin_op = if matches!(op, UnaryOpKind::USub) {
                            BinOpKind::Sub
                        } else {
                            BinOpKind::Add
                        };
                        (bin_op, MirExpr::IntLiteral(0), operand)
                    };
                    let ty = binop_result_ty(bin_op, left.ty(), right.ty());
                    MirExpr::BinOp {
                        op: bin_op,
                        left: Box::new(left),
                        right: Box::new(right),
                        ty,
                    }
                }
                UnaryOpKind::Invert => {
                    let negated_ty = binop_result_ty(BinOpKind::Sub, Ty::Int, operand.ty());
                    let negated = MirExpr::BinOp {
                        op: BinOpKind::Sub,
                        left: Box::new(MirExpr::IntLiteral(0)),
                        right: Box::new(operand),
                        ty: negated_ty.clone(),
                    };
                    let ty = binop_result_ty(BinOpKind::Sub, negated_ty, Ty::Int);
                    MirExpr::BinOp {
                        op: BinOpKind::Sub,
                        left: Box::new(negated),
                        right: Box::new(MirExpr::IntLiteral(1)),
                        ty,
                    }
                }
                UnaryOpKind::Not => MirExpr::Not(Box::new(operand)),
            }
        }
        HirExpr::BoolOp {
            op,
            left,
            right,
            truth_only,
        } => super::boolop::lower_bool_op(
            *op,
            lower_expr(left, scopes, classes, current_class),
            lower_expr(right, scopes, classes, current_class),
            *truth_only,
        ),
        // Part 1 of #891: a native tuple's arity was checked statically by
        // `pycc_types`, so the tuple itself is the unpacked value; a CPython
        // object is unpacked at run time by `MirExpr::ObjUnpack`.
        HirExpr::Unpack { value, arity } => {
            let value = lower_expr(value, scopes, classes, current_class);
            if value.ty() == Ty::Object {
                MirExpr::ObjUnpack {
                    value: Box::new(value),
                    arity: *arity,
                }
            } else {
                value
            }
        }
        HirExpr::IfExp { test, body, orelse } => super::if_exp::lower_if_exp(
            lower_expr(test, scopes, classes, current_class),
            lower_expr(body, scopes, classes, current_class),
            lower_expr(orelse, scopes, classes, current_class),
        ),
        HirExpr::BinOp { op, left, right } => {
            let left = lower_expr(left, scopes, classes, current_class);
            let right = lower_expr(right, scopes, classes, current_class);
            let ty = binop_result_ty(*op, left.ty(), right.ty());
            MirExpr::BinOp {
                op: *op,
                left: Box::new(left),
                right: Box::new(right),
                ty,
            }
        }
        HirExpr::Compare { op, left, right } => {
            let mut left_lowered = lower_expr(left, scopes, classes, current_class);
            let mut right_lowered = lower_expr(right, scopes, classes, current_class);
            // #1476: an identity test compares the objects themselves, never
            // a narrowed native copy, so both narrowed operands are read raw.
            if matches!(op, CmpOpKind::Is | CmpOpKind::IsNot) {
                left_lowered = super::object_narrow::object_operand(left_lowered);
                right_lowered = super::object_narrow::object_operand(right_lowered);
            }
            // Part 1 of #1371: an object operand takes CPython's own
            // comparison, never the native one below.
            let (left_lowered, right_lowered) =
                match super::obj_compare::lower_object_compare(*op, left_lowered, right_lowered) {
                    Ok(object_compare) => return object_compare,
                    Err(operands) => *operands,
                };
            // #378 (PR-18): `==`/`!=` between same-class dataclass instances
            // is rewritten to a `MirExpr::Call` to the class's
            // compiler-synthesized `__eq__` method. `!=` is lowered as
            // `__eq__(left, right) != True` (a `MirExpr::Compare` against a
            // `BoolLiteral`, predating #604's `MirExpr::Not`) rather than
            // negating the call result with `MirExpr::Not` directly --
            // `MirExpr::Not` computes truthiness (bool/int/float/str/
            // None/Optional) via `pycc_codegen`'s `truthy` helper, but this
            // call's result is already a `bool`, so an equality compare
            // against `true` is the simpler, equally-correct rewrite. This
            // mirrors how `@property` redirects attribute access to method
            // calls -- a MIR-level rewrite, not a new MIR node. Only `Eq`
            // and `NotEq` are rewritten, and only for dataclass classes
            // (whose synthesized `__eq__` has a known-correct signature);
            // other comparison operators (`<`, `<=`, `>`, `>=`) and
            // non-dataclass classes
            // fall through to the default `MirExpr::Compare` (the type
            // checker rejects them with `T0021` before they reach MIR
            // lowering in normal compilation, but the MIR itself stays
            // semantically correct for defense-in-depth).
            if matches!(op, pycc_hir::CmpOpKind::Eq | pycc_hir::CmpOpKind::NotEq)
                && let Some(eq_mangled) = super::compare_chain::dataclass_eq_callee(
                    &left_lowered.ty(),
                    &right_lowered.ty(),
                    classes,
                )
            {
                let eq_call = MirExpr::Call {
                    callee: eq_mangled,
                    args: vec![left_lowered, right_lowered],
                    ty: Ty::Bool,
                };
                // The `matches!` guard above limits entry to
                // `Eq`/`NotEq`, so an if/else (rather than a match
                // with a `_` arm) suffices and avoids an unreachable
                // arm that would be permanently uncovered under
                // D-014's 100% coverage gate.
                if *op == pycc_hir::CmpOpKind::Eq {
                    return eq_call;
                }
                return MirExpr::Compare {
                    op: pycc_hir::CmpOpKind::NotEq,
                    left: Box::new(eq_call),
                    right: Box::new(MirExpr::BoolLiteral(true)),
                    ty: Ty::Bool,
                };
            }
            MirExpr::Compare {
                op: *op,
                left: Box::new(left_lowered),
                right: Box::new(right_lowered),
                ty: Ty::Bool,
            }
        }
        // #1212: every operand is lowered once, left to right; a
        // same-dataclass `==`/`!=` link keeps both operands as values and
        // names the synthesized `__eq__` instead of owning them in a `Call`.
        HirExpr::CompareChain { first, links } => super::compare_chain::lower_compare_chain(
            lower_expr(first, scopes, classes, current_class),
            links
                .iter()
                .map(|link| {
                    (
                        link.op,
                        lower_expr(&link.right, scopes, classes, current_class),
                    )
                })
                .collect(),
            classes,
        ),
        HirExpr::FString(parts) => MirExpr::FString(
            parts
                .iter()
                .map(|p| match p {
                    FStringPart::Literal(s) => MirFStringPart::Literal(s.clone()),
                    FStringPart::Interpolation(e) => {
                        let lowered = lower_expr(e, scopes, classes, current_class);
                        // #378 (PR-18): if the interpolation is a class
                        // instance with `__repr__`, rewrite it to a call to
                        // `__repr__` so the codegen's `to_str` receives a
                        // `str` scalar, not an Instance scalar (which would
                        // panic). This mirrors how `==`/`!=` on instances is
                        // rewritten to `__eq__` calls in the Compare arm.
                        // Part 3A of #541 (#736): try the exception-message
                        // rewrite first -- see the identical `print`-argument
                        // ordering comment above for why.
                        let lowered = rewrite_exception_to_message(&lowered, classes);
                        let rewritten = rewrite_instance_to_repr(&lowered, classes);
                        MirFStringPart::Interpolation(Box::new(rewritten))
                    }
                })
                .collect(),
        ),
        // Part 2d of #1371: a list display the empty-container pre-pass
        // resolved to an object slot.
        HirExpr::ObjectList(elements) => MirExpr::ObjList {
            elements: elements
                .iter()
                .map(|e| lower_expr(e, scopes, classes, current_class))
                .collect(),
        },
        HirExpr::ListLiteral(elements) => MirExpr::ListLiteral(
            elements
                .iter()
                .map(|e| lower_expr(e, scopes, classes, current_class))
                .collect(),
        ),
        // `HirExpr::Subscript` is reused unconditionally by `pycc_hir`'s own
        // lowering for both a list read and a dict read (it has no type
        // information to pick a different node) -- `pycc_types::infer_expr_in`
        // accepts either base equally (see its own `Subscript` arm), so this
        // is the point where the real type is resolved and where a
        // dict-typed base is routed into `MirExpr::DictGet` instead of
        // `MirExpr::Subscript`, mirroring `lower_stmt`'s own `HirStmt::ForList`
        // arm doing the same list/dict routing for iteration.
        HirExpr::Subscript { base, index } => {
            // PEP 560 (#610): `C[x]` on a bare class name is
            // `C.__class_getitem__(x)`. `pycc_types`' own `Subscript` arm
            // has already resolved the hook through the MRO and rejected a
            // class that does not define it, so reaching this point with a
            // class-name base means the call is valid. The guard is the same
            // one that arm applies -- a name bound as a value shadows the
            // class, and `C[0]` then reads that value's element.
            //
            // Lowering is delegated to this function's own `MethodCall` arm
            // by way of a synthetic node rather than duplicated here: that
            // arm already handles both the `@staticmethod` and the
            // `@classmethod` spelling of the hook (the latter needs a
            // `NullInstance` receiver prepended), and re-deriving either
            // shape here would be a second copy of logic that must not
            // drift. `lower_expr` on a bare class name would panic (class
            // names are not in scope), which is why the interception happens
            // before the base is lowered.
            if let HirExpr::Name(class_name) = base.as_ref()
                && !scopes.iter().any(|scope| scope.contains_key(class_name))
                && classes.contains_key(class_name.as_str())
            {
                return lower_expr(
                    &HirExpr::MethodCall {
                        base: base.clone(),
                        method: "__class_getitem__".to_string(),
                        args: vec![(**index).clone()],
                    },
                    scopes,
                    classes,
                    current_class,
                );
            }
            let base = lower_expr(base, scopes, classes, current_class);
            let index = lower_expr(index, scopes, classes, current_class);
            match base.ty() {
                // Part 3 of #1026 (PR 3b of #1082): a foreign CPython
                // object base routes to its own node, in exactly the
                // position `Ty::Dict` occupies and for exactly the same
                // reason -- `MirExpr::Subscript`'s own `ty()` arm panics on
                // any base that is neither list nor tuple, so the split has
                // to happen here, where the lowered base's type is known.
                // The dispatch is on `base.ty()` and nothing else: a
                // foreign name is always in scope, so the PEP 560
                // class-name interception above cannot fire for one, and an
                // extra guard would only desynchronise this lowering from
                // the checker (the lesson #1098 records for `len`).
                Ty::Object => MirExpr::ObjSubscript {
                    base: Box::new(base),
                    index: Box::new(index),
                },
                Ty::Dict(_) => MirExpr::DictGet {
                    dict: Box::new(base),
                    key: Box::new(index),
                },
                // Part 2 of #1027, in that same position and for that same
                // reason: without an arm of its own a `memoryview` base
                // reaches the catch-all below and `MirExpr::Subscript`'s
                // `ty()` panics on it. The dispatch is on `base.ty()` alone
                // here too -- the type checker has already refused every
                // `memoryview` subscript this arm must not see.
                Ty::MemoryView => MirExpr::BufferGet {
                    base: Box::new(base),
                    index: Box::new(index),
                },
                _ => MirExpr::Subscript {
                    base: Box::new(base),
                    index: Box::new(index),
                },
            }
        }
        HirExpr::ListAppend { list, value } => MirExpr::ListAppend {
            list: lower_container_receiver(list, scopes, classes, current_class).0,
            value: Box::new(lower_expr(value, scopes, classes, current_class)),
        },
        HirExpr::DictLiteral(pairs) => MirExpr::DictLiteral(
            pairs
                .iter()
                .map(|(k, v)| {
                    (
                        lower_expr(k, scopes, classes, current_class),
                        lower_expr(v, scopes, classes, current_class),
                    )
                })
                .collect(),
        ),
        HirExpr::SetLiteral(elements) => {
            let elements: Vec<MirExpr> = elements
                .iter()
                .map(|e| lower_expr(e, scopes, classes, current_class))
                .collect();
            // #1343: the first element's type is the literal's, as
            // `MirExpr::ty` derives it.
            let ops = elements
                .first()
                .and_then(|first| set_ops::lower_set_element_ops(&first.ty(), scopes, classes))
                .map(Box::new);
            MirExpr::SetLiteral { elements, ops }
        }
        HirExpr::TupleLiteral(elements) => MirExpr::TupleLiteral(
            elements
                .iter()
                .map(|e| lower_expr(e, scopes, classes, current_class))
                .collect(),
        ),
        // PR-12 Task 8 (D-118): purely structural -- recurse into `base` and
        // every present bound, same as every other MIR-lowering site in this
        // function. `pycc_types` already validated the base is
        // `Ty::List(Int)` and every present bound is `int`-assignable
        // (Task 7); this lowering does not re-validate or apply the runtime
        // clamping/panic behavior D-118 requires -- that is
        // `pycc_codegen`'s job, operating on this already-lowered shape.
        //
        // Part 2b of #1371: a CPython object base is split into its own
        // `MirExpr::ObjSlice`, because `MirExpr::Slice::ty()` answers its
        // base's type and codegen's `Slice` arm is list/str/tuple-only.
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            let base = Box::new(lower_expr(base, scopes, classes, current_class));
            let lower_bound = |bound: &Option<Box<HirExpr>>| {
                bound
                    .as_deref()
                    .map(|e| Box::new(lower_expr(e, scopes, classes, current_class)))
            };
            let (start, stop, step) = (lower_bound(start), lower_bound(stop), lower_bound(step));
            if base.ty() == Ty::Object {
                MirExpr::ObjSlice {
                    base,
                    start,
                    stop,
                    step,
                }
            } else {
                MirExpr::Slice {
                    base,
                    start,
                    stop,
                    step,
                }
            }
        }
        // PR-12 Task 11 (D-119): `list`'s element type is resolved via the
        // same `lookup` mechanism every other name reference in this crate
        // uses, mirroring `HirExpr::Subscript`'s own base-type lookup above.
        // #1263: an attribute receiver's type is its lowered read's own.
        HirExpr::ListPop { list } => {
            let (list, list_ty) = lower_container_receiver(list, scopes, classes, current_class);
            let Ty::List(elem_ty) = list_ty else {
                panic!(
                    "pycc_mir: internal error: `.pop()` receiver {list:?} is not list-typed -- pycc_types::check should have rejected this HIR before it reached pycc_mir"
                )
            };
            MirExpr::ListPop { list, ty: *elem_ty }
        }
        HirExpr::DictGetOrDefault { dict, key, default } => {
            let (dict, dict_ty) = lower_container_receiver(dict, scopes, classes, current_class);
            let Ty::Dict(kv) = dict_ty else {
                panic!(
                    "pycc_mir: internal error: `.get()` receiver {dict:?} is not dict-typed -- pycc_types::check should have rejected this HIR before it reached pycc_mir"
                )
            };
            MirExpr::DictGetOrDefault {
                dict,
                key: Box::new(lower_expr(key, scopes, classes, current_class)),
                default: Box::new(lower_expr(default, scopes, classes, current_class)),
                ty: kv.1,
            }
        }
        HirExpr::SetAdd { set, value } => {
            let Ty::Set(elem_ty) = lookup(scopes, set) else {
                panic!("pycc_types admits `.add()` only on a set")
            };
            MirExpr::SetAdd {
                set: set.clone(),
                value: Box::new(lower_expr(value, scopes, classes, current_class)),
                ops: set_ops::lower_set_element_ops(&elem_ty, scopes, classes).map(Box::new),
            }
        }
        // D-154 (Part 1 of #375): `base.attr` -- resolved to a compile-time
        // slot index against the base's class's `HirClassDef`, per the
        // class-instance-layout ADR (never a runtime string-keyed lookup).
        // #377: if `attr` is a `@property`, the access is rewritten to an
        // ordinary `MirExpr::Call` to the getter's mangled name (with `base`
        // as `self`), reusing the existing method-call/codegen infrastructure
        // with no new MIR/codegen variant.
        // #432: property and attribute resolution walk the MRO, so an
        // attribute or property declared in a base class is found when
        // accessed on a derived class instance. The slot index is computed
        // from the MRO's flat attribute layout (`mro_attrs`).
        HirExpr::AttrGet { base, attr } => {
            // #433: `super().attr` — resolve the attribute starting from
            // the next class in the current class's MRO, using `self` (the
            // current function's first parameter) as the instance.
            // #587: only a `@property` resolves this way. A `super` object
            // proxies class-level attributes and descriptors, not the
            // instance's own attributes, so `super().<instance attr>` no
            // longer lowers to a slot read against `self` — the type
            // checker rejects it with `T0047` first.
            if matches!(base.as_ref(), HirExpr::Super) {
                let current = match current_class {
                    Some(c) => c,
                    None => panic!(
                        "pycc_mir: internal error: `HirExpr::Super` reached lower_expr outside a \
                         method body -- pycc_hir::lower_expr should have rejected this with C0001"
                    ),
                };
                let self_expr = self_expr(scopes);
                // #1337 (D-254): the MRO `super()` continues is the
                // *receiver's* -- a copy's `self` is typed as the subclass it
                // was compiled for -- after the body's own class, as in
                // CPython.
                let self_ty = self_expr.ty();
                let receiver = receiver_class(&self_ty).unwrap_or(current);
                let class_def = &classes[receiver];
                let current_pos = class_def
                    .mro
                    .iter()
                    .position(|c| c == current)
                    .expect("pycc_mir: internal error: class not found in its receiver's MRO");
                let super_mro = &class_def.mro[current_pos + 1..];
                // #915: one pass over the slice, checking every class-level
                // member kind on each class before moving to the next --
                // mirroring `pycc_types::class::resolve_super_attr_get`,
                // where the reasoning lives: a `super` object resolves
                // against one class `__dict__` at a time, so MRO position
                // decides, not member kind.
                //
                // A class attribute (#911) folds to its literal through the
                // same `fold_class_attr` helper the `Base.X`, `self.X` and
                // `d.X` sites use, so `super().X` produces MIR identical to
                // `Base.X` and `pycc_codegen` needs no change. The
                // `self_expr` receiver computed above is deliberately
                // discarded on that path: a class attribute has no runtime
                // storage, so the read needs no instance, and `super()` is
                // side-effect-free, so dropping it loses nothing.
                for mro_class in super_mro {
                    let mro_def = &classes[mro_class.as_str()];
                    if let Some(prop) = mro_def.properties.iter().find(|p| p.name == *attr) {
                        let getter =
                            exact_callee(receiver, mro_class, prop.getter.clone(), scopes, classes);
                        let ty = lookup(scopes, &format!("$fn:{getter}"));
                        return MirExpr::Call {
                            callee: getter,
                            args: vec![self_expr],
                            ty,
                        };
                    }
                    if let Some(folded) = fold_class_attr(mro_def, attr) {
                        return folded;
                    }
                }
                // #587: nothing else resolves through `super()`. A `super`
                // object proxies class-level attributes and descriptors
                // along the MRO, not the instance's own attributes, so an
                // `AttrGet` naming an instance attribute (or a name declared
                // nowhere in the MRO at all) is rejected by
                // `pycc_types::class::resolve_super_attr_get` with `T0047`
                // or `T0044` before any HIR reaches this crate. Properties
                // and class attributes are the only class-level members pycc
                // models today, so the two loops above are exhaustive for
                // well-typed input.
                panic!(
                    "pycc_mir: internal error: `super().{attr}` is not a property or class \
                     attribute on any class after `{current}` in its MRO -- pycc_types::check \
                     should have rejected this HIR with T0047 or T0044 before it reached pycc_mir"
                );
            }
            // #379 (PR-19): `Color.RED` — accessing an enum member by name
            // on the enum class. The base is `HirExpr::Name` referring to
            // an enum class (a class with non-empty `enum_members`), and
            // `attr` is a member name. Lower to `MirExpr::Name` reading a
            // synthetic global (`<Class>.<Member>.enum_member`) that
            // codegen initializes once at module-init time with the
            // member's singleton instance. The `.enum_member` suffix
            // ensures the name cannot collide with a real Python
            // identifier (which cannot contain `.`). The subsequent
            // `.value`/`.name` read on this result is a separate
            // `AttrGet` that resolves to a slot via the enum class's
            // `attrs = [("value", Int), ("name", Str)]` table.
            // A value binding of the enum class's own name shadows the
            // class here exactly as it does for the class-attribute fold
            // below, so this interception is guarded the same way -- without
            // it `def f(Color: D) -> int: return Color.RED` would read the
            // enum member's singleton instead of the parameter's slot.
            if !matches!(base.as_ref(), HirExpr::Name(name)
                if scopes.iter().any(|scope| scope.contains_key(name)))
                && let Some(enum_member_expr) =
                    try_lower_enum_member_attr(base.as_ref(), attr.as_str(), classes)
            {
                return enum_member_expr;
            }
            // #911 (Part 1 of #885): `W.MIN_WIDTH` -- a class-level attribute
            // read through the class name. Intercepted before `lower_expr`
            // for the same reason the enum-member read above is: the base is
            // a class name, not a value binding, so lowering it as an
            // expression would fail.
            //
            // #974: the walk runs over `class_def.mro`, most-derived first,
            // so an *inherited* class attribute (`Derived.LIMIT`) folds too.
            // It stops at the first class that declares `attr` in any other
            // class-level namespace, because CPython binds that declaration
            // instead -- `declares_name_outside_class_attrs` is the shared
            // predicate, and `pycc_types::class::lookup_class_attr_by_class_name`
            // runs the identical walk over it so the type the checker
            // reported and the constant folded here can never disagree (the
            // #960 failure mode). This is deliberately *not* the instance
            // arm's walk further below: that one consults instance slots
            // first, and a class object has no instance `__dict__`.
            // An active value binding of that name shadows the class, so
            // `B.X` reads the parameter's instance slot rather than the class
            // attribute -- `pycc_types`' own class-name `AttrGet` arm applies
            // the matching binding/local guard, and this scope check is the
            // MIR-side spelling of it (the same one the `__class_getitem__`
            // interception above already uses). Without it the shadowed name
            // would reach the internal-error panic below.
            if let HirExpr::Name(class_name) = base.as_ref()
                && !scopes.iter().any(|scope| scope.contains_key(class_name))
                && let Some(class_def) = classes.get(class_name.as_str())
            {
                // Part 1 of #1284: `C.name` where the winning class-level
                // binding is `name = staticmethod(<foreign callable>)` reads
                // the foreign callable itself.
                if let Some(target) = foreign_static::class_name_target(class_def, classes, attr) {
                    return lower_expr(&target.read_expr(), scopes, classes, current_class);
                }
                for mro_class in &class_def.mro {
                    let mro_def = mro_class_def(mro_class, classes);
                    if let Some(folded) = fold_class_attr(mro_def, attr) {
                        return folded;
                    }
                    if declares_name_outside_class_attrs(mro_def, attr) {
                        break;
                    }
                }
                // The base really is a class name -- `classes.get`
                // succeeded and the scope guard above proved no value binding
                // shadows it -- so there is nothing to fall back to and
                // `lower_expr` below would fail on it. A name that is *not* a
                // class (the ordinary `d.LIMIT` local or parameter), and a
                // class name shadowed by a binding, both fall through.
                panic!(
                    "pycc_mir: internal error: attribute `{attr}` is not a class attribute \
                     reachable through class `{class_name}`'s MRO -- pycc_types::check should \
                     have rejected this HIR with T0044 before it reached pycc_mir"
                );
            }
            let base = lower_expr(base, scopes, classes, current_class);
            // Part 2 of #1026: `numpy.pi` -- the base is a CPython object,
            // so there is no `HirClassDef` to resolve `attr` against and no
            // slot index to compute. The attribute name survives lowering
            // as a `String` (D-105's String-keyed-node precedent) and the
            // load becomes a runtime `pycc_ext_obj_getattr` call.
            //
            // Placed immediately after `lower_expr` rather than beside the
            // enum-member and class-name interceptions above because a
            // foreign module's local name *is* a value binding in `scopes`
            // (`pycc_mir::build` binds it as `Ty::Object`), so both of those
            // interceptions already decline it on their own scope guards.
            // It must come before `class_def_of`, whose `Ty::Object` input
            // would reach `class.rs`'s internal-error panic.
            if base.ty() == Ty::Object {
                return MirExpr::ObjAttrGet {
                    base: Box::new(base),
                    attr: attr.clone(),
                    ty: Ty::Object,
                };
            }
            let class_def = class_def_of(&base, classes);
            // Part 1 of #1284: `x.name` reaching a
            // `staticmethod(<foreign callable>)` class attribute reads the
            // foreign callable. The receiver is not part of the read, but a
            // non-name receiver is still evaluated first (#1346).
            if let Some(target) = foreign_static::instance_target(class_def, classes, attr) {
                let value = lower_expr(&target.read_expr(), scopes, classes, current_class);
                return sequence_after(base, value);
            }
            // #432: walk the MRO for property lookup first (matching
            // CPython's descriptor protocol precedence), then for regular
            // attribute slots using the flat MRO layout.
            for mro_class in &class_def.mro {
                let mro_def = mro_class_def(mro_class, classes);
                if let Some(prop) = mro_def.properties.iter().find(|p| p.name == *attr) {
                    let getter = exact_callee(
                        &class_def.name,
                        mro_class,
                        prop.getter.clone(),
                        scopes,
                        classes,
                    );
                    let ty = lookup(scopes, &format!("$fn:{getter}"));
                    return MirExpr::Call {
                        callee: getter,
                        args: vec![base],
                        ty,
                    };
                }
            }
            // #960: the instance-slot lookup runs *before* the
            // class-attribute fold below, matching
            // `pycc_types::class::resolve_attr_get`'s own properties ->
            // instance `attrs` -> `class_attrs` order and, through it,
            // CPython's precedence: an instance `__dict__` entry shadows a
            // non-data-descriptor class attribute. Two independent sibling
            // bases can contribute an instance slot and a same-named class
            // attribute to one derived class without
            // `pycc_hir::class::attrs::reject_class_attr_collisions` ever
            // comparing them (it only walks a class's own `class_attrs`
            // against its own MRO), so the two really can meet here, and
            // folding first made `pycc_mir` disagree with the type checker
            // -- silently printing the class attribute's value, and aborting
            // codegen outright when the two declared types differ.
            //
            // A class attribute still deliberately never enters
            // `mro_attrs`, so it occupies no instance slot
            // and changes no other attribute's slot index or the allocation
            // size of the class's instances. The one cost of the order is
            // that `mro_attrs` now runs for a class-attribute read too,
            // where it previously never ran; MRO chains are shallow and this
            // is compile-time only. It adds no new panic path either: the
            // property walk above already resolves every entry of
            // `class_def.mro` through `mro_class_def`.
            let flat_attrs = mro_attrs(class_def, classes);
            if let Some((slot, (_, ty))) = flat_attrs
                .iter()
                .enumerate()
                .find(|(_, (name, _))| name == attr)
            {
                return MirExpr::AttrGet {
                    base: Box::new(base),
                    slot,
                    ty: ty.clone(),
                };
            }
            // #911 (Part 1 of #885): `w.MIN_WIDTH` / `self.MIN_WIDTH` -- a
            // class-level attribute read through an instance, reached only
            // when the slot lookup above missed. Folded to its constant
            // *before* the panic below, which is the tail for a name that is
            // neither a slot nor a class attribute anywhere in the MRO.
            //
            // Discarding the already-lowered `base` here is sound because
            // `pycc_types` restricts a class-attribute read's base to a bare
            // name -- evaluating it has no observable effect.
            for mro_class in &class_def.mro {
                let mro_def = mro_class_def(mro_class, classes);
                if let Some(folded) = fold_class_attr(mro_def, attr) {
                    return folded;
                }
            }
            panic!(
                "pycc_mir: internal error: attribute `{attr}` not declared on class `{}` \
                 or any base in its MRO -- pycc_types::check should have rejected this \
                 HIR before it reached pycc_mir",
                class_def.name
            )
        }
        // D-154 (Part 1 of #375): `base.method(args)` resolves to the
        // method's mangled, compile-time-known function symbol (D-006's
        // static-dispatch framing for a non-inherited class -- no vtable,
        // no runtime dispatch), then lowers into an ordinary `MirExpr::Call`
        // with `base` prepended as `self` -- unlike instantiation, a method
        // call never allocates, so it needs no dedicated MIR node of its
        // own (see `MirExpr::Instantiate`'s own doc comment for the
        // contrasting case).
        // #432: method resolution walks the MRO, so a method declared in a
        // base class is found when called on a derived class instance. A
        // subclass method shadows a base class method of the same name (the
        // subclass appears first in the MRO).
        // Issue #1188: one of the four container method names in a module
        // that can see a user class defining it; lowered under the reading
        // `pycc_types` chose.
        HirExpr::ReceiverDispatchedCall { call, container } => {
            receiver_dispatch::lower_receiver_dispatched_call(
                call,
                container,
                scopes,
                classes,
                current_class,
            )
        }
        HirExpr::MethodCall { base, method, args } => {
            // #433: `super().method(args)` — resolve the method starting
            // from the next class in the current class's MRO, using `self`
            // (the current function's first parameter) as the instance.
            // Lowers to a direct `MirExpr::Call` to the resolved method's
            // mangled name, with `self` prepended as the first argument —
            // no vtable, no runtime dispatch (D-006 static-dispatch framing,
            // per the #433 ADR).
            if matches!(base.as_ref(), HirExpr::Super) {
                let current = match current_class {
                    Some(c) => c,
                    None => panic!(
                        "pycc_mir: internal error: `HirExpr::Super` reached lower_expr outside a \
                         method body -- pycc_hir::lower_expr should have rejected this with C0001"
                    ),
                };
                let self_expr = self_expr(scopes);
                // #1337 (D-254): the receiver's MRO after the body's own
                // class, as in the `super().attr` arm above.
                let self_ty = self_expr.ty();
                let receiver = receiver_class(&self_ty).unwrap_or(current);
                let class_def = &classes[receiver];
                let current_pos = class_def
                    .mro
                    .iter()
                    .position(|c| c == current)
                    .expect("pycc_mir: internal error: class not found in its receiver's MRO");
                let super_mro = &class_def.mro[current_pos + 1..];
                // #966: `super().__init__()` ranks constructors exactly as
                // `Instantiate` does, so it takes the same skip -- for
                // `class C(A, B)` with `A` init-less, `super().__init__()`
                // must reach `B.__init__` rather than `A`'s D-225 stub.
                // The skip is gated on the method name because every other
                // `super().m()` resolution is name-agnostic and must stay
                // that way; the `or_else` pass is mandatory, since
                // `class A: pass` / `class C(A)` calling `super().__init__()`
                // has nothing but the implicit constructor to reach and
                // would otherwise panic here.
                let skip_implicit_init = method == "__init__";
                let super_method = |skip_implicit: bool| {
                    super_mro.iter().find_map(|mro_class| {
                        let mro_def = &classes[mro_class.as_str()];
                        if skip_implicit && mro_def.implicit_object_init {
                            return None;
                        }
                        mro_def
                            .methods
                            .iter()
                            .find(|(name, _)| name == method)
                            .map(|(_, mangled)| (mro_class, mangled.clone()))
                    })
                };
                let (owner, mangled) = super_method(skip_implicit_init)
                    .or_else(|| super_method(false))
                    .expect(
                        "pycc_mir: internal error: method not declared on class or any base in its \
                     MRO after the current class -- pycc_types::check should have rejected this \
                     HIR before it reached pycc_mir",
                    );
                let mangled = exact_callee(receiver, owner, mangled, scopes, classes);
                let ty = lookup(scopes, &format!("$fn:{mangled}"));
                let mut call_args = Vec::with_capacity(args.len() + 1);
                call_args.push(self_expr);
                call_args.extend(
                    args.iter()
                        .map(|a| lower_expr(a, scopes, classes, current_class)),
                );
                return MirExpr::Call {
                    callee: mangled,
                    args: call_args,
                    ty,
                };
            }
            // #436: `ClassName.static_method(args)` or
            // `ClassName.class_method(args)` — a method call on a class
            // name (not an instance). The base is `HirExpr::Name` referring
            // to a registered class. `lower_expr` on a bare class name
            // would panic (class names are not in the scope), so intercept
            // here before lowering the base.
            // A value binding of that name shadows the class, so
            // `B.m()` calls the parameter's instance method rather than the
            // class's static or class method -- `pycc_types`' own
            // `MethodCall` class-name arm applies the matching
            // binding/local guard, and this scope check is the MIR-side
            // spelling of it, the same one the class-attribute fold and the
            // enum-member interception already use.
            if let HirExpr::Name(class_name) = base.as_ref()
                && receiver_dispatch::is_unshadowed_class_name(class_name, scopes, classes)
            {
                let class_def = &classes[class_name.as_str()];
                // Part 1 of #1284: `C.name(args)` reaching a
                // `staticmethod(<foreign callable>)` class attribute calls
                // the foreign callable.
                if let Some(target) = foreign_static::class_name_target(class_def, classes, method)
                {
                    return lower_expr(
                        &target.call_expr(args.clone()),
                        scopes,
                        classes,
                        current_class,
                    );
                }
                let static_mangled = class_def.mro.iter().find_map(|mro_class| {
                    let mro_def = mro_class_def(mro_class, classes);
                    mro_def
                        .static_methods
                        .iter()
                        .find(|(name, _)| name == method)
                        .map(|(_, mangled)| mangled.clone())
                });
                if let Some(mangled) = static_mangled {
                    let ty = lookup(scopes, &format!("$fn:{mangled}"));
                    let call_args: Vec<MirExpr> = args
                        .iter()
                        .map(|a| lower_expr(a, scopes, classes, current_class))
                        .collect();
                    return MirExpr::Call {
                        callee: mangled,
                        args: call_args,
                        ty,
                    };
                }
                let class_mangled = class_def.mro.iter().find_map(|mro_class| {
                    let mro_def = mro_class_def(mro_class, classes);
                    mro_def
                        .class_methods
                        .iter()
                        .find(|(name, _)| name == method)
                        .map(|(_, mangled)| (mro_class, mangled.clone()))
                });
                if let Some((owner, mangled)) = class_mangled {
                    let mangled = exact_callee(class_name, owner, mangled, scopes, classes);
                    let ty = lookup(scopes, &format!("$fn:{mangled}"));
                    let mut call_args = Vec::with_capacity(args.len() + 1);
                    call_args.push(MirExpr::NullInstance {
                        ty: Ty::Instance(Box::new(class_name.clone())),
                    });
                    call_args.extend(
                        args.iter()
                            .map(|a| lower_expr(a, scopes, classes, current_class)),
                    );
                    return MirExpr::Call {
                        callee: mangled,
                        args: call_args,
                        ty,
                    };
                }
            }
            let base = lower_expr(base, scopes, classes, current_class);
            // Part 2 of #1026 (PR 2b of #1081): a method call on a foreign
            // CPython object, which has no `HirClassDef` and so no mangled
            // symbol to dispatch to. Placed exactly where the
            // `HirExpr::AttrGet` arm places its own `ObjAttrGet` branch:
            // after `lower_expr` (a foreign module's local name *is* a value
            // binding in `scopes`, so lowering it is correct and necessary)
            // and before `class_def_of`, whose `Ty::Object` input would hit
            // `class.rs`'s "not a class instance" panic. Every earlier
            // branch in this arm is unreachable for such a base: `Super` is
            // a distinct HIR node, and the class-name guard requires a name
            // that is *not* in `scopes`, which a foreign import's local name
            // always is.
            if base.ty() == Ty::Object {
                return MirExpr::ObjMethodCall {
                    base: Box::new(base),
                    method: method.clone(),
                    args: args
                        .iter()
                        .map(|a| lower_expr(a, scopes, classes, current_class))
                        .collect(),
                };
            }
            let class_def = class_def_of(&base, classes);
            // Part 1 of #1284: `x.name(args)` reaching a
            // `staticmethod(<foreign callable>)` class attribute calls the
            // foreign callable. The receiver is not an argument, but a
            // non-name receiver is still evaluated first, before the
            // arguments (#1346).
            if let Some(target) = foreign_static::instance_target(class_def, classes, method) {
                let value = lower_expr(
                    &target.call_expr(args.clone()),
                    scopes,
                    classes,
                    current_class,
                );
                return sequence_after(base, value);
            }
            // #436: check static_methods and class_methods before regular
            // method resolution. Static methods can be called on both
            // classes and instances; class methods can too. When called on
            // an instance, a class method receives the instance as `cls`,
            // while a static method receives nothing -- its receiver is
            // evaluated for its effects only (#1346).
            let static_mangled = class_def.mro.iter().find_map(|mro_class| {
                let mro_def = mro_class_def(mro_class, classes);
                mro_def
                    .static_methods
                    .iter()
                    .find(|(name, _)| name == method)
                    .map(|(_, mangled)| mangled.clone())
            });
            if let Some(mangled) = static_mangled {
                let ty = lookup(scopes, &format!("$fn:{mangled}"));
                let call_args: Vec<MirExpr> = args
                    .iter()
                    .map(|a| lower_expr(a, scopes, classes, current_class))
                    .collect();
                let call = MirExpr::Call {
                    callee: mangled,
                    args: call_args,
                    ty,
                };
                return sequence_after(base, call);
            }
            let class_mangled = class_def.mro.iter().find_map(|mro_class| {
                let mro_def = mro_class_def(mro_class, classes);
                mro_def
                    .class_methods
                    .iter()
                    .find(|(name, _)| name == method)
                    .map(|(_, mangled)| (mro_class, mangled.clone()))
            });
            if let Some((owner, mangled)) = class_mangled {
                let mangled = exact_callee(&class_def.name, owner, mangled, scopes, classes);
                let ty = lookup(scopes, &format!("$fn:{mangled}"));
                let mut call_args = Vec::with_capacity(args.len() + 1);
                call_args.push(base);
                call_args.extend(
                    args.iter()
                        .map(|a| lower_expr(a, scopes, classes, current_class)),
                );
                return MirExpr::Call {
                    callee: mangled,
                    args: call_args,
                    ty,
                };
            }
            regular_method::lower_regular_method_call(
                base,
                class_def,
                method,
                args,
                scopes,
                classes,
                current_class,
            )
        }
        // Part 2a of #1371: `table[k](args)`. `pycc_types` admits the node
        // only when the callee is a CPython object, so it is the existing
        // `ObjCall` with a computed callee rather than a `Name` read; the
        // callee is lowered before the arguments, CPython's order. Codegen
        // decides from the callee's MIR shape whether the shim borrows or
        // consumes it (`pycc_codegen::foreign_call::callee_is_produced`).
        HirExpr::ExprCall { callee, args } => {
            let callee = lower_expr(callee, scopes, classes, current_class);
            debug_assert_eq!(
                callee.ty(),
                Ty::Object,
                "pycc_types admits a call of a subscript result only on a CPython object"
            );
            MirExpr::ObjCall {
                callee: Box::new(callee),
                args: args
                    .iter()
                    .map(|a| lower_expr(a, scopes, classes, current_class))
                    .collect(),
            }
        }
        // Part 8 of #1371: `pycc_types` admits a keyword call only on a
        // CPython object, so its positional half lowers to `ObjCall` or
        // `ObjMethodCall` through the arms above, and the keyword values
        // ride beside it.
        HirExpr::KeywordCall { call, keywords, .. } => {
            // Any other positional half is a front-end defect that
            // `pycc_codegen::foreign_call_emit::emit_keyword_call` reports.
            let call = lower_expr(call, scopes, classes, current_class);
            let (names, values) = keywords
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        lower_expr(value, scopes, classes, current_class),
                    )
                })
                .unzip();
            MirExpr::ObjKeywordCall(Box::new(crate::ObjKeywordCall {
                call,
                names,
                values,
            }))
        }
        // PEP 695 (#387): `GenericClassInstantiate` should never reach MIR
        // — `pycc_types::monomorphize` rewrites every
        // `GenericClassInstantiate` expression to an ordinary
        // `HirExpr::Call` to the mangled class name before MIR lowering
        // runs. If this arm is reached, it indicates a bug in the
        // monomorphization pass.
        HirExpr::GenericClassInstantiate { class, .. } => {
            panic!(
                "pycc_mir: internal error: `GenericClassInstantiate` for class `{class}` \
                 reached MIR lowering -- pycc_types::monomorphize should have rewritten it \
                 to an ordinary `HirExpr::Call` before this point"
            )
        }
        // PEP 572 (#774): `target := value`. Lowers `value` first (bottom-up,
        // like every other composite arm here), then wraps it in
        // `MirExpr::NamedExpr` carrying the lowered value's own type -- this
        // node's own "evaluate, store, yield" codegen (see
        // `pycc_codegen::emit_expr_unchecked`'s `MirExpr::NamedExpr` arm)
        // does the actual store. This function only ever holds a read-only
        // `scopes: &[HashMap<String, Ty>]` slice, so it cannot itself
        // register `name`'s binding for a later statement to `lookup` --
        // that happens one level up, in `pycc_mir::stmt::lower_stmt`'s
        // `ExprStmt`/`If`/`While` arms, via
        // `pycc_mir::stmt::collect_named_expr_bindings`, which walks the
        // already-lowered `MirExpr` tree (so it can read `ty()` directly
        // rather than re-inferring it) and calls `bind_variable` for every
        // `NamedExpr` node found, before that statement's body/next
        // statement is lowered. `pycc_hir::stmt::lower_stmt`'s own
        // `contains_named_expr` restriction guarantees a `NamedExpr` node
        // only ever appears nested inside exactly those three statement
        // placements, so no other `lower_stmt` arm needs this treatment.
        // PEP 572 (#774): by the time this arm runs, `pre_bind_named_expr_targets`
        // (called by `pycc_mir::stmt::lower_stmt` before it lowers the whole
        // statement) has already bound `name` into `scopes` -- see that
        // function's own doc comment for why a pre-pass, not this ordinary
        // bottom-up lowering, has to be the one doing the actual `scopes`
        // mutation. This arm's own job is unchanged from a first glance:
        // lower `value` (again -- cheap, `value` has no side effects beyond
        // further nested walrus bindings the pre-pass already applied) and
        // wrap it in `MirExpr::NamedExpr` carrying its type.
        HirExpr::NamedExpr { name, value } => {
            let value = lower_expr(value, scopes, classes, current_class);
            let ty = value.ty();
            MirExpr::NamedExpr {
                name: name.clone(),
                value: Box::new(value),
                ty,
            }
        }
        // #1254 (D-250): `cond` and the elements are lowered against a copy
        // of `scopes` with one pushed frame holding the synthesized loop
        // variable, so it never becomes a binding of the enclosing scope.
        // `resolve_comp_source` lowers the range operands before it binds
        // `var`, so they still read only the enclosing bindings.
        // `narrowed_ty` searches every frame, so narrowing carries over.
        HirExpr::Comprehension(comp) => {
            let mut inner = scopes.to_vec();
            inner.push(HashMap::new());
            let (source, var_ty) = super::resolve_comp_source(
                &comp.iter,
                &comp.var,
                &mut inner,
                classes,
                current_class,
            );
            let cond = comp
                .cond
                .as_ref()
                .map(|c| lower_expr(c, &inner, classes, current_class));
            let elt = match &comp.elt {
                CompElt::List(e) => MirCompElt::List(lower_expr(e, &inner, classes, current_class)),
                CompElt::Set(e) => {
                    let e = lower_expr(e, &inner, classes, current_class);
                    // #1344: a `set[C]` element carries its class's hash and
                    // equality, exactly as a set literal's does.
                    let ops =
                        set_ops::lower_set_element_ops(&e.ty(), &inner, classes).map(Box::new);
                    MirCompElt::Set(e, ops)
                }
                CompElt::Dict { key, value } => MirCompElt::Dict {
                    key: lower_expr(key, &inner, classes, current_class),
                    value: lower_expr(value, &inner, classes, current_class),
                },
            };
            MirExpr::Comprehension(Box::new(MirComprehension {
                var: comp.var.clone(),
                var_ty,
                source,
                cond,
                elt,
            }))
        }
        // #433: a bare `HirExpr::Super` should never reach MIR lowering —
        // HIR lowering rejects a standalone `super()` with C0001, and
        // `super().method()`/`super().attr` are handled by the special-case
        // blocks above before recursing into `lower_expr` for the base.
        HirExpr::Super => {
            panic!(
                "pycc_mir: internal error: a bare `HirExpr::Super` reached MIR lowering -- \
                 pycc_hir::lower_expr should have rejected this with C0001, or the \
                 `MethodCall`/`AttrGet` arms should have intercepted it before recursing"
            )
        }
    }
}

/// PEP 572 (#774): walks `expr` for every `HirExpr::NamedExpr { name, value }`
/// node, in the expression's own left-to-right evaluation order, and binds
/// each `name` into `scopes` *before* `lower_stmt` calls `lower_expr` on the
/// whole enclosing statement.
///
/// This pre-pass exists because `lower_expr` itself only ever holds a
/// read-only `scopes: &[HashMap<String, Ty>]` slice -- by design, since it is
/// a pure bottom-up lowering with no other reason to mutate scope state. That
/// is fine for every ordinary case, where a `NamedExpr`'s binding only needs
/// to be visible to a *later statement* (handled by
/// `pycc_mir::stmt::collect_named_expr_bindings`, which walks the
/// already-lowered `MirExpr` tree after the fact). It breaks down for a
/// walrus value that itself references an *earlier walrus in the same
/// expression* -- `(a := 1) + (b := a + 1)` -- where `b`'s value needs `a`
/// already bound in `scopes` while `lower_expr` is still partway through
/// lowering the very same top-level expression that defines `a`. Without
/// this pre-pass, `lower_expr`'s `HirExpr::Name("a")` arm would call
/// `lookup(scopes, "a")`, which panics (`pycc_types::check` already proved
/// the program well-typed, including this exact ordering, by running its own
/// mirror-image pre-pass -- see `pycc_types::collect_named_expr_bindings`'s
/// doc comment for the type-checker side of this same fix).
///
/// Each `name`'s type is computed by actually lowering its `value` here
/// (recursing into `value` first, so a nested `NamedExpr` inside it is bound
/// before `value` itself is lowered) -- cheap and side-effect-free (beyond
/// the further nested bindings this same pre-pass already applies), and the
/// simplest way to get a `Ty` without re-deriving one structurally from the
/// unlowered `HirExpr`. `lower_stmt`'s own subsequent full-statement
/// `lower_expr` call re-lowers this same subtree, which is redundant but
/// harmless: `bind_variable`'s `.entry(..).or_insert(..)` semantics make a
/// repeated bind of the same name a no-op.
pub(super) fn pre_bind_named_expr_targets(
    expr: &HirExpr,
    scopes: &mut Vec<HashMap<String, Ty>>,
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) {
    match expr {
        HirExpr::NamedExpr { name, value } => {
            pre_bind_named_expr_targets(value, scopes, classes, current_class);
            let lowered_value = lower_expr(value, scopes, classes, current_class);
            let ty = lowered_value.ty();
            // D-068 review of #780/#774's interaction (blocker finding 1): a
            // walrus target is a reassignment exactly like `Assign`'s own arm
            // in `stmt.rs` (see its paired `kill_narrowing`/`bind_variable`
            // calls), so it must clear any stale narrowing sentinel for
            // `name` the same way -- otherwise a subsequent read still
            // lowers to an unconditional `MirExpr::OptionalUnwrap` for a
            // value the walrus may have just overwritten with `None`.
            super::kill_narrowing(scopes, name);
            super::bind_variable(scopes, name.clone(), ty);
        }
        HirExpr::IntLiteral(_)
        | HirExpr::FloatLiteral(_)
        | HirExpr::BoolLiteral(_)
        | HirExpr::StringLiteral(_)
        | HirExpr::EmptyList(_)
        | HirExpr::EmptyDict(_)
        | HirExpr::NoneLiteral
        | HirExpr::NotImplemented
        | HirExpr::Name(_)
        | HirExpr::Super => {}
        HirExpr::ListPop { list } => {
            if let Some(receiver) = list.attr_expr() {
                pre_bind_named_expr_targets(receiver, scopes, classes, current_class);
            }
        }
        // #1254 (D-250): `pycc_hir` refuses a walrus inside a comprehension.
        HirExpr::Comprehension(_) => {}
        HirExpr::Call { args, .. } | HirExpr::ReceiverClassCall { args } => {
            for arg in args {
                pre_bind_named_expr_targets(arg, scopes, classes, current_class);
            }
        }
        HirExpr::BinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            pre_bind_named_expr_targets(left, scopes, classes, current_class);
            pre_bind_named_expr_targets(right, scopes, classes, current_class);
        }
        HirExpr::IfExp { test, body, orelse } => {
            for part in [test, body, orelse] {
                pre_bind_named_expr_targets(part, scopes, classes, current_class);
            }
        }
        HirExpr::CompareChain { first, links } => {
            for operand in pycc_hir::compare_chain_operands(first, links) {
                pre_bind_named_expr_targets(operand, scopes, classes, current_class);
            }
        }
        HirExpr::UnaryOp { operand, .. } | HirExpr::Unpack { value: operand, .. } => {
            pre_bind_named_expr_targets(operand, scopes, classes, current_class)
        }
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(inner) = part {
                    pre_bind_named_expr_targets(inner, scopes, classes, current_class);
                }
            }
        }
        HirExpr::ListLiteral(es)
        | HirExpr::ObjectList(es)
        | HirExpr::SetLiteral(es)
        | HirExpr::TupleLiteral(es) => {
            for e in es {
                pre_bind_named_expr_targets(e, scopes, classes, current_class);
            }
        }
        HirExpr::Subscript { base, index } => {
            pre_bind_named_expr_targets(base, scopes, classes, current_class);
            pre_bind_named_expr_targets(index, scopes, classes, current_class);
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            pre_bind_named_expr_targets(base, scopes, classes, current_class);
            for bound in [start, stop, step].into_iter().flatten() {
                pre_bind_named_expr_targets(bound, scopes, classes, current_class);
            }
        }
        HirExpr::ListAppend { list, value } => {
            if let Some(receiver) = list.attr_expr() {
                pre_bind_named_expr_targets(receiver, scopes, classes, current_class);
            }
            pre_bind_named_expr_targets(value, scopes, classes, current_class);
        }
        HirExpr::SetAdd { value, .. } => {
            pre_bind_named_expr_targets(value, scopes, classes, current_class);
        }
        HirExpr::DictLiteral(pairs) => {
            for (k, v) in pairs {
                pre_bind_named_expr_targets(k, scopes, classes, current_class);
                pre_bind_named_expr_targets(v, scopes, classes, current_class);
            }
        }
        HirExpr::DictGetOrDefault { dict, key, default } => {
            if let Some(receiver) = dict.attr_expr() {
                pre_bind_named_expr_targets(receiver, scopes, classes, current_class);
            }
            pre_bind_named_expr_targets(key, scopes, classes, current_class);
            pre_bind_named_expr_targets(default, scopes, classes, current_class);
        }
        HirExpr::AttrGet { base, .. } => {
            pre_bind_named_expr_targets(base, scopes, classes, current_class)
        }
        HirExpr::MethodCall { base, args, .. } => {
            pre_bind_named_expr_targets(base, scopes, classes, current_class);
            for arg in args {
                pre_bind_named_expr_targets(arg, scopes, classes, current_class);
            }
        }
        HirExpr::ReceiverDispatchedCall { call, .. } => {
            pre_bind_named_expr_targets(call, scopes, classes, current_class)
        }
        HirExpr::GenericClassInstantiate { args, .. } => {
            for arg in args {
                pre_bind_named_expr_targets(arg, scopes, classes, current_class);
            }
        }
        HirExpr::ExprCall { callee, args } => {
            pre_bind_named_expr_targets(callee, scopes, classes, current_class);
            for arg in args {
                pre_bind_named_expr_targets(arg, scopes, classes, current_class);
            }
        }
        HirExpr::KeywordCall { call, keywords, .. } => {
            pre_bind_named_expr_targets(call, scopes, classes, current_class);
            for (_, value) in keywords {
                pre_bind_named_expr_targets(value, scopes, classes, current_class);
            }
        }
    }
}

/// #911 (Part 1 of #885): folds a class-level attribute read into the
/// literal recorded in `HirClassDef::class_attrs`, or `None` when
/// `class_def` declares no attribute of that name.
///
/// A class attribute has no runtime storage at all -- no instance slot, no
/// module global (unlike an enum member's singleton), and no `pycc_codegen`
/// footprint -- so every read is resolved here, at MIR-lowering time.
fn fold_class_attr(class_def: &HirClassDef, attr: &str) -> Option<MirExpr> {
    let (_, _, value) = class_def
        .class_attrs
        .iter()
        .find(|(name, _, _)| name == attr)?;
    Some(match value {
        ClassAttrValue::Int(i) => MirExpr::IntLiteral(*i),
        ClassAttrValue::Float(f) => MirExpr::FloatLiteral(*f),
        ClassAttrValue::Bool(b) => MirExpr::BoolLiteral(*b),
        ClassAttrValue::Str(s) => MirExpr::StringLiteral(s.clone()),
        // Part 1 of #1284: no constant to fold. Every class-name and
        // instance read reaches `foreign_static`'s rewrite before this
        // fold, so the only caller that gets here is the `super()` walk,
        // which `pycc_types` refuses; returning `None` lets that walk fall
        // through to its own internal-error panic.
        ClassAttrValue::ForeignStatic(_) => return None,
    })
}

/// Lowers the receiver of a `ListAppend`/`ListPop`/`DictGetOrDefault` node
/// together with its static type (#1263): a bare name keeps the plain name
/// and its scope type, exactly as before; an attribute receiver is lowered
/// like any other expression (a slot read, or a `@property` getter call)
/// and typed by that lowered read.
fn lower_container_receiver(
    receiver: &ContainerReceiver,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> (MirContainerReceiver, Ty) {
    match receiver {
        ContainerReceiver::Name(name) => (
            MirContainerReceiver::Name(name.clone()),
            lookup(scopes, name),
        ),
        ContainerReceiver::Attr(read) => {
            let lowered = lower_expr(read, scopes, classes, current_class);
            let ty = lowered.ty();
            (MirContainerReceiver::Attr(Box::new(lowered)), ty)
        }
    }
}
