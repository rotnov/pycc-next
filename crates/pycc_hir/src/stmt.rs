//! Statement lowering: `lower_stmt`, `lower_body`, and
//! `lower_elif_else_clauses`, the recursive-descent core that walks a
//! function or module body and produces `HirStmt` nodes (or a capability /
//! context-invalidity diagnostic).
//!
//! Extracted from `lib.rs` per AGENTS.md's file-decomposition rule (issue
//! #141): this is the one part of `crates/pycc_hir/src/lib.rs` this change
//! touches, and it was already a well-defined, low-fan-in cohesion unit (11
//! call expressions total -- 9 within this module's own recursive descent
//! plus 2 from `lib.rs` -- all in production code, zero from the inline test
//! module -- every existing test reaches lowering exclusively through the
//! public `lower_checked` entry point in `lib.rs`).
//!
//! `in_loop: bool` (D-148) is the module's own piece of state: `true`
//! exactly when the statement being lowered is (transitively) inside a
//! `while`/`for`/`for-range` loop body reached via one of *this* module's
//! own `lower_body` calls -- entering a function body via `lower_function`
//! resets it to `false`, and an `if`/`elif`/`else` branch passes its caller's
//! value through unchanged (a conditional does not introduce or reset loop
//! nesting). It exists so `Stmt::Break`/`Stmt::Continue` can distinguish a
//! context-invalid occurrence (no enclosing loop -- CPython raises
//! `SyntaxError`, so this now reuses `L0001`) from a valid-but-unimplemented
//! occurrence (a real enclosing loop -- still `C0001`, loop control-flow
//! codegen remains out of scope).
//!
//! `in_function: bool` (D-149, issue #361's own follow-up) is this module's
//! second piece of threaded context, but purely as a pass-through: none of
//! this module's own `in_loop`-driven logic reads it. `lower_stmt`/
//! `lower_body`/`lower_elif_else_clauses` all gained it because they sit on
//! the only route from the two places that decide it (`module.rs`'s
//! `lower_top_level_item` module-level dispatch and `func.rs`'s
//! `lower_function` body dispatch) to `expr.rs`'s `lower_expr`, which every one of these three
//! functions calls directly (test expressions, `Return` values, `AnnAssign`
//! values, assignment/subscript values) -- D-148's own framing ("the two
//! lowering passes stay independently parameterized") remains true in the
//! sense that the two flags never share a variable or collapse into one
//! enum, but is refined by D-149: this module is not left untouched by an
//! `expr.rs`-side flag the way that framing might suggest.
//!
//! `in_finally: bool` (D-193, PEP 765, issue #738 -- Part 1 of #543) is this
//! module's third piece of threaded context, and it governs `Stmt::Return`,
//! `Stmt::Break`, and `Stmt::Continue` uniformly. It becomes `true` while
//! lowering the `finalbody` of a `try`/`finally` (`Stmt::Try`'s own `body`,
//! `handlers`, and `orelse` inherit whatever value was already in scope --
//! entering a `finally` clause is the only thing that sets it), propagates
//! unchanged through `if`/`elif`/`else`, nested `try`'s non-`finally` parts,
//! and `match`/`case`, and is reset to `false` while lowering the body of a
//! `while`/`for` loop reached from inside that `finally` (mirroring
//! `in_loop`'s own reset on loop entry) -- entering a function body via
//! `lower_function` resets it to `false` too, exactly like `in_loop`.
//!
//! The finally-specific diagnostic only fires when a valid escape target
//! also exists: `in_finally && in_function` for `Stmt::Return`, `in_finally
//! && in_loop` for `Stmt::Break`/`Stmt::Continue`. Verified directly against
//! `python3.14 -W all`: a `return`/`break`/`continue` directly in a
//! `finally` with NO valid target anywhere (e.g. a bare `break` at module
//! scope) makes CPython raise the pre-existing `SyntaxError` for that
//! missing target (`'break' outside loop`, `'continue' not properly in
//! loop`, `'return' outside function`) as the actual fatal error --
//! CPython's finally-specific `SyntaxWarning` prints alongside it, but is
//! not itself fatal and does not override the missing-target error. Only
//! once a real target exists outside the `finally` does CPython's fatal
//! failure become the finally-specific one (with exit code 0, since a
//! `SyntaxWarning` alone does not fail compilation). pycc mirrors that
//! precedence: without a valid target, this module keeps deferring to its
//! own pre-existing `in_loop`/`T0024` handling instead of reporting the
//! finally-specific message.
//!
//! This exact shape -- one flag shared by all three statement kinds, reset
//! by the nearest enclosing loop but not by a plain conditional or a nested
//! non-`finally` `try` part -- was verified empirically against CPython
//! 3.14's actual `SyntaxWarning` behavior (not assumed from the PEP text
//! alone): `return`, `break`, and `continue` are equally suppressed by an
//! intervening loop inside the `finally`, even though a `return` inside that
//! inner loop still exits the function exactly as it would without the
//! loop. A naive reading of PEP 765 predicts `return` should NOT be shielded
//! by an intervening loop the way `break`/`continue` are (a `return` always
//! escapes the `finally` regardless of loop nesting) -- CPython's compiler
//! does not implement it that way, so a two-flag design mirroring that naive
//! reading would silently diverge from upstream's own diagnostic surface.
//! `while`/`for`'s own `else:` clause is untested here because both are
//! unsupported syntax in this compiler today (`while/else`/`for/else` reject
//! with `C0001` before either loop kind's body is lowered).
//!
//! `except_star: ExceptStarCtx` (#795, PEP 654) is this module's fourth
//! piece of threaded context, and the first that is not a `bool`. The type,
//! and the full table of which statements each of its three states rejects,
//! live with the `except*` lowering itself in [`exception`]; the arms below
//! only read the value this module threads through them.

