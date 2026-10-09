//! Protocol class lowering (#380, PR-20), extracted from `class.rs` per
//! AGENTS.md's file-decomposition rule (issue #890; tracking issue #548),
//! following `class/mro.rs`'s precedent. The body was the original
//! `lower_protocol_class`'s, unchanged, until
//! [#984](https://github.com/rotnov/pycc/issues/984) added the one
//! reserved-name call below: a `Protocol` body never reaches
//! `super::body`'s method loop, because `super::lower_class` returns through
//! this function first, so `def __slots__` needed its own guard here.

use super::reserved_names::reject_reserved_protocol_method_name;
use super::{ClassAnnotationInfo, HirClassDef, ProtocolMember, is_declaration_body};
use crate::{HirItem, Ty, unsupported};
use pycc_ast::{Expr, Stmt};
use pycc_diag::Diagnostic;

/// #380 (PR-20): Lowers a protocol class body. Protocol methods (with
/// `...` or `pass` bodies) are recorded as `ProtocolMember::Method` and
/// are NOT lowered to `HirItem::Function`s. Protocol annotated assignments
/// (`x: int`) are recorded as `ProtocolMember::Attribute`. Protocol
/// members from base protocols are inherited. A protocol method with an
/// implementation body is rejected with `C0001`. A protocol `__init__` is
/// rejected with `C0001`.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_protocol_class(
    def: &pycc_ast::StmtClassDef,
    class_name: String,
    bases: Vec<String>,
    mro: Vec<String>,
    type_param: Option<String>,
    runtime_checkable: bool,
    defined_classes: &[(String, HirClassDef)],
    class_name_defs: &[ClassAnnotationInfo],
) -> Result<(HirClassDef, Vec<HirItem>), Diagnostic> {
    let mut protocol_members: Vec<ProtocolMember> = Vec::new();
    // Inherit protocol members from base protocols.
    for base_name in &bases {
        // This should have been validated already in `lower_class`.
        // Using `.expect()` (whose panic path lives in libcore, outside
        // this crate's instrumented regions) instead of a
        // `let .. else { panic!() }` avoids a permanently-uncovered
        // branch under D-014's 100%-region coverage gate.
        let base_def = &defined_classes
            .iter()
            .find(|(n, _)| n == base_name)
            .expect("pycc_hir: internal error: protocol base not found in defined_classes -- lower_class should have validated this")
            .1;
        for member in &base_def.protocol_members {
            // Only add inherited members that are not redeclared in this
            // class (redeclarations are added below from the class body).
            let name = match member {
                ProtocolMember::Method { name, .. } => name,
                ProtocolMember::Attribute { name, .. } => name,
            };
            if !protocol_members.iter().any(|m| match m {
                ProtocolMember::Method { name: n, .. } => n == name,
                ProtocolMember::Attribute { name: n, .. } => n == name,
            }) {
                protocol_members.push(member.clone());
            }
        }
    }
    for stmt in &def.body {
        match stmt {
            Stmt::FunctionDef(method_def) => {
                let method_name = method_def.name.to_string();
                // Reject `__init__` in a protocol class.
                if method_name == "__init__" {
                    return Err(unsupported(
                        format!(
                            "a protocol class `{class_name}` cannot define `__init__` -- \
                             protocols are not instantiated"
                        ),
                        method_def.range,
                    ));
                }
                // Reject decorators on protocol methods.
                if !method_def.decorator_list.is_empty() {
                    return Err(unsupported(
                        format!(
                            "decorators on protocol method `{class_name}.{method_name}` are \
                             not supported yet"
                        ),
                        method_def.range,
                    ));
                }
                // Reject generic protocol methods.
                if method_def.type_params.is_some() {
                    return Err(unsupported(
                        format!(
                            "a generic protocol method `{class_name}.{method_name}` is not \
                             supported yet"
                        ),
                        method_def.range,
                    ));
                }
                // The method body must be declaration-style (`...` or
                // `pass`).
                if !is_declaration_body(&method_def.body) {
                    return Err(unsupported(
                        format!(
                            "a protocol method `{class_name}.{method_name}` must have a \
                             declaration-style body (`...` or `pass`), not an implementation"
                        ),
                        method_def.range,
                    ));
                }
                // #984: `def __slots__` diverges in a `Protocol` body too --
                // `type.__new__` iterates `__slots__` while the `class`
                // statement executes, so CPython 3.13.9 raises `TypeError:
                // 'function' object is not iterable` and never creates the
                // class, while pycc recorded a protocol member and ran the
                // program. A `Protocol` body never reaches
                // `super::body`'s method loop -- `lower_class` returns
                // through this function first -- so the guard needs its own
                // call site here rather than sharing that one.
                //
                // Deliberately placed *after* every rejection above: the only
                // input it newly rejects is the decorator-free, non-generic,
                // declaration-body `def __slots__` that is falsely accepted
                // today, and each other spelling keeps the more specific
                // diagnostic it already reported.
                reject_reserved_protocol_method_name(&method_name, method_def.range.into())?;
                // Lower the method's parameter and return types. The
                // receiver is handled specially (the member signature does
                // not carry it -- `pycc_types::class`'s conformance check
                // skips exactly one parameter on the implementation side),
                // matching how `lower_method` handles it for regular
                // methods. The remaining parameters go through
                // `lower_arg_list`.
                //
                // #1181: stripped *positionally*, not by the name `self`,
                // since a protocol method's receiver may be spelled
                // anything. Deliberately **not** a bare
                // `args.split_first()`: PEP 570's positional-only receiver
                // (`def val(this, /, x: int)`) lives in `posonlyargs`, and
                // `args` then holds only the real parameters -- splitting
                // that list would eat `x` and leave an arity-0 member
                // against a 1-parameter implementation. Split off
                // `posonlyargs` first and fall back to `args` only when it
                // is empty, the same two-branch shape `class::receiver`'s
                // own `split_receiver` uses. Positional stripping is safe
                // here because this function rejects *every* decorator on a
                // protocol method before this point, so no
                // `@staticmethod`/`@classmethod` member's first real
                // parameter can be eaten. A protocol method declaring no
                // parameter at all keeps the pre-#1181 graceful handling:
                // nothing is stripped.
                let posonlyargs = &method_def.parameters.posonlyargs;
                let all_args = &method_def.parameters.args;
                let (posonly_rest, args_rest) = if let Some((_, rest)) = posonlyargs.split_first() {
                    (rest, all_args.as_slice())
                } else if let Some((_, rest)) = all_args.split_first() {
                    (&[][..], rest)
                } else {
                    (&[][..], &[][..])
                };
                let (params, return_ty) = {
                    let method_is_public = crate::is_public_name(&method_name);
                    let mut p = crate::lower_arg_list(
                        posonly_rest,
                        method_is_public,
                        &method_name,
                        type_param.as_deref(),
                        Some(&class_name),
                        &[],
                        class_name_defs,
                        crate::func::params::DefaultPolicy::Reject,
                    )?
                    .0;
                    p.extend(
                        crate::lower_arg_list(
                            args_rest,
                            method_is_public,
                            &method_name,
                            type_param.as_deref(),
                            Some(&class_name),
                            &[],
                            class_name_defs,
                            crate::func::params::DefaultPolicy::Reject,
                        )?
                        .0,
                    );
                    let r = crate::lower_return_annotation(
                        method_def.returns.as_deref(),
                        method_is_public,
                        &method_name,
                        type_param.as_deref(),
                        Some(&class_name),
                        &[],
                        class_name_defs,
                    )?;
                    (p, r)
                };
                // Part 1 of #1027: a protocol method's `-> memoryview` is
                // unsatisfiable in *every* artifact mode, so it is refused
                // here at the declaration rather than by either of
                // `src/memoryview_mode.rs`'s mode gates -- neither of which
                // could see it anyway, because a protocol method is recorded
                // as a `ProtocolMember::Method` and is never lowered to an
                // `HirItem::Function` (see this module's header). Part 1 adds
                // no expression that *produces* a `memoryview`, so no class
                // could implement the member: natively the implementing
                // method's own signature is refused (`I0405`), and under
                // `pycc build --ext` its return type is refused too
                // (`C0001`/`C0003`). The mirror image -- a `memoryview`
                // *parameter* -- is deliberately not refused here, because a
                // class genuinely can satisfy it under `--ext`, where an
                // exported function receives the view and passes it inward;
                // that position is mode-dependent and belongs to
                // `src/memoryview_mode.rs`'s native gate.
                //
                // Placed at the construction site rather than on the
                // assembled `protocol_members` vector so that a derived
                // `class Q(P, Protocol)` does not re-report its inherited
                // copy of the same member: the loop above copies base
                // members in wholesale, and only the declaring class runs
                // this arm.
                if return_ty == Ty::MemoryView {
                    return Err(unsupported(
                        format!(
                            "protocol method `{class_name}.{method_name}` returns a \
                             buffer, which is not supported yet -- a protocol is \
                             dispatched dynamically, and a buffer return reaches the \
                             CPython host only through the statically generated \
                             wrapper of a `pycc build --ext` export (#1174), which a \
                             protocol member has none of: #1027 and #1129 admit a \
                             buffer as a parameter of such an export, Part 2a of \
                             #1142 produces one as storage the allocating frame frees \
                             on exit, and Part 1 of #1175 returns a parameter-bound \
                             name over a second export taken on the host's own \
                             argument object"
                        ),
                        method_def.range,
                    ));
                }
                // Protocol member signatures exclude `self` (already
                // stripped above).
                let param_tys: Vec<Ty> = params.iter().map(|(_, ty)| ty.clone()).collect();
                let member = ProtocolMember::Method {
                    name: method_name.clone(),
                    param_tys,
                    return_ty,
                };
                // Replace if redeclared, otherwise add.
                if let Some(existing) = protocol_members.iter().position(
                    |m| matches!(m, ProtocolMember::Method { name, .. } if name == &method_name),
                ) {
                    protocol_members[existing] = member;
                } else {
                    protocol_members.push(member);
                }
            }
            Stmt::AnnAssign(ann) => {
                let Expr::Name(target) = ann.target.as_ref() else {
                    return Err(unsupported(
                        "a protocol attribute annotation must target a bare name (`x: int`), \
                         not an attribute access, subscript, or other expression",
                        pycc_ast::expr_range(&ann.target),
                    ));
                };
                let attr_name = target.id.to_string();
                let attr_ty = crate::annotation_to_ty(
                    &ann.annotation,
                    type_param.as_deref(),
                    Some(&class_name),
                    &[],
                    class_name_defs,
                )?;
                // D-228 (issue #918): a container-typed protocol
                // *attribute* is rejected. D-228 first gave the reason that
                // no class could satisfy it, because every path that
                // establishes an instance attribute restricted the slot to
                // a scalar word (D-154). #1262 made that reasoning false: a
                // hand-written `__init__` may now seed a `list[int]` or
                // `dict[str, int]` slot from a parameter
                // (`class::init_slot`). The rejection stays because
                // conformance checking and tests for a container-typed
                // protocol attribute are their own seam -- the attribute is
                // unimplemented, not unsatisfiable.
                //
                // This gate deliberately does *not* extend to a protocol
                // *method*'s parameters. A parameter type is a signature
                // type, not an instance slot, so `def f(self, xs: list[int])`
                // in a protocol body lowers through `crate::lower_arg_list`
                // like any other parameter and runs end to end (measured
                // against CPython 3.14, pinned by the
                // `issue_918_container_annotations` integration test). Nor
                // does it extend to a protocol method's *return* type: a
                // container return type lowers and runs end to end since
                // #925 removed the return-position gate that once rejected
                // it -- this gate covers attributes only. (A *protocol* return
                // type, `def clone(self) -> P: ...`, is rejected separately by
                // `crate::func::lower_return_annotation`'s own #934 gate,
                // which is the shared return-annotation seam and not part of
                // this attribute check.)
                //
                // Non-container attribute types keep their existing
                // behaviour exactly: `Ty::Instance`, `Ty::Optional`,
                // `Ty::None` and `Ty::Protocol` attributes are all still
                // accepted, which is why this is a container check and not a
                // reuse of `is_scalar_slot_type`.
                if matches!(
                    attr_ty,
                    Ty::List(_) | Ty::Dict(_) | Ty::Set(_) | Ty::FrozenSet(_) | Ty::Tuple(_)
                ) {
                    return Err(unsupported(
                        format!(
                            "protocol attribute `{class_name}.{attr_name}` has container type \
                             `{}`, which is not supported yet as a protocol attribute; a \
                             container type in a protocol method's parameter is supported",
                            attr_ty.name()
                        ),
                        ann.range,
                    ));
                }
                // Part 1 of #1027: a buffer-typed attribute is refused
                // too, for a reason that still holds after #1262 admitted
                // `list[int]`/`dict[str, int]` slots: no class could
                // satisfy it. A class attribute slot holds a scalar, a
                // `list[int]` or a `dict[str, int]` (D-154), never a
                // buffer. Part 2a of #1142 (#1165)
                // added the type's first producing expression, and does not
                // weaken this: `a = ndarray(n)` binds a *local* name inside
                // the allocating function and nothing may carry the value
                // out of it, so there is still no value a class could store
                // in such a slot.
                // Kept as its own arm rather than folded into the container
                // list above because the message's reasoning differs: a
                // buffer is admitted at a `pycc build --ext`
                // signature and nowhere else, which is the sentence a
                // reader needs here.
                if matches!(attr_ty, Ty::MemoryView) {
                    return Err(unsupported(
                        format!(
                            "protocol attribute `{class_name}.{attr_name}` has a buffer \
                             type, which is not supported yet -- no class could \
                             satisfy it, because a class attribute slot holds a scalar \
                             (`int`, `float`, `bool`, `str`), a `list[int]` or a \
                             `dict[str, int]`, never a buffer; #1027 and #1129 \
                             admit a buffer as a parameter of a `pycc build --ext` export, \
                             and Part 2a of #1142 only as a local binding inside the \
                             function that allocated it"
                        ),
                        ann.range,
                    ));
                }
                // A protocol attribute cannot have a default value.
                if ann.value.is_some() {
                    return Err(unsupported(
                        format!(
                            "a protocol attribute `{class_name}.{attr_name}` cannot have a \
                             default value -- protocol attributes are requirements, not \
                             initializers"
                        ),
                        ann.range,
                    ));
                }
                let member = ProtocolMember::Attribute {
                    name: attr_name.clone(),
                    ty: attr_ty,
                };
                if let Some(existing) = protocol_members.iter().position(
                    |m| matches!(m, ProtocolMember::Attribute { name, .. } if name == &attr_name),
                ) {
                    protocol_members[existing] = member;
                } else {
                    protocol_members.push(member);
                }
            }
            Stmt::Pass(_) => {
                // `pass` is a no-op in a protocol body.
            }
            // #744: a docstring (a bare string-literal expression statement)
            // is a no-op, matching `validate_init_subclass_body`'s existing
            // precedent for the same construct.
            Stmt::Expr(expr_stmt) if matches!(*expr_stmt.value, Expr::StringLiteral(_)) => {}
            _ => {
                // Issue #890: name the rejected statement kind. Only a
                // docstring `Expr` is accepted above, so the common stub
                // idiom `class P(Protocol): ...` lands here and must name
                // the expression inside the statement, not just "an
                // expression statement".
                let kind = match stmt {
                    Stmt::Expr(expr_stmt) => format!(
                        "an expression statement ({})",
                        pycc_ast::expr_kind_name(&expr_stmt.value)
                    ),
                    _ => pycc_ast::stmt_kind_name(stmt).to_string(),
                };
                return Err(unsupported(
                    format!(
                        "a protocol class body must contain only method definitions (`def ...`) \
                         and annotated assignments (`x: int`) -- {kind} is not supported yet"
                    ),
                    pycc_ast::stmt_range(stmt),
                ));
            }
        }
    }
    Ok((
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: class_name,
            bases,
            mro,
            attrs: Vec::new(),
            methods: Vec::new(),
            properties: Vec::new(),
            static_methods: Vec::new(),
            class_methods: Vec::new(),
            type_param,
            is_enum: false,
            implicit_object_init: false,
            method_defaults: Vec::new(),
            enum_members: Vec::new(),
            is_dataclass: false,
            dataclass_fields: Vec::new(),
            is_protocol: true,
            runtime_checkable,
            protocol_members,
            abstract_methods: Vec::new(),
            is_abstract: false,
        },
        Vec::new(),
    ))
}
