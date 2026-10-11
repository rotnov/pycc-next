//! Lowering of calls: user functions, builtins, receiver-class and keyword calls.

use super::*;

// #1411: `type(self)(args)` constructs the class `self` is typed as
// in this body. pycc dispatches statically (D-006), and an inherited
// body that constructs through its receiver is lowered once more per
// subclass with `self` retyped (D-254), so this is the receiver's
// run-time class, as in CPython. Resolved straight to the class
// rather than through the `Call` callee path, which a local of the
// same name as the class could otherwise claim.
pub(crate) fn lower_receiver_class_call_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::ReceiverClassCall { args } = expr else {
        unreachable!()
    };
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

pub(crate) fn lower_call_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::Call { callee, args } = expr else {
        unreachable!()
    };
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
    // `len(t)` of a tuple bound to a name (the only tuple `pycc_types`
    // admits there) is the tuple type's arity.
    if callee == "len"
        && args.len() == 1
        && matches!(args[0], MirExpr::Name { .. })
        && let Ty::Tuple(elements) = args[0].ty()
    {
        return MirExpr::IntLiteral(elements.len() as i64);
    }
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
    // `PyListObj`. See `MirExpr::BufferLen`'s own doc comment.
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
    let is_user_defined = scopes
        .iter()
        .any(|s| s.contains_key(&format!("$fn:{callee}")));
    let args = if !is_user_defined && !classes.contains_key(callee.as_str()) {
        match builtin_call::lower_builtin_call(callee, args) {
            Ok(call) => return call,
            Err(args) => args,
        }
    } else {
        args
    };
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
        && let base = super::super::object_narrow::object_operand(base.clone())
        && base.ty() == Ty::Object
        && !scopes
            .iter()
            .any(|scope| scope.contains_key(&format!("$fn:{callee}")))
    {
        return MirExpr::ObjType {
            base: Box::new(base),
        };
    }
    let ty = if callee == "round"
        && !scopes
            .iter()
            .any(|scope| scope.contains_key(&format!("$fn:{callee}")))
    {
        // Mirrors `pycc_types::round::round_call_ty`.
        if args.len() == 2 { Ty::Float } else { Ty::Int }
    } else if callee == "print" {
        Ty::None
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
    } else if (callee == "abs" || callee == "divmod")
        && !scopes
            .iter()
            .any(|scope| scope.contains_key(&format!("$fn:{callee}")))
    {
        // Mirrors `pycc_types::int_builtins`: `abs` keeps a `float`
        // argument's type and makes an `int`/`bool` one `int`;
        // `divmod` of two ints is `tuple[int, int]`.
        match (callee.as_str(), args.first().map(MirExpr::ty)) {
            ("abs", Some(Ty::Float)) => Ty::Float,
            ("abs", _) => Ty::Int,
            _ => Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])),
        }
    } else {
        lookup(scopes, &format!("$fn:{callee}"))
    };
    MirExpr::Call {
        callee: callee.clone(),
        args,
        ty,
    }
}

// Part 2a of #1371: `table[k](args)`. `pycc_types` admits the node
// only when the callee is a CPython object, so it is the existing
// `ObjCall` with a computed callee rather than a `Name` read; the
// callee is lowered before the arguments, CPython's order. Codegen
// decides from the callee's MIR shape whether the shim borrows or
// consumes it (`pycc_codegen::foreign_call::callee_is_produced`).
pub(crate) fn lower_expr_call_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::ExprCall { callee, args } = expr else {
        unreachable!()
    };
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
pub(crate) fn lower_keyword_call_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::KeywordCall { call, keywords, .. } = expr else {
        unreachable!()
    };
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
