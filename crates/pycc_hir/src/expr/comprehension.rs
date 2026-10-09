//! Comprehension lowering (PR-12, D-117; #1254, D-250): the shared header,
//! the loop variable's synthesized name, the name rename, the walrus
//! refusal, and the one [`HirComprehension`] node that both the expression
//! form and the `name = <comp>` statement form are built from.
//!
//! Extracted from `expr.rs` per AGENTS.md's file-decomposition rule (D-185
//! tracking issue #552). `lower_range_call` stays in `expr.rs` because
//! `Stmt::For` shares it.

use super::keyword_bind::SignatureTable;
use super::{contains_named_expr, lower_condition, lower_expr, lower_range_call};
use crate::int_boundary::check_boundary_literal;
use crate::{
    CompElt, CompIter, ContainerReceiver, FStringPart, HirComprehension, HirExpr, HirStmt,
    ImportBinding, unsupported,
};
use pycc_ast::Expr;
use pycc_diag::Diagnostic;

/// Rewrites every occurrence of the bare name `from` inside `expr` to `to`
/// (PR-12, D-117) -- used to give a comprehension's own loop variable a
/// synthesized, collision-proof internal name (see `synthesize_comp_var_name`
/// below) without inventing real lexical scoping. Exhaustive over `HirExpr`
/// on purpose: a future variant added to this enum must add its own arm here
/// too, the same "let the compiler enumerate every site" discipline this
/// project's own `Scalar::List` precedent (D-107) already established for
/// `pycc_codegen`. Safe to apply blindly (no risk of renaming an unrelated
/// same-named binding from some other nested scope): a comprehension's
/// `elt`/`cond`/`key`/`value` can hold no lambda and no nested function def,
/// and a nested comprehension (#1254) renames its own loop variable to a
/// digit-led name before this runs, so the only occurrences of `from` left
/// inside it are reads of the enclosing loop variable.
/// Renames a container node's receiver (#1263): a bare-name receiver is
/// renamed only when it equals `from`, exactly as the plain `String` field
/// always was; an attribute receiver is an ordinary sub-expression and is
/// renamed recursively (`[self.xs.pop() for _ in r]` keeps `self`).
fn rename_receiver(receiver: ContainerReceiver, from: &str, to: &str) -> ContainerReceiver {
    match receiver {
        ContainerReceiver::Name(n) => {
            ContainerReceiver::Name(if n == from { to.to_string() } else { n })
        }
        ContainerReceiver::Attr(receiver) => {
            ContainerReceiver::Attr(Box::new(rename_name_in_expr(*receiver, from, to)))
        }
    }
}

