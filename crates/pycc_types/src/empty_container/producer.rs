//! The producer scan of D-245's source 3, extracted from `super`
//! (`empty_container.rs`, then 1,112 lines) per AGENTS.md's decomposition
//! rule when #1265 widened it to an attribute target.

use super::{Resolution, collect_binding_sites, nested_bodies, scoped_for_body};
use crate::Environment;
use pycc_hir::{ContainerReceiver, HirExpr, HirStmt, Ty};

/// The first producer use of `target` anywhere in `body`, in source order.
///
/// Scanning the whole function body from its start -- rather than only
/// forward from the assignment being resolved -- is what implements the
/// **first-wins-within-a-scope** rule for a name assigned `[]` in more than
/// one branch: `if c: xs = []; xs.append(1)` / `else: xs = [];
/// xs.append("a")` resolves *both* nodes from the first producer, and the
/// second branch's `append` then fails with the ordinary element-type
/// mismatch. A name-keyed resolution cannot represent two different types
/// for one binding, and neither can the binding itself.
///
/// A producer is recognized only in **statement position** -- a bare
/// `xs.append(v)` or `d[k] = v`. `HirExpr::ListAppend` is also a valid value
/// expression (`y = xs.append(v)` binds `None`), and such an occurrence is
/// *not* a producer here, so `xs = []` followed only by `y = xs.append(1)`
/// reports `T0003`. That restriction is deliberate and matches the rewrite
/// side: `rewrite_body` and `body_has_empty_literal` likewise visit direct
/// assignment values and block bodies, never nested *expression* positions, so
/// the whole pass has one statable shape. The boundary is between statement
/// and expression nesting, not between one block form and another: every block
/// form is walked, via the shared [`nested_bodies`] inventory. See D-245
/// item 8.
///
/// The scan returns the first *syntactic* producer occurrence for the name,
/// not the first shape-compatible one: a `ListAppend` on a name later used as
/// a dict ends the scan with a `Resolution::List`. `Resolution::matches`
/// discards a resolution of the wrong shape at the rewrite site, so a
/// cross-shape hit costs a missed resolution (`T0003`) and never yields a
/// wrong element type. Such a program fails type-checking on its own terms
/// anyway. The same is true of a producer whose value does not infer: the
/// scan ends there with a miss rather than continuing to a later producer,
/// because a later producer's element type is not the one the program's
/// first use asks for, and selecting it would be a wrong resolution rather
/// than a missed one. See [`ProducerScan`].
pub(super) fn find_producer(
    body: &[HirStmt],
    target: &str,
    env: &Environment,
    local_names: &[&str],
) -> Option<Resolution> {
    match scan_body(body, ProducerTarget::Name(target), env, local_names) {
        ProducerScan::Resolved(resolution) => {
            Some(widen_numeric(resolution, body, target, env, local_names))
        }
        ProducerScan::Matched | ProducerScan::NotFound => None,
    }
}

/// A numeric element (or dict value) type the first producer gave, widened
/// to `int | float` when another producer stores the other kind of number
/// (`out.append(0)` ... `out.append(best)` with a `float` best): one
/// `int | float` element holds both, where the first producer's type alone
/// would refuse the later store.
fn widen_numeric(
    resolution: Resolution,
    body: &[HirStmt],
    target: &str,
    env: &Environment,
    local_names: &[&str],
) -> Resolution {
    let numeric = |ty: &Ty| matches!(ty, Ty::Int | Ty::Bool | Ty::Float | Ty::Num);
    let first = match &resolution {
        Resolution::List(element) | Resolution::Dict(_, element) if numeric(element) => {
            element.clone()
        }
        _ => return resolution,
    };
    let mut stored = Vec::new();
    collect_stored_values(body, target, env, local_names, &mut stored);
    let floating = |ty: &Ty| matches!(ty, Ty::Float | Ty::Num);
    let mixed = stored.iter().all(numeric)
        && (stored.iter().any(floating) || floating(&first))
        && (stored.iter().any(|ty| !floating(ty)) || !floating(&first));
    if !mixed {
        return resolution;
    }
    match resolution {
        Resolution::List(_) => Resolution::List(Ty::Num),
        Resolution::Dict(key, _) => Resolution::Dict(key, Ty::Num),
    }
}