mod ann_assign;
mod assert_stmt;
mod assign;
mod aug_assign;
pub(crate) mod chain_assign;
pub(crate) mod del;
pub(crate) use chain_assign::lower_stmt_expanded;
mod exception;
mod for_loop;
mod match_stmt;
mod type_checking;
mod unpack;
mod unsupported;

#[cfg(test)]
use crate::HirPattern;
use crate::class::ClassAnnotationInfo;
use crate::expr::keyword_bind::SignatureTable;
use crate::expr::{contains_named_expr, lower_condition, lower_expr, lower_range_call};
use crate::stmt::type_checking::{
    break_context_violation, check_guarded_body, continue_context_violation,
    return_context_violation,
};
use crate::{CompIter, HirExpr, HirStmt, ImportBinding, Ty, context_invalid, unsupported};
pub(crate) use exception::ExceptStarCtx;
use exception::lower_except_handler;
use match_stmt::lower_match;
#[cfg(test)]
use match_stmt::lower_pattern;
#[cfg(test)]
use pycc_ast::Pattern;
use pycc_ast::{ElifElseClause, Expr, Stmt};
use pycc_diag::Diagnostic;

/// True exactly when `test` is CPython's standard `typing.TYPE_CHECKING`
/// guard expression, spelled either as the bare name (`from typing import
/// TYPE_CHECKING`) or the qualified attribute access (`import typing`,
/// then `typing.TYPE_CHECKING`) (#790). Checked purely syntactically,
/// matching this module's existing bare-name `Final` precedent (`is_final`,
/// a local `let` binding below in the `Stmt::AnnAssign` arm, not a separate
/// function) and `expr.rs`'s textual, non-import-gated resolution of
/// `math.sqrt`-shaped attribute access -- `lower_stmt`/`lower_body` have no
/// access to the module-level import side-table `module::lower_all` builds, so
/// this recognizes the two concrete spellings the real-world idiom uses
/// rather than resolving a general expression through `pycc_std`.
/// Deliberately does not recognize a negated or compound test (`if not
/// TYPE_CHECKING:`, `if TYPE_CHECKING and x:`) -- only the bare guard
/// itself has the real-world precedent (bodies containing constructs pycc
/// doesn't support) this fold exists to unblock; a compound test is left
/// to ordinary lowering, which type-checks both branches as it would any
/// other `if`. Since #790 registered `TYPE_CHECKING` as a resolvable
/// marker symbol, using it inside such a test now reaches the dedicated
/// `type_checking_marker_is_not_a_value` diagnostic (T0021) rather than
/// the unresolved-import error (C0002) it produced before #790 -- a
/// different diagnostic at a later pipeline stage, not unchanged behavior.
///
/// Known gap (tracked in #798, deliberately not fixed here): this check is
/// not import-gated and not shadow-aware. A module that never imports
/// `typing`/`TYPE_CHECKING` but happens to bind its own truthy module-level
/// `TYPE_CHECKING` name would, under CPython, execute the guarded body --
/// this compiler still folds it away as dead code, silently diverging from
/// CPython for that (contrived) program. #798 tracks gating the fold on an
/// actual `typing` import.
///
/// Part 1 of #883 (#962): the attribute form also folds through a module
/// alias -- `import typing as t` then `if t.TYPE_CHECKING:` -- when the
/// receiver resolves to `StdModule::Typing` through the very same
/// last-binding-wins lookup every other stdlib receiver uses
/// (`expr::std_receiver`), so `import typing as t; import enum as t` does
/// *not* fold: the live `t` is `enum`, and CPython raises `AttributeError`
/// there. Without the alias branch, the test would lower through
/// `expr::std_receiver` into `Name("typing.TYPE_CHECKING")` and fail in
/// `pycc_types` as a marker used as a value, a worse diagnostic than the
/// bare spelling gets. The bare-name spelling stays syntactic and
/// non-gated (#798), and so does the `typing.` spelling through
/// `std_receiver`'s textual fallback -- unless `typing` itself is rebound
/// to another stdlib module by an alias. An alias of any *other* module
/// (`import math as t`) does not fold, so `t.TYPE_CHECKING` then reaches
/// the ordinary attribute path and its "has no attribute" diagnostic.
pub(crate) fn is_type_checking_guard(test: &Expr, imports: &[ImportBinding]) -> bool {
    match test {
        Expr::Name(name) => name.id.as_str() == "TYPE_CHECKING",
        Expr::Attribute(attr) => {
            attr.attr.as_str() == "TYPE_CHECKING"
                && match attr.value.as_ref() {
                    Expr::Name(receiver) => {
                        crate::expr::std_receiver(receiver.id.as_str(), imports)
                            == Some(pycc_std::StdModule::Typing)
                    }
                    _ => false,
                }
        }
        _ => false,
    }
}

