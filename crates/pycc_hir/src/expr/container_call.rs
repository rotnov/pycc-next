//! The container-method call fast paths (`.append()`/`.pop()`/`.get()`/
//! `.add()`) of `lower_expr`'s `Expr::Call`-over-`Expr::Attribute` branch,
//! extracted from `expr.rs` per AGENTS.md's file-decomposition rule (issue
//! #890; tracking issue #552). Each body is the original `if` block's,
//! unchanged; only the dispatch on the attribute name moved into
//! `lower_container_method_call`.
//!
//! These run with no type information available (HIR lowering precedes
//! `pycc_types`): they recognize the *syntactic* shape `name.method(...)`
//! and cannot tell a real `list`/`dict`/`set` receiver from a class
//! instance whose own method shares one of the four names. In a module from
//! which such a class is reachable (issue #1188), or which can hold a
//! CPython object whose method may share one of the names (issue #1095),
//! `receiver_dispatch` therefore keeps both readings in a
//! `HirExpr::ReceiverDispatchedCall` and lets the receiver's static type
//! choose; everywhere else these fast paths run exactly as they always have.
//!
//! The receiver of `.append()`/`.pop()`/`.get()` is a bare name (D-105
//! point 3) or, since #1263 (Part 2 of #1218), an attribute read such as
//! `self.xs` -- see [`lower_container_receiver`]. `.add()` stays bare-name
//! only: no `set` instance slot exists yet (#1262 admits `list[int]` and
//! `dict[str, int]` slots only).

use super::lower_expr;
use crate::expr::keyword_bind::SignatureTable;
use crate::int_boundary::check_boundary_literal;
use crate::{ContainerReceiver, HirExpr, ImportBinding, unsupported};
use pycc_ast::Expr;
use pycc_diag::Diagnostic;