/// The types of the values every producer of `target` in `body` stores
/// (those that infer).
fn collect_stored_values(
    body: &[HirStmt],
    target: &str,
    env: &Environment,
    local_names: &[&str],
    out: &mut Vec<Ty>,
) {
    for stmt in body {
        // Issue #1188's receiver-dispatched `xs.append(v)` (an `--ext`
        // module's) stores too.
        let dispatched = match stmt {
            HirStmt::ExprStmt(HirExpr::ReceiverDispatchedCall {
                call,
                container: pycc_hir::ContainerFallback::Admitted,
            }) => call.container_form(),
            _ => None,
        };
        let dispatched_value = match &dispatched {
            Some(HirExpr::ListAppend { list, value })
                if receiver_matches(list, ProducerTarget::Name(target)) =>
            {
                Some(&**value)
            }
            _ => None,
        };
        let value = match stmt {
            _ if dispatched_value.is_some() => dispatched_value,
            HirStmt::ExprStmt(HirExpr::ListAppend { list, value })
                if receiver_matches(list, ProducerTarget::Name(target)) =>
            {
                Some(&**value)
            }
            HirStmt::DictSet { dict, value, .. } if dict == target => Some(value),
            _ => None,
        };
        if let Some(value) = value
            && let Ok(ty) = crate::infer_expr_in(env, local_names, value)
        {
            out.push(ty);
        }
        for nested in nested_bodies(stmt) {
            collect_stored_values(nested, target, env, local_names, out);
        }
    }
}

/// What a producer scan looks for: a local name (`xs.append(v)`,
/// `d[k] = v`), or -- #1265 -- an instance attribute on the method's own
/// receiver (`self.xs.append(v)`). `receivers` is every spelling the
/// receiver has in the scanned body: the canonical `self`, plus the source
/// spelling a renamed receiver is aliased under (#1181; see
/// `attr_slot::receiver_spellings`).
#[derive(Clone, Copy)]
pub(super) enum ProducerTarget<'a> {
    Name(&'a str),
    SelfAttr {
        receivers: &'a [&'a str],
        attr: &'a str,
    },
}

/// Whether a container method's `receiver` is `target`. The one predicate
/// both `append` arms of [`scan_for_producer`] share -- the direct
/// `HirExpr::ListAppend` and #1188's dispatched form -- so the two can never
/// disagree about which receivers count.
fn receiver_matches(receiver: &ContainerReceiver, target: ProducerTarget<'_>) -> bool {
    match (receiver, target) {
        (ContainerReceiver::Name(name), ProducerTarget::Name(wanted)) => name == wanted,
        (ContainerReceiver::Attr(read), ProducerTarget::SelfAttr { receivers, attr }) => {
            matches!(read.as_ref(), HirExpr::AttrGet { base, attr: read_attr }
                if read_attr == attr
                    && matches!(base.as_ref(), HirExpr::Name(base) if receivers.contains(&base.as_str())))
        }
        _ => false,
    }
}

/// The three-outcome scan of one function body for `target`'s first
/// producer. [`find_producer`] collapses it to an `Option`; #1265's class
/// phase keeps the `Matched` outcome, because the first-syntactic-producer
/// rule carries across method boundaries: a `Matched` in one method ends the
/// whole class's scan.
pub(super) fn scan_body(
    body: &[HirStmt],
    target: ProducerTarget<'_>,
    env: &Environment,
    local_names: &[&str],
) -> ProducerScan {
    let mut sites: Vec<&str> = Vec::new();
    collect_binding_sites(body, &mut sites);
    let mut aliases: Vec<&str> = Vec::new();
    if let ProducerTarget::SelfAttr { receivers, attr } = target {
        collect_attr_aliases(body, receivers, attr, &mut aliases);
    }
    scan_for_producer(body, target, env, local_names, &sites, &aliases)
}

/// Every desugaring temporary bound to the receiver's `attr` in `body` at
/// any depth: `self.d[k] = v` reaches HIR as `_pycc_N = self.d; _pycc_N[k]
/// = v` (`pycc_ast`'s computed-base subscript store). A `_pycc_` name is
/// bound exactly once, so a store through it is a store into the attribute.
fn collect_attr_aliases<'a>(
    body: &'a [HirStmt],
    receivers: &[&str],
    attr: &str,
    aliases: &mut Vec<&'a str>,
) {
    for stmt in body {
        if let HirStmt::Assign {
            target,
            value: HirExpr::AttrGet { base, attr: read },
        } = stmt
            && target.starts_with("_pycc_")
            && read == attr
            && matches!(base.as_ref(), HirExpr::Name(base) if receivers.contains(&base.as_str()))
        {
            aliases.push(target);
        }
        for nested in nested_bodies(stmt) {
            collect_attr_aliases(nested, receivers, attr, aliases);
        }
    }
}