pub(crate) fn rename_name_in_expr(expr: HirExpr, from: &str, to: &str) -> HirExpr {
    let recurse = |e: HirExpr| rename_name_in_expr(e, from, to);
    match expr {
        HirExpr::Name(n) => HirExpr::Name(if n == from { to.to_string() } else { n }),
        HirExpr::IntLiteral(_)
        | HirExpr::FloatLiteral(_)
        | HirExpr::BoolLiteral(_)
        | HirExpr::StringLiteral(_)
        | HirExpr::EmptyList(_)
        | HirExpr::EmptyDict(_)
        | HirExpr::NoneLiteral
        | HirExpr::NotImplemented => expr,
        // `callee` (a bare `String`, never an `HirExpr::Name`) is
        // deliberately left untouched even if it equals `from`: this HIR
        // subset has no first-class functions, so `callee` always names a
        // module-level function definition, never a local variable this
        // rename could plausibly shadow -- unlike `args`, which are
        // recursed into normally.
        HirExpr::Call { callee, args } => HirExpr::Call {
            callee,
            args: args.into_iter().map(recurse).collect(),
        },
        HirExpr::UnaryOp { op, operand } => HirExpr::UnaryOp {
            op,
            operand: Box::new(recurse(*operand)),
        },
        HirExpr::Unpack { value, arity } => HirExpr::Unpack {
            value: Box::new(recurse(*value)),
            arity,
        },
        // `truth_only` is carried through: a renamed comprehension filter
        // stays a truth position.
        HirExpr::BoolOp {
            op,
            left,
            right,
            truth_only,
        } => HirExpr::BoolOp {
            op,
            left: Box::new(recurse(*left)),
            right: Box::new(recurse(*right)),
            truth_only,
        },
        HirExpr::BinOp { op, left, right } => HirExpr::BinOp {
            op,
            left: Box::new(recurse(*left)),
            right: Box::new(recurse(*right)),
        },
        HirExpr::IfExp { test, body, orelse } => HirExpr::IfExp {
            test: Box::new(recurse(*test)),
            body: Box::new(recurse(*body)),
            orelse: Box::new(recurse(*orelse)),
        },
        HirExpr::Compare { op, left, right } => HirExpr::Compare {
            op,
            left: Box::new(recurse(*left)),
            right: Box::new(recurse(*right)),
        },
        HirExpr::CompareChain { first, links } => HirExpr::CompareChain {
            first: Box::new(recurse(*first)),
            links: links
                .into_iter()
                .map(|link| crate::CompareLink {
                    op: link.op,
                    right: recurse(link.right),
                })
                .collect(),
        },
        HirExpr::FString(parts) => HirExpr::FString(
            parts
                .into_iter()
                .map(|part| match part {
                    FStringPart::Literal(s) => FStringPart::Literal(s),
                    FStringPart::Interpolation(e) => {
                        FStringPart::Interpolation(Box::new(recurse(*e)))
                    }
                })
                .collect(),
        ),
        HirExpr::ListLiteral(es) => HirExpr::ListLiteral(es.into_iter().map(recurse).collect()),
        HirExpr::ObjectList(es) => HirExpr::ObjectList(es.into_iter().map(recurse).collect()),
        HirExpr::Subscript { base, index } => HirExpr::Subscript {
            base: Box::new(recurse(*base)),
            index: Box::new(recurse(*index)),
        },
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => HirExpr::Slice {
            base: Box::new(recurse(*base)),
            start: start.map(|s| Box::new(recurse(*s))),
            stop: stop.map(|s| Box::new(recurse(*s))),
            step: step.map(|s| Box::new(recurse(*s))),
        },
        HirExpr::ListAppend { list, value } => HirExpr::ListAppend {
            list: rename_receiver(list, from, to),
            value: Box::new(recurse(*value)),
        },
        HirExpr::DictLiteral(pairs) => HirExpr::DictLiteral(
            pairs
                .into_iter()
                .map(|(k, v)| (recurse(k), recurse(v)))
                .collect(),
        ),
        HirExpr::SetLiteral(es) => HirExpr::SetLiteral(es.into_iter().map(recurse).collect()),
        HirExpr::TupleLiteral(es) => HirExpr::TupleLiteral(es.into_iter().map(recurse).collect()),
        // `list`/`dict`/`set` base-name fields are plain `String`s, mirroring
        // `ListAppend`'s own arm exactly: renamed only when they equal
        // `from`, otherwise left untouched. This matters for a
        // comprehension's own `elt`/`cond` referencing e.g. `xs.pop()` where
        // `xs` is the loop variable being synthesized-renamed -- the common
        // case (some other, non-loop-variable base) must not be touched.
        HirExpr::ListPop { list } => HirExpr::ListPop {
            list: rename_receiver(list, from, to),
        },
        HirExpr::DictGetOrDefault { dict, key, default } => HirExpr::DictGetOrDefault {
            dict: rename_receiver(dict, from, to),
            key: Box::new(recurse(*key)),
            default: Box::new(recurse(*default)),
        },
        HirExpr::SetAdd { set, value } => HirExpr::SetAdd {
            set: if set == from { to.to_string() } else { set },
            value: Box::new(recurse(*value)),
        },
        HirExpr::AttrGet { base, attr } => HirExpr::AttrGet {
            base: Box::new(recurse(*base)),
            attr,
        },
        // Issue #1188: only `call` holds sub-expressions; the container
        // reading is derived from it, so renaming `call` renames both.
        HirExpr::ReceiverDispatchedCall { call, container } => HirExpr::ReceiverDispatchedCall {
            call: Box::new(recurse(*call)),
            container,
        },
        HirExpr::MethodCall { base, method, args } => HirExpr::MethodCall {
            base: Box::new(recurse(*base)),
            method,
            args: args.into_iter().map(recurse).collect(),
        },
        HirExpr::GenericClassInstantiate {
            class,
            type_arg,
            args,
        } => HirExpr::GenericClassInstantiate {
            class,
            type_arg,
            args: args.into_iter().map(recurse).collect(),
        },
        HirExpr::ExprCall { callee, args } => HirExpr::ExprCall {
            callee: Box::new(recurse(*callee)),
            args: args.into_iter().map(recurse).collect(),
        },
        HirExpr::ReceiverClassCall { args } => HirExpr::ReceiverClassCall {
            args: args.into_iter().map(recurse).collect(),
        },
        HirExpr::KeywordCall {
            call,
            keywords,
            span,
        } => HirExpr::KeywordCall {
            call: Box::new(recurse(*call)),
            keywords: keywords
                .into_iter()
                .map(|(name, value)| (name, recurse(value)))
                .collect(),
            span,
        },
        // #433: `Super` carries no names to rename — it is a compile-time
        // marker, not a value with sub-expressions.
        HirExpr::Super => expr,
        // PEP 572 (#774): a walrus target is renamed exactly like a bound
        // `Name` would be (mirroring `HirExpr::Name`'s own arm above) if it
        // happens to collide with the comprehension loop variable being
        // synthesized-renamed; `value` is recursed into normally. A walrus
        // inside a comprehension is refused (D-250), but the renaming runs
        // *before* `refuse_walrus` checks the renamed tree, so this arm is
        // reached on real input: it keeps the doomed tree structurally
        // correct until `refuse_walrus` reports the C0001.
        HirExpr::NamedExpr { name, value } => HirExpr::NamedExpr {
            name: if name == from { to.to_string() } else { name },
            value: Box::new(recurse(*value)),
        },
        HirExpr::Comprehension(comp) => {
            HirExpr::Comprehension(Box::new(rename_in_comprehension(*comp, from, to)))
        }
    }
}