/// Lowers one statement to one HIR statement.
///
/// # Panics
///
/// On a multi-target `Stmt::Assign` (a chained assignment), which only
/// [`lower_stmt_expanded`] accepts.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_stmt(
    stmt: &Stmt,
    aliases: &[(String, Ty)],
    in_loop: bool,
    in_function: bool,
    in_finally: bool,
    except_star: ExceptStarCtx,
    class_name: Option<&str>,
    type_param: Option<&str>,
    class_defs: &[ClassAnnotationInfo],
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirStmt, Diagnostic> {
    let lowered = match stmt {
        Stmt::Expr(expr_stmt) => HirStmt::ExprStmt(lower_expr(
            &expr_stmt.value,
            in_function,
            class_name,
            imports,
            signatures,
        )?),
        Stmt::Assign(assign) => {
            assign::lower_assign(assign, in_function, class_name, imports, signatures)?
        }
        // #1209: `target op= value` is lowered as the plain assignment
        // `target = target op value` (see `aug_assign`), through this same
        // function so every `Stmt::Assign` check, including the walrus
        // placement check below, applies to the rewritten statement.
        Stmt::AugAssign(aug) => {
            return lower_stmt(
                &aug_assign::desugar_aug_assign(aug)?,
                aliases,
                in_loop,
                in_function,
                in_finally,
                except_star,
                class_name,
                type_param,
                class_defs,
                imports,
                signatures,
            );
        }
        // #1369: `assert test, msg` is lowered as `if test: pass` / `else:
        // raise AssertionError(msg)` (see `assert_stmt`), through this same
        // function so the `Stmt::If` truthiness rules and the walrus
        // placement check below apply to the rewritten statement unchanged.
        Stmt::Assert(assert) => {
            return lower_stmt(
                &assert_stmt::desugar_assert(assert),
                aliases,
                in_loop,
                in_function,
                in_finally,
                except_star,
                class_name,
                type_param,
                class_defs,
                imports,
                signatures,
            );
        }
        Stmt::AnnAssign(ann) => ann_assign::lower_ann_assign(
            ann,
            aliases,
            in_function,
            class_name,
            type_param,
            class_defs,
            imports,
            signatures,
        )?,
        Stmt::If(if_stmt) if is_type_checking_guard(&if_stmt.test, imports) => {
            // #790: `if TYPE_CHECKING:` is CPython's standard idiom for
            // guarding imports/statements meant only for static type
            // checkers -- `typing.TYPE_CHECKING` is always `False` at
            // runtime, so the guarded body never executes. Constant-fold it
            // here, before either `lower_expr` or `lower_body` ever sees the
            // body: the guard commonly wraps constructs this compiler
            // doesn't support elsewhere (forward-reference-only imports,
            // typing-only names), and since the body is genuinely dead code
            // it must never block compilation of the rest of the module.
            // Only `orelse` (an `elif`/`else` chain, live at runtime
            // whenever the `TYPE_CHECKING` guard itself is skipped) is
            // lowered normally. The synthesized `HirExpr::BoolLiteral(false)`
            // test documents the fold in the HIR itself rather than
            // silently keeping the original (never evaluated) test
            // expression around.
            //
            // #905: the fold hides the body from `lower_body`, and with it
            // every context check `lower_stmt` performs -- CPython rejects a
            // `return` in a `finally` or a `break` with no loop at compile
            // time whether or not the branch ever runs. Re-check the body
            // for exactly those violations (and nothing else) before it is
            // discarded; see `type_checking`'s own module doc comment for
            // the contract that keeps this silent on capability gaps.
            check_guarded_body(
                &if_stmt.body,
                in_loop,
                in_function,
                in_finally,
                except_star,
                class_name,
                imports,
                signatures,
            )?;
            HirStmt::If {
                test: HirExpr::BoolLiteral(false),
                body: vec![],
                orelse: lower_elif_else_clauses(
                    &if_stmt.elif_else_clauses,
                    aliases,
                    in_loop,
                    in_function,
                    in_finally,
                    except_star,
                    class_name,
                    type_param,
                    class_defs,
                    imports,
                    signatures,
                )?,
            }
        }
        Stmt::If(if_stmt) => HirStmt::If {
            test: lower_condition(&if_stmt.test, in_function, class_name, imports, signatures)?,
            body: lower_body(
                &if_stmt.body,
                aliases,
                in_loop,
                in_function,
                in_finally,
                except_star,
                class_name,
                type_param,
                class_defs,
                imports,
                signatures,
            )?,
            orelse: lower_elif_else_clauses(
                &if_stmt.elif_else_clauses,
                aliases,
                in_loop,
                in_function,
                in_finally,
                except_star,
                class_name,
                type_param,
                class_defs,
                imports,
                signatures,
            )?,
        },
        Stmt::While(while_stmt) => {
            if !while_stmt.orelse.is_empty() {
                return Err(unsupported(
                    "while/else is not supported yet",
                    while_stmt.range,
                ));
            }
            HirStmt::While {
                test: lower_condition(
                    &while_stmt.test,
                    in_function,
                    class_name,
                    imports,
                    signatures,
                )?,
                body: lower_body(
                    &while_stmt.body,
                    aliases,
                    true,
                    in_function,
                    // A loop entered from inside a `finally` shields its own
                    // body from the outer `finally`'s PEP 765 restriction --
                    // verified against CPython 3.14 (see this module's own
                    // doc comment above).
                    false,
                    // #795 (PEP 654): a loop entered from inside an `except*`
                    // clause shields its own body's `break`/`continue` -- but
                    // not its `return` -- from that clause's restriction, so
                    // the context is demoted rather than cleared.
                    except_star.shielded_by_loop(),
                    class_name,
                    type_param,
                    class_defs,
                    imports,
                    signatures,
                )?,
            }
        }
        Stmt::For(for_stmt) => for_loop::lower_for(
            for_stmt,
            aliases,
            in_function,
            except_star,
            class_name,
            type_param,
            class_defs,
            imports,
            signatures,
        )?,
        Stmt::Return(ret) => {
            // #795 (PEP 654) and PEP 765 (#738, Part 1 of #543): both
            // rules, their `in_function` conjuncts and their relative
            // precedence live in `return_context_violation`, which #905's
            // `TYPE_CHECKING` walker calls too so the rule exists once.
            // `L0001` is reused (not a new code) for this class of
            // post-parse context violation.
            //
            // The `except*` check runs first because CPython's own
            // precedence makes it the fatal `SyntaxError` while the PEP 765
            // restriction is only a `SyntaxWarning`; the `in_function`
            // conjunct on both matches CPython 3.14.6, which reports the
            // pre-existing `SyntaxError: 'return' outside function` at
            // module scope -- pycc defers to its own `T0024` there.
            if let Some(message) = return_context_violation(in_function, in_finally, except_star) {
                return Err(context_invalid(message, pycc_ast::stmt_range(stmt)));
            }
            // #1418: an `ext` module's admitted `return NotImplemented`.
            HirStmt::Return(
                ret.value
                    .as_deref()
                    .map(
                        |e| match crate::not_implemented::lower_return_value(e, aliases) {
                            Some(lowered) => Ok(lowered),
                            None => lower_expr(e, in_function, class_name, imports, signatures),
                        },
                    )
                    .transpose()?,
            )
        }
        Stmt::Break(_) => {
            // The three `L0001` rules -- #795 (PEP 654)'s `except*` ban,
            // PEP 765's `finally` ban, and D-148's pre-existing "no
            // enclosing loop" check -- live in `break_context_violation`,
            // shared with #905's `TYPE_CHECKING` walker. Their precedence
            // is CPython's: the `except*` error is fatal regardless of any
            // enclosing loop (hence no `in_loop` conjunct, and hence
            // `ExceptStarCtx::InsideLoopShielded`), while the `finally`
            // restriction only applies once a valid loop target genuinely
            // exists for the `break` to escape to; with no loop at all
            // CPython's fatal error is `'break' outside loop`. A loop
            // defined *inside* the `finally` has already reset `in_finally`
            // to `false` by the time this arm runs.
            //
            // `None` means valid Python that this compiler simply does not
            // implement yet -- a `break` with a real enclosing loop -- which
            // is a `C0001` capability gap, not a context violation.
            return Err(
                match break_context_violation(in_loop, in_finally, except_star) {
                    Some(message) => context_invalid(message, pycc_ast::stmt_range(stmt)),
                    None => unsupported(
                        "statement kind not supported yet: `break` inside a loop",
                        pycc_ast::stmt_range(stmt),
                    ),
                },
            );
        }
        Stmt::Continue(_) => {
            // See the `Stmt::Break` arm above -- identical rationale,
            // identical CPython precedence, identical shared predicate.
            return Err(
                match continue_context_violation(in_loop, in_finally, except_star) {
                    Some(message) => context_invalid(message, pycc_ast::stmt_range(stmt)),
                    None => unsupported(
                        "statement kind not supported yet: `continue` inside a loop",
                        pycc_ast::stmt_range(stmt),
                    ),
                },
            );
        }
        Stmt::Match(match_stmt) => lower_match(
            match_stmt,
            aliases,
            in_loop,
            in_function,
            in_finally,
            except_star,
            class_name,
            type_param,
            class_defs,
            imports,
            signatures,
        )?,
        Stmt::Try(try_stmt) => {
            let body = lower_body(
                &try_stmt.body,
                aliases,
                in_loop,
                in_function,
                in_finally,
                except_star,
                class_name,
                type_param,
                class_defs,
                imports,
                signatures,
            )?;
            // #795 (PEP 654): an `except*` clause body is the only thing
            // that *sets* the context; a plain `except` clause propagates
            // whatever was already in scope (an ordinary `try`/`except`
            // nested inside an `except*` clause body does not shield its
            // contents). `try_stmt.is_star` is only in scope here, so the
            // caller derives the value rather than `lower_except_handler`.
            let handler_except_star = if try_stmt.is_star {
                ExceptStarCtx::InsideUnshielded
            } else {
                except_star
            };
            let handlers = try_stmt
                .handlers
                .iter()
                .map(|h| {
                    // Part 3 of #382 (#542, PEP 654): `except*` requires
                    // every clause to name a type. Unlike PEP 758's
                    // "at least one name in a tuple" rule (checked in
                    // `lower_except_handler`, since ruff's parser accepts an
                    // empty `except ():`), a completely typeless
                    // `except*:` is rejected by ruff's own parser as a
                    // syntax error before this lowering pass ever runs (see
                    // `lower_try_star_bare_except_star_is_rejected_at_parse_time`),
                    // so no defensive re-check belongs here -- one would be
                    // unreachable and untestable under this repository's
                    // 100%-region coverage gate.
                    lower_except_handler(
                        h,
                        aliases,
                        in_loop,
                        in_function,
                        in_finally,
                        handler_except_star,
                        class_name,
                        type_param,
                        class_defs,
                        imports,
                        signatures,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let orelse = lower_body(
                &try_stmt.orelse,
                aliases,
                in_loop,
                in_function,
                in_finally,
                except_star,
                class_name,
                type_param,
                class_defs,
                imports,
                signatures,
            )?;
            // Entering a `finally` clause always sets `in_finally` to `true`
            // for its own body -- unconditionally, regardless of the
            // incoming value -- since a `return`/`break`/`continue` directly
            // inside it always escapes this `finally` (a nested `finally`
            // inside an outer `finally` is still a violation of its own).
            let finalbody = lower_body(
                &try_stmt.finalbody,
                aliases,
                in_loop,
                in_function,
                true,
                // #795 (PEP 654): unlike `in_finally`, the `except*` context
                // is *propagated* into a `finally`, never cleared -- CPython
                // rejects a `return` inside a `finally` that is itself nested
                // in an `except*` clause body. A try-star's own `finalbody`
                // is not inside its own handlers, so the incoming value is
                // already `Outside` there and that case stays accepted.
                except_star,
                class_name,
                type_param,
                class_defs,
                imports,
                signatures,
            )?;
            if try_stmt.is_star {
                HirStmt::TryStar {
                    body,
                    handlers,
                    orelse,
                    finalbody,
                }
            } else {
                HirStmt::Try {
                    body,
                    handlers,
                    orelse,
                    finalbody,
                }
            }
        }
        Stmt::Raise(raise_stmt) => {
            let exc = raise_stmt
                .exc
                .as_deref()
                .map(|e| lower_expr(e, in_function, class_name, imports, signatures))
                .transpose()?;
            // PEP 409: `raise X from None` suppresses the implicit
            // `__context__` chain. Its only observable effect in CPython is
            // traceback rendering, and pycc neither populates `__context__`
            // nor emits traceback frames yet, so a `None` cause is recorded
            // as "no cause" rather than lowered as an expression -- `HirExpr`
            // has no `None` variant to lower it into, and adding one solely
            // for a value nothing downstream can observe would put an
            // unreachable arm in every exhaustive `HirExpr` match in the
            // workspace. This deliberately collapses `raise X from None` and
            // `raise X` into the same HIR; the distinction has to be
            // reintroduced here when implicit `__context__` chaining lands.
            let cause = match raise_stmt.cause.as_deref() {
                None | Some(Expr::NoneLiteral(_)) => None,
                Some(cause) => Some(lower_expr(
                    cause,
                    in_function,
                    class_name,
                    imports,
                    signatures,
                )?),
            };
            HirStmt::Raise { exc, cause }
        }
        // A CPython-backed `import` (#1291) or `from ... import` (#1383)
        // nested in a module-level `if`/`try` block, whose bindings `module::lower_top_level_item`
        // recorded before lowering the block.
        other => match crate::import::nested_foreign_import(other, imports) {
            Some(lowered) if !in_function => lowered,
            _ => return Err(unsupported::unsupported_statement(other, in_function)),
        },
    };
    // PEP 572 (#774): reject a walrus lowered anywhere other than the three
    // placements the issue permits -- an `if`/`while` test (that arm's own
    // `HirExpr::NamedExpr` node is left exactly as `lower_expr` produced
    // it, whatever nesting it carries) or a bare expression statement.
    // Every other statement kind's own *immediate* expression fields are
    // walked with `contains_named_expr`; a nested `Vec<HirStmt>` body
    // (`body`/`orelse`/`handlers`/`finalbody`) is deliberately not
    // re-walked here, because each of its own statements already ran
    // through this exact same check via its own recursive `lower_stmt`
    // call -- re-walking it here would be redundant, and for `Try`'s
    // `handlers: Vec<HirExceptHandler>` (whose only fields are bare
    // exception-type-name strings, an optional `as` binding name, and its
    // own `body`) there is no expression to walk in the first place.
    //
    // The three permitted placements are folded into this same exhaustive
    // match as `false` arms (never a violation there) rather than guarded
    // by a separate outer `if`, so this stays a single match with no
    // structurally-unreachable arm left for the 100%-region coverage gate
    // to demand a test for (an earlier revision guarded the match with
    // `if !matches!(lowered, If | While | ExprStmt)` and gave those three
    // variants an `unreachable!()` arm inside it -- correct, but provably
    // untestable, since the outer guard makes the compiler's own
    // exhaustiveness requirement the only reason that arm exists).
    //
    // `HirStmt::ForList` joins this same never-a-violation group rather than
    // getting its own arm: unlike every other variant matched below,
    // `lowered` can never actually be a `ForList` here in the first place --
    // the `Stmt::For` arm above returns a `ForList` directly (see its own
    // `return Ok(HirStmt::ForList { .. })`) before this match is ever
    // reached, so a standalone `HirStmt::ForList { .. } => false` arm is
    // dead code no test can reach without faking the earlier control flow.
    // Its own reasoning would still hold if it *were* reachable -- `list` is
    // a bare variable name (D-105), not an expression, so there would be
    // nothing here for a walrus to be nested inside -- folding it into this
    // arm keeps that reasoning on record without leaving an unreachable arm
    // of its own for the coverage gate to flag.
    //
    // `HirStmt::ForObject` (PR 3c of #1082) gets a real arm instead: its
    // `iter` field is an expression that can nest a walrus, and unlike
    // `ForList`'s bare list name the arm is genuinely reached --
    // `ForRange`'s own arm below proves the `Stmt::For` result reaches this
    // match. Leaving it in the never-a-violation group admitted a walrus
    // that `killed_names` does not see: that function's `ForObject` arm
    // records the loop target and the body but not the iterable, so a kill
    // written there survived `apply_kill_prescan` and a read at the top of
    // a re-enterable narrowed region was checked against the pre-kill
    // narrowing. Refusing the placement here keeps the iterable exactly
    // where `ForRange`'s `range(...)` arguments already are, so no shape
    // reaches the prescan with a kill it cannot report.
    let violates_walrus_placement = match &lowered {
        HirStmt::If { .. }
        | HirStmt::While { .. }
        | HirStmt::ExprStmt(_)
        | HirStmt::ForList { .. } => false,
        HirStmt::ForObject { iter, .. } => contains_named_expr(iter),
        HirStmt::Assign { value, .. } => contains_named_expr(value),
        HirStmt::AnnAssign { value, .. } => value.as_ref().is_some_and(contains_named_expr),
        HirStmt::ForRange {
            start, stop, step, ..
        } => contains_named_expr(start) || contains_named_expr(stop) || contains_named_expr(step),
        HirStmt::DictSet { key, value, .. } => {
            contains_named_expr(key) || contains_named_expr(value)
        }
        HirStmt::ListCompAssign {
            iter, cond, elt, ..
        } => {
            comp_iter_contains_named_expr(iter)
                || cond.as_deref().is_some_and(contains_named_expr)
                || contains_named_expr(elt)
        }
        HirStmt::DictCompAssign {
            iter,
            cond,
            key,
            value,
            ..
        } => {
            comp_iter_contains_named_expr(iter)
                || cond.as_deref().is_some_and(contains_named_expr)
                || contains_named_expr(key)
                || contains_named_expr(value)
        }
        HirStmt::SetCompAssign {
            iter, cond, elt, ..
        } => {
            comp_iter_contains_named_expr(iter)
                || cond.as_deref().is_some_and(contains_named_expr)
                || contains_named_expr(elt)
        }
        HirStmt::Return(value) => value.as_ref().is_some_and(contains_named_expr),
        HirStmt::AttrSet { base, value, .. } => {
            contains_named_expr(base) || contains_named_expr(value)
        }
        HirStmt::Match { subject, .. } => contains_named_expr(subject),
        // `handlers`' own only expression-shaped content is each handler's
        // own `body: Vec<HirStmt>`, already independently checked (see this
        // block's own doc comment above).
        // `del` never reaches here (`lower_stmt_expanded` lowers it, and
        // `stmt::del::lower_delete` refuses a walrus in a slice target).
        // A nested foreign import (#1291) holds only names.
        HirStmt::Try { .. }
        | HirStmt::TryStar { .. }
        | HirStmt::Delete { .. }
        | HirStmt::DeleteSlice { .. }
        | HirStmt::DeleteAttr { .. }
        | HirStmt::ForeignImport { .. } => false,
        HirStmt::Raise { exc, cause } => {
            exc.as_ref().is_some_and(contains_named_expr)
                || cause.as_ref().is_some_and(contains_named_expr)
        }
    };
    if violates_walrus_placement {
        return Err(unsupported(
            WALRUS_PLACEMENT_MESSAGE,
            pycc_ast::stmt_range(stmt),
        ));
    }
    Ok(lowered)
}

/// PEP 572 (#774): the one wording of the walrus-placement refusal, shared
/// by `lower_stmt` and `del::lower_delete` (a walrus in a slice `del`
/// target, Part 2c of #1371).
pub(crate) const WALRUS_PLACEMENT_MESSAGE: &str = "a walrus assignment (`:=`) is only supported in an `if`/`while` \
     condition or as a bare expression statement (#774)";

/// PEP 572 (#774): `CompIter`'s own `contains_named_expr` counterpart --
/// `CompIter::Range`'s `start`/`stop`/`step` are ordinary `HirExpr` fields
/// (e.g. `[x for x in range(n := 5)]`); `CompIter::Name` is a bare variable
/// name with nothing to walk. A walrus found here is rejected for the same
/// reason every other comprehension field is: #774's own explicit
/// permitted-scope-cut leaves comprehension-embedded walrus scoping
/// unimplemented, so this is a `core` gap recorded in the
/// conformance-breadth manifest, not a silent one.
fn comp_iter_contains_named_expr(iter: &CompIter) -> bool {
    match iter {
        CompIter::Range { start, stop, step } => {
            contains_named_expr(start) || contains_named_expr(stop) || contains_named_expr(step)
        }
        CompIter::Iterable(iterable) => contains_named_expr(iterable),
        CompIter::Name(_) => false,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_body(
    body: &[Stmt],
    aliases: &[(String, Ty)],
    in_loop: bool,
    in_function: bool,
    in_finally: bool,
    except_star: ExceptStarCtx,
    class_name: Option<&str>,
    type_param: Option<&str>,
    class_defs: &[ClassAnnotationInfo],
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<Vec<HirStmt>, Diagnostic> {
    // #435: `Stmt::Pass` is a no-op — filter it out rather than lowering it
    // to a statement. This allows method bodies like `def __init_subclass__:
    // pass` and `def __set_name__: pass` to compile, which is required for
    // PEP 487/PEP 487 hook recognition. A body consisting solely of `pass`
    // produces an empty `Vec<HirStmt>`, which is a valid no-op body.
    let mut lowered = Vec::with_capacity(body.len());
    for stmt in body.iter().filter(|stmt| !matches!(stmt, Stmt::Pass(_))) {
        lowered.extend(lower_stmt_expanded(
            stmt,
            aliases,
            in_loop,
            in_function,
            in_finally,
            except_star,
            class_name,
            type_param,
            class_defs,
            imports,
            signatures,
        )?);
    }
    Ok(lowered)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_elif_else_clauses(
    clauses: &[ElifElseClause],
    aliases: &[(String, Ty)],
    in_loop: bool,
    in_function: bool,
    in_finally: bool,
    except_star: ExceptStarCtx,
    class_name: Option<&str>,
    type_param: Option<&str>,
    class_defs: &[ClassAnnotationInfo],
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<Vec<HirStmt>, Diagnostic> {
    let Some((first, rest)) = clauses.split_first() else {
        return Ok(vec![]);
    };
    match &first.test {
        // #790: `elif TYPE_CHECKING:` gets the same constant-fold as a
        // leading `if TYPE_CHECKING:` (see `lower_stmt`'s `Stmt::If` arm's
        // own doc comment) -- the guarded body is dead at runtime either
        // way, and CPython's `elif` is just sugar for a nested `if` inside
        // the enclosing `else`.
        Some(test) if is_type_checking_guard(test, imports) => {
            // #905: and the same context re-check as the leading
            // `if TYPE_CHECKING:` fold in `lower_stmt`.
            check_guarded_body(
                &first.body,
                in_loop,
                in_function,
                in_finally,
                except_star,
                class_name,
                imports,
                signatures,
            )?;
            Ok(vec![HirStmt::If {
                test: HirExpr::BoolLiteral(false),
                body: vec![],
                orelse: lower_elif_else_clauses(
                    rest,
                    aliases,
                    in_loop,
                    in_function,
                    in_finally,
                    except_star,
                    class_name,
                    type_param,
                    class_defs,
                    imports,
                    signatures,
                )?,
            }])
        }
        Some(test) => Ok(vec![HirStmt::If {
            test: lower_condition(test, in_function, class_name, imports, signatures)?,
            body: lower_body(
                &first.body,
                aliases,
                in_loop,
                in_function,
                in_finally,
                except_star,
                class_name,
                type_param,
                class_defs,
                imports,
                signatures,
            )?,
            orelse: lower_elif_else_clauses(
                rest,
                aliases,
                in_loop,
                in_function,
                in_finally,
                except_star,
                class_name,
                type_param,
                class_defs,
                imports,
                signatures,
            )?,
        }]),
        None => {
            assert!(
                rest.is_empty(),
                "pycc_hir: an else clause must be the last elif_else_clause"
            );
            lower_body(
                &first.body,
                aliases,
                in_loop,
                in_function,
                in_finally,
                except_star,
                class_name,
                type_param,
                class_defs,
                imports,
                signatures,
            )
        }
    }
}

#[cfg(test)]
mod tests;