/// Whether `dict` is the dict `target` names: the local itself, or for an
/// attribute target one of its desugaring aliases.
fn dict_matches(dict: &str, target: ProducerTarget<'_>, aliases: &[&str]) -> bool {
    match target {
        ProducerTarget::Name(wanted) => dict == wanted,
        ProducerTarget::SelfAttr { .. } => aliases.contains(&dict),
    }
}

/// The outcome of scanning one statement list for `target`'s first producer.
///
/// The middle variant is what makes the scan stop at the *first syntactic*
/// producer rather than the first *inferring* one: a producer whose value
/// fails to infer -- because it reads a name the flat whole-function
/// environment never bound, such as one assigned inside a `match` case body --
/// ends the scan with a miss instead of falling through to a later producer
/// that might carry an entirely different element type.
pub(super) enum ProducerScan {
    /// No statement in this list, or in any body nested inside it, names
    /// `target` in producer position.
    NotFound,
    /// A producer for `target` was found, but its element type could not be
    /// inferred. The scan is over; the caller resolves nothing.
    Matched,
    /// A producer for `target` was found and its element type inferred.
    Resolved(Resolution),
}

fn scan_for_producer<'a>(
    body: &'a [HirStmt],
    target: ProducerTarget<'_>,
    env: &Environment,
    local_names: &[&str],
    sites: &[&'a str],
    aliases: &[&str],
) -> ProducerScan {
    for stmt in body {
        // Issue #1188: an admitted container reading of a receiver-dispatched
        // `target.append(v)` is a producer too. `target` is bound to an empty
        // list literal, so the container reading is the only one it can take.
        let dispatched = match stmt {
            HirStmt::ExprStmt(HirExpr::ReceiverDispatchedCall {
                call,
                container: pycc_hir::ContainerFallback::Admitted,
            }) => call.container_form(),
            _ => None,
        };
        if let Some(HirExpr::ListAppend { list, value }) = &dispatched
            && receiver_matches(list, target)
        {
            return match crate::infer_expr_in(env, local_names, value) {
                Ok(element) => ProducerScan::Resolved(Resolution::List(element)),
                Err(_) => ProducerScan::Matched,
            };
        }
        match stmt {
            // #1263 added the attribute receiver (`self.xs.append(v)`),
            // which is never a producer for a local name; #1265 makes it the
            // producer for a `ProducerTarget::SelfAttr`.
            HirStmt::ExprStmt(HirExpr::ListAppend { list, value })
                if receiver_matches(list, target) =>
            {
                return match crate::infer_expr_in(env, local_names, value) {
                    Ok(element) => ProducerScan::Resolved(Resolution::List(element)),
                    Err(_) => ProducerScan::Matched,
                };
            }
            // A dict producer is `d[k] = v` on the local, or a store through
            // one of an attribute's desugaring aliases (`self.d[k] = v`).
            // An augmented update `d[k] += v` reads the element it stores,
            // so its value's type is the element type itself: it is a
            // consumer, and the scan moves on to the next producer (the
            // `d[k] = 1` of `try: d[k] += 1 except KeyError: d[k] = 1`).
            HirStmt::DictSet { dict, value, .. }
                if dict_matches(dict, target, aliases) && reads_own_element(value, dict) => {}
            HirStmt::DictSet { dict, key, value } if dict_matches(dict, target, aliases) => {
                return match (
                    crate::infer_expr_in(env, local_names, key),
                    crate::infer_expr_in(env, local_names, value),
                ) {
                    (Ok(key_ty), Ok(value_ty)) => {
                        ProducerScan::Resolved(Resolution::Dict(key_ty, value_ty))
                    }
                    _ => ProducerScan::Matched,
                };
            }
            _ => {}
        }
        for nested in nested_bodies(stmt) {
            let scoped = scoped_for_body(stmt, nested, env, sites);
            let inner = scoped.as_ref().unwrap_or(env);
            match scan_for_producer(nested, target, inner, local_names, sites, aliases) {
                ProducerScan::NotFound => {}
                outcome => return outcome,
            }
        }
    }
    ProducerScan::NotFound
}

/// Whether `value` is the lowered value of an augmented update of `dict`'s
/// own element (`d[k] op= v` becomes `d[k] = d[k] op v`).
fn reads_own_element(value: &HirExpr, dict: &str) -> bool {
    matches!(value, HirExpr::AugBinOp { left, .. }
        if matches!(left.as_ref(), HirExpr::Subscript { base, .. }
            if matches!(base.as_ref(), HirExpr::Name(name) if name == dict)))
}