/// Synthesizes a collision-proof internal name for a comprehension's loop
/// variable (D-117): a leading digit can never begin a valid Python
/// identifier (confirmed against the vendored `ruff_python_parser`'s own
/// tokenizer -- a `NAME` token cannot start with a decimal digit), so this
/// string can never be produced by lowering real Python source, no matter
/// what the user names their own variables -- no new lexical-scoping
/// machinery is needed; this is just another ordinary entry in the existing
/// flat, name-keyed slot model. Seeded by the loop target's own byte offset,
/// not a mutable counter: two distinct comprehensions in one file can never
/// share a target's start offset, so this needs no threaded lowering state
/// and stays fully deterministic across repeated compiles of the same
/// source.
///
/// Takes a plain `u32` byte offset (from `pycc_ast::expr_range`) rather than
/// naming `ruff_text_size::TextSize` directly -- `pycc_hir` depends only on
/// `pycc_ast`, never on `ruff_text_size` (Step 0's own re-export widening is
/// this crate's one and only upstream-crate seam), and `pycc_ast`'s own
/// `expr_range`/`stmt_range` exist specifically to keep that boundary from
/// leaking (see their doc comments).
fn synthesize_comp_var_name(target_start: u32, source_name: &str) -> String {
    format!("0comp_{target_start}_{source_name}")
}