/// Lowers `call` when `attr` names one of the four hand-recognized
/// container methods; `None` means "not a container method", and the
/// caller falls through to the stdlib-intrinsic / instance-method paths
/// unchanged.
pub(super) fn lower_container_method_call(
    call: &pycc_ast::ExprCall,
    attr: &pycc_ast::ExprAttribute,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Option<Result<HirExpr, Diagnostic>> {
    match attr.attr.as_str() {
        "append" => Some(lower_list_append(
            call,
            attr,
            in_function,
            class_name,
            imports,
            signatures,
        )),
        "pop" => Some(lower_list_pop(
            call,
            attr,
            in_function,
            class_name,
            imports,
            signatures,
        )),
        "get" => Some(lower_dict_get(
            call,
            attr,
            in_function,
            class_name,
            imports,
            signatures,
        )),
        "add" => Some(lower_set_add(
            call,
            attr,
            in_function,
            class_name,
            imports,
            signatures,
        )),
        _ => None,
    }
}

/// Lowers the receiver of `.append()`/`.pop()`/`.get()` (#1263): a bare
/// name keeps D-105's `Name` shape; anything else is lowered generically
/// and admitted only when it is an attribute read (`HirExpr::AttrGet`,
/// including a chain through a `@property`). A call, a subscript, and a
/// `pycc_std`-resolved module constant such as `math.pi` (which lowers to
/// `HirExpr::Name("math.pi")`, not `AttrGet`) keep a `C0001` refusal whose
/// wording names both accepted forms. An attribute of a foreign module
/// (`sys.argv`) *is* an `AttrGet` and is admitted here; `pycc_types`
/// refuses it as `I0404` because its type is `object`.
fn lower_container_receiver(
    attr: &pycc_ast::ExprAttribute,
    refusal: &str,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<ContainerReceiver, Diagnostic> {
    if let Expr::Name(name) = attr.value.as_ref() {
        return Ok(ContainerReceiver::Name(name.id.as_str().to_string()));
    }
    let receiver = lower_expr(&attr.value, in_function, class_name, imports, signatures)?;
    if !matches!(receiver, HirExpr::AttrGet { .. }) {
        return Err(unsupported(refusal, pycc_ast::expr_range(&attr.value)));
    }
    Ok(ContainerReceiver::Attr(Box::new(receiver)))
}

fn lower_list_append(
    call: &pycc_ast::ExprCall,
    attr: &pycc_ast::ExprAttribute,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirExpr, Diagnostic> {
    let list = lower_container_receiver(
        attr,
        "`.append()` is only supported on a name or an instance attribute so far",
        in_function,
        class_name,
        imports,
        signatures,
    )?;
    let [value] = &*call.arguments.args else {
        return Err(unsupported(
            format!(
                "list.append() takes exactly one argument, got {}",
                call.arguments.args.len()
            ),
            call.range,
        ));
    };
    let value_span = pycc_ast::expr_range(value);
    let value = lower_expr(value, in_function, class_name, imports, signatures)?;
    check_boundary_literal(&value, value_span, "`list.append()` value")?;
    Ok(HirExpr::ListAppend {
        list,
        value: Box::new(value),
    })
}

fn lower_list_pop(
    call: &pycc_ast::ExprCall,
    attr: &pycc_ast::ExprAttribute,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirExpr, Diagnostic> {
    let list = lower_container_receiver(
        attr,
        "`.pop()` is only supported on a name or an instance attribute so far",
        in_function,
        class_name,
        imports,
        signatures,
    )?;
    let [] = &*call.arguments.args else {
        return Err(unsupported(
            format!(
                "list.pop() takes no arguments, got {}",
                call.arguments.args.len()
            ),
            call.range,
        ));
    };
    Ok(HirExpr::ListPop { list })
}

fn lower_dict_get(
    call: &pycc_ast::ExprCall,
    attr: &pycc_ast::ExprAttribute,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirExpr, Diagnostic> {
    let dict = lower_container_receiver(
        attr,
        "`.get()` is only supported on a name or an instance attribute so far",
        in_function,
        class_name,
        imports,
        signatures,
    )?;
    let [key, default] = &*call.arguments.args else {
        // Issue #890: this fast path cannot see the receiver's type, so
        // the message must not assert one. The receiver may be a real
        // dict (`x = {"a": 1}; x.get("a")`) or something else entirely
        // (`v.get()` on an `int`, a `ContextVar`); a non-dict receiver's
        // own `.get()` is unsupported and is reported by `pycc_types`'s
        // `T0033` only for the two-argument shape, because every other
        // arity is rejected here first. The wording is receiver-neutral
        // on purpose.
        return Err(unsupported(
            format!(
                "`.get()` is only supported as `dict.get(key, default)` with exactly two arguments so far, got {}",
                call.arguments.args.len()
            ),
            call.range,
        ));
    };
    let default_span = pycc_ast::expr_range(default);
    let key = lower_expr(key, in_function, class_name, imports, signatures)?;
    let default = lower_expr(default, in_function, class_name, imports, signatures)?;
    check_boundary_literal(&default, default_span, "`dict.get()` default")?;
    Ok(HirExpr::DictGetOrDefault {
        dict,
        key: Box::new(key),
        default: Box::new(default),
    })
}

fn lower_set_add(
    call: &pycc_ast::ExprCall,
    attr: &pycc_ast::ExprAttribute,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirExpr, Diagnostic> {
    let Expr::Name(set_name) = attr.value.as_ref() else {
        return Err(unsupported(
            "`.add()` is only supported on a bare-name set so far",
            pycc_ast::expr_range(&attr.value),
        ));
    };
    let [value] = &*call.arguments.args else {
        return Err(unsupported(
            format!(
                "set.add() takes exactly one argument, got {}",
                call.arguments.args.len()
            ),
            call.range,
        ));
    };
    let value_span = pycc_ast::expr_range(value);
    let value = lower_expr(value, in_function, class_name, imports, signatures)?;
    check_boundary_literal(&value, value_span, "`set.add()` value")?;
    Ok(HirExpr::SetAdd {
        set: set_name.id.as_str().to_string(),
        value: Box::new(value),
    })
}