/// Resolves a comprehension's `for var in <iter>` clause into a `CompIter`,
/// reusing `Stmt::For`'s own iterable-shape acceptance (D-117): `range(...)`
/// or a bare name (resolved to `Ty::List`/`Ty::Dict`/`Ty::Set`/
/// `Ty::FrozenSet` downstream by `pycc_types`/`pycc_mir`, exactly like a
/// plain `for` loop). Any other iterable expression (Part 1 of #1255) lowers
/// to [`CompIter::Iterable`]; this step has no type information, so whether
/// it is iterable -- only a CPython object is, so far -- is decided by
/// `pycc_types`, which refuses every other type with `C0001`.
fn lower_comprehension_iter(
    iter_expr: &Expr,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<CompIter, Diagnostic> {
    if let Expr::Name(name) = iter_expr {
        return Ok(CompIter::Name(name.id.as_str().to_string()));
    }
    let range_call = match iter_expr {
        Expr::Call(call) if matches!(call.func.as_ref(), Expr::Name(callee) if callee.id.as_str() == "range") => {
            Some(call)
        }
        _ => None,
    };
    let Some(call) = range_call else {
        // Literal `true`, for the same enclosing-scope reason as the
        // `range(...)` operands below (D-149 correction 6).
        let iterable = lower_expr(iter_expr, true, class_name, imports, signatures)?;
        return Ok(CompIter::Iterable(Box::new(iterable)));
    };
    if !call.arguments.keywords.is_empty() {
        return Err(unsupported(
            "keyword arguments to range() are not supported yet",
            call.range,
        ));
    }
    // Literal `true`, not comprehension-internal, not the threaded ambient
    // value (D-149 correction 6): a comprehension's outermost iterable --
    // including a `range(...)` call and its arguments -- evaluates in the
    // *enclosing* scope per real CPython grammar, confirmed against the
    // oracle (`[x for x in range((yield 3))]` is `'yield' outside function`
    // at module scope, and valid inside a `def` -- the ordinary
    // scope-dependent rule, not the comprehension-internal one). The literal
    // `true` here is not the theoretically correct value; it reproduces
    // today's unconditional `C0001` behavior for this narrow sub-position
    // with zero regression risk, and getting the enclosing-scope split fully
    // right for it is deliberately deferred (see D-149 and its own "out of
    // scope" section).
    let (start, stop, step) = lower_range_call(call, true, class_name, imports, signatures)?;
    Ok(CompIter::Range { start, stop, step })
}

/// Validates and lowers a comprehension's shared shape (D-117): exactly one
/// generator clause, no `async for`, a bare-name loop target, at most one
/// `if` filter. Returns the loop target's *source* name, its synthesized
/// internal replacement, the resolved `CompIter`, and the (not-yet-renamed)
/// lowered `if`-filter expression, if present -- renaming is the caller's
/// job (`lower_list_comp`/`lower_set_comp`/`lower_dict_comp` below), since `elt`/`key`/`value` also need the
/// identical rename and this helper has no visibility into which of those
/// the caller is building.
///
/// `iter` (the resolved `CompIter` returned above) is deliberately **never**
/// renamed to this comprehension's *own* synthesized variable -- neither here
/// nor by `lower_list_comp`/`lower_set_comp`/`lower_dict_comp` -- unlike
/// `cond`/`elt`/`key`/`value`, which all are. (A comprehension nested in an
/// *enclosing* comprehension's `cond`/`elt` does get its `iter` renamed to the
/// enclosing variable, by `rename_in_comprehension`; that is a different
/// rename.) This is not an
/// oversight: it matches real CPython scoping. A comprehension's outermost
/// iterable expression evaluates in the *enclosing* scope, before the
/// comprehension's own scope exists at all -- `[i for i in range(i)]`'s
/// `range(i)` reads the *enclosing* `i`, not the comprehension's own loop
/// variable (confirmed directly against CPython). Renaming `iter`'s
/// occurrences of the source loop-variable name would therefore be actively
/// wrong, not merely redundant: it would make `range(i)` read the
/// comprehension's own (not-yet-bound) synthesized variable instead of
/// whatever `i` means in the enclosing scope. See
/// `a_comprehension_range_iterable_referencing_the_loop_variables_own_source_name_is_not_renamed`
/// and
/// `a_comprehension_bare_name_iterable_sharing_the_loop_variables_own_source_name_is_not_renamed`
/// below, which pin this behavior directly -- without them, a future change
/// that "fixed" this asymmetry by renaming `iter` too would silently break
/// correct scoping with every existing test still green.
pub(crate) fn lower_comprehension_header(
    generators: &[pycc_ast::Comprehension],
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<(String, String, CompIter, Option<HirExpr>), Diagnostic> {
    // Named `generator`, not `gen` -- `gen` is a reserved keyword as of the
    // 2024 edition (this workspace's own edition, reserved for a future
    // generator-block feature), so the brief's own `gen` binding does not
    // compile here.
    let [generator] = generators else {
        return Err(unsupported(
            "a comprehension with more than one `for` clause is not supported yet",
            generators.first().map(|g| g.range).unwrap_or_default(),
        ));
    };
    if generator.is_async {
        return Err(unsupported(
            "async comprehensions are not supported yet",
            generator.range,
        ));
    }
    let Expr::Name(var) = &generator.target else {
        return Err(unsupported(
            "only a bare name comprehension target is supported so far",
            pycc_ast::expr_range(&generator.target),
        ));
    };
    let cond = match generator.ifs.as_slice() {
        [] => None,
        // Literal `true`, not the threaded ambient value: a comprehension's
        // `if`-filter is lexically inside the comprehension's own scope
        // (D-149 correction 5), so a `yield` there is governed by a third,
        // scope-independent CPython rule (`'yield' inside list
        // comprehension`, unconditionally invalid regardless of what
        // encloses the comprehension) that this issue deliberately does not
        // implement -- hardcoding `true` here preserves today's exact
        // `C0001`-in-both-scopes behavior byte-for-byte instead of emitting
        // the wrong classification.
        [single] => Some(lower_condition(
            single, true, class_name, imports, signatures,
        )?),
        _ => {
            return Err(unsupported(
                "a comprehension with more than one `if` filter is not supported yet",
                generator.range,
            ));
        }
    };
    let iter = lower_comprehension_iter(&generator.iter, class_name, imports, signatures)?;
    let source_name = var.id.as_str().to_string();
    let synth_var =
        synthesize_comp_var_name(pycc_ast::expr_range(&generator.target).start, &source_name);
    Ok((source_name, synth_var, iter, cond))
}

/// Refuses a walrus anywhere inside a lowered comprehension (#1254, D-250).
/// CPython binds a comprehension-embedded walrus in the *enclosing* scope,
/// which the node-scoped loop variable does not model. Checked once here,
/// on the finished node, so the statement and expression forms share it.
fn refuse_walrus<R>(comp: HirComprehension, range: R) -> Result<HirComprehension, Diagnostic>
where
    std::ops::Range<u32>: From<R>,
{
    if comprehension_contains_named_expr(&comp) {
        return Err(unsupported(
            "a walrus assignment (`:=`) inside a comprehension is not supported yet",
            range,
        ));
    }
    Ok(comp)
}

/// Whether any part of `comp` -- the range operands, `cond`, or the element
/// expressions -- contains a walrus. `contains_named_expr`'s comprehension
/// arm delegates here.
pub(crate) fn comprehension_contains_named_expr(comp: &HirComprehension) -> bool {
    let iter_has = match &comp.iter {
        CompIter::Range { start, stop, step } => {
            contains_named_expr(start) || contains_named_expr(stop) || contains_named_expr(step)
        }
        CompIter::Iterable(iterable) => contains_named_expr(iterable),
        CompIter::Name(_) => false,
    };
    iter_has
        || comp.cond.as_ref().is_some_and(contains_named_expr)
        || match &comp.elt {
            CompElt::List(e) | CompElt::Set(e) => contains_named_expr(e),
            CompElt::Dict { key, value } => contains_named_expr(key) || contains_named_expr(value),
        }
}

/// Renames `from` to `to` throughout a nested comprehension (#1254): its
/// range operands, a bare-name or expression iterable, `cond` and the element expressions.
/// A nested comprehension's own `var` is digit-led, so it never equals
/// `from`, and its source-name occurrences were already renamed when it was
/// lowered. The iterable *is* renamed here, unlike the outermost iterable
/// of the comprehension being renamed: a nested comprehension's iterable is
/// evaluated inside the enclosing comprehension's scope (PEP 709), so
/// `[len([y for y in x]) for x in range(3)]` reads the outer loop variable.
fn rename_in_comprehension(comp: HirComprehension, from: &str, to: &str) -> HirComprehension {
    let recurse = |e: HirExpr| rename_name_in_expr(e, from, to);
    let iter = match comp.iter {
        CompIter::Range { start, stop, step } => CompIter::Range {
            start: recurse(start),
            stop: recurse(stop),
            step: recurse(step),
        },
        CompIter::Name(n) => CompIter::Name(if n == from { to.to_string() } else { n }),
        CompIter::Iterable(iterable) => CompIter::Iterable(Box::new(recurse(*iterable))),
    };
    HirComprehension {
        var: comp.var,
        iter,
        cond: comp.cond.map(recurse),
        elt: match comp.elt {
            CompElt::List(e) => CompElt::List(recurse(e)),
            CompElt::Set(e) => CompElt::Set(recurse(e)),
            CompElt::Dict { key, value } => CompElt::Dict {
                key: recurse(key),
                value: recurse(value),
            },
        },
    }
}

/// Lowers `[elt for ...]` into a [`HirComprehension`], in either position.
pub(crate) fn lower_list_comp(
    comp: &pycc_ast::ExprListComp,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirComprehension, Diagnostic> {
    let (source_name, synth_var, iter, cond) =
        lower_comprehension_header(&comp.generators, class_name, imports, signatures)?;
    // Literal `true`: `elt` is lexically inside the comprehension's own
    // scope, same reasoning as `lower_comprehension_header`'s `cond` arm
    // above (D-149 correction 5) -- preserves today's `C0001` classification
    // for a comprehension-internal `yield`/`yield from` in both enclosing
    // scopes.
    let elt_hir = lower_expr(&comp.elt, true, class_name, imports, signatures)?;
    check_boundary_literal(
        &elt_hir,
        pycc_ast::expr_range(&comp.elt),
        "listcomp element",
    )?;
    let elt = rename_name_in_expr(elt_hir, &source_name, &synth_var);
    let cond = cond.map(|c| rename_name_in_expr(c, &source_name, &synth_var));
    refuse_walrus(
        HirComprehension {
            var: synth_var,
            iter,
            cond,
            elt: CompElt::List(elt),
        },
        comp.range,
    )
}

/// Lowers `{elt for ...}` into a [`HirComprehension`], in either position.
pub(crate) fn lower_set_comp(
    comp: &pycc_ast::ExprSetComp,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirComprehension, Diagnostic> {
    let (source_name, synth_var, iter, cond) =
        lower_comprehension_header(&comp.generators, class_name, imports, signatures)?;
    // Literal `true`: same reasoning as `lower_list_comp`'s `elt` above
    // (D-149 correction 5).
    let elt_hir = lower_expr(&comp.elt, true, class_name, imports, signatures)?;
    check_boundary_literal(&elt_hir, pycc_ast::expr_range(&comp.elt), "setcomp element")?;
    let elt = rename_name_in_expr(elt_hir, &source_name, &synth_var);
    let cond = cond.map(|c| rename_name_in_expr(c, &source_name, &synth_var));
    refuse_walrus(
        HirComprehension {
            var: synth_var,
            iter,
            cond,
            elt: CompElt::Set(elt),
        },
        comp.range,
    )
}

/// Lowers `{key: value for ...}` into a [`HirComprehension`], in either
/// position.
pub(crate) fn lower_dict_comp(
    comp: &pycc_ast::ExprDictComp,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirComprehension, Diagnostic> {
    // Real Python's dict-comprehension grammar (`{k: v for ...}`) has no
    // `**`-unpacking form the way a plain `Expr::Dict` literal does -- but
    // unlike that literal case, the parser does *not* reject
    // `{**x for k in y}`-shaped source at parse time: confirmed directly
    // against the vendored `ruff_python_parser` (0.0.6), which parses it
    // successfully as `ExprDictComp { key: None, value: Name("x"), .. }`,
    // silently dropping the `**` token rather than erroring. The brief this
    // task followed assumed `key: None` was unreachable from real parsed
    // source and modeled it with an `unreachable!()`/`.expect()` internal
    // panic; that assumption is false, so this is a real (if unusual)
    // C0001 capability diagnostic, mirroring `Expr::Dict`'s own analogous
    // `**`-unpacking rejection, not an internal-error panic.
    let Some(key_expr) = comp.key.as_deref() else {
        return Err(unsupported(
            "dict-unpacking (`**expr`) inside a dict comprehension is not supported yet",
            pycc_ast::expr_range(&comp.value),
        ));
    };
    let (source_name, synth_var, iter, cond) =
        lower_comprehension_header(&comp.generators, class_name, imports, signatures)?;
    // Literal `true` for both `key` and `value`: same reasoning as
    // `lower_list_comp`'s `elt` above (D-149 correction 5) -- `key` and
    // `value` are both lexically inside the comprehension's own scope.
    let key = rename_name_in_expr(
        lower_expr(key_expr, true, class_name, imports, signatures)?,
        &source_name,
        &synth_var,
    );
    let value_hir = lower_expr(&comp.value, true, class_name, imports, signatures)?;
    check_boundary_literal(
        &value_hir,
        pycc_ast::expr_range(&comp.value),
        "dictcomp value",
    )?;
    let value = rename_name_in_expr(value_hir, &source_name, &synth_var);
    let cond = cond.map(|c| rename_name_in_expr(c, &source_name, &synth_var));
    refuse_walrus(
        HirComprehension {
            var: synth_var,
            iter,
            cond,
            elt: CompElt::Dict { key, value },
        },
        comp.range,
    )
}

/// `target = <comp>` (PR-12, D-117): the statement form keeps its own
/// `HirStmt` variants -- about twenty statement passes dispatch on them --
/// and is built from the same lowered node as the expression form.
///
/// A comprehension over an iterable expression (Part 1 of #1255) is the
/// exception: it produces a CPython object, not a native container, so it
/// lowers to a plain `HirStmt::Assign` of the expression form and binds
/// `target` exactly as any other object-valued assignment does. None of
/// the native-container statement passes applies to it.
pub(crate) fn comp_assign_stmt(target: &str, comp: HirComprehension) -> HirStmt {
    if let CompIter::Iterable(_) = comp.iter {
        return HirStmt::Assign {
            target: target.to_string(),
            value: HirExpr::Comprehension(Box::new(comp)),
        };
    }
    let HirComprehension {
        var,
        iter,
        cond,
        elt,
    } = comp;
    let target = target.to_string();
    let cond = cond.map(Box::new);
    match elt {
        CompElt::List(elt) => HirStmt::ListCompAssign {
            target,
            var,
            iter,
            cond,
            elt: Box::new(elt),
        },
        CompElt::Set(elt) => HirStmt::SetCompAssign {
            target,
            var,
            iter,
            cond,
            elt: Box::new(elt),
        },
        CompElt::Dict { key, value } => HirStmt::DictCompAssign {
            target,
            var,
            iter,
            cond,
            key: Box::new(key),
            value: Box::new(value),
        },
    }
}
