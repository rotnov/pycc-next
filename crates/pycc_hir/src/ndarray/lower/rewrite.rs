//! The rewrite of one scope's statements into `HirExpr::Nd` operations
//! and the checks after each binding.

use super::*;

/// The check after a binding of a typed array name (and of a parameter,
/// before the body). Every binding is checked, whatever the value's static
/// facts say: a module global, an attribute or a call result can hold a
/// different object by the time it is read, and CPython code can change an
/// array's dtype or dimension count in place (`__setstate__`, `resize`).
pub(super) fn check_stmt(value: HirExpr, nd: NdTy, cached: bool) -> HirStmt {
    HirStmt::ExprStmt(nd_expr(NdOp::Check, value, Vec::new(), nd, cached))
}

pub(super) fn nd_expr(
    op: NdOp,
    base: HirExpr,
    args: Vec<HirExpr>,
    nd: NdTy,
    cached: bool,
) -> HirExpr {
    HirExpr::Nd(Box::new(NdExpr {
        op,
        base,
        args,
        nd,
        cached,
    }))
}

/// Rewrites one scope's statements.
pub(super) struct Scope<'c, 'f> {
    pub(super) ctx: &'c mut Ctx,
    pub(super) facts: &'f Facts,
    /// The function's own typed array return, if any.
    pub(super) ret: Option<NdTy>,
}

impl Scope<'_, '_> {
    fn fresh(&mut self) -> String {
        self.ctx.fresh += 1;
        format!("_pycc_nd{}", self.ctx.fresh)
    }

    fn nd(&self, expr: &HirExpr) -> Option<NdTy> {
        nd_of_value(self.ctx, self.facts, expr)
    }

    /// Whether an operation on `base` may use its header snapshot
    /// ([`NdExpr::cached`]): `base` is a cache-safe local of this scope.
    fn cached(&self, base: &HirExpr) -> bool {
        matches!(base, HirExpr::Name(name) if self.facts.cache_safe.contains(name))
    }

    /// [`nd_expr`] with [`Self::cached`] decided for `base`.
    fn op(&self, op: NdOp, base: HirExpr, args: Vec<HirExpr>, nd: NdTy) -> HirExpr {
        let cached = self.cached(&base);
        nd_expr(op, base, args, nd, cached)
    }

    /// The check after a binding of `name`.
    fn check_name(&self, name: String, nd: NdTy) -> HirStmt {
        let cached = self.facts.cache_safe.contains(&name);
        check_stmt(HirExpr::Name(name), nd, cached)
    }

    /// The index expressions of a subscript of an `ndim`-dimensional
    /// array, or `None` when it is not an element index.
    fn element_indices(&self, index: &HirExpr, ndim: u8) -> Option<Vec<HirExpr>> {
        match index {
            HirExpr::TupleLiteral(elements) if elements.len() == usize::from(ndim) => {
                (!elements.iter().any(|e| matches!(e, HirExpr::Slice { .. })))
                    .then(|| elements.clone())
            }
            HirExpr::TupleLiteral(_) | HirExpr::Slice { .. } => None,
            HirExpr::Name(name) if ndim >= 2 => {
                let len = lookup(self.ctx, self.facts, name, |f| f.tuples.get(name).copied())?;
                (len == usize::from(ndim)).then(|| {
                    (0..len)
                        .map(|i| HirExpr::Subscript {
                            base: Box::new(index.clone()),
                            index: Box::new(HirExpr::IntLiteral(i64::try_from(i).unwrap_or(0))),
                        })
                        .collect()
                })
            }
            other if ndim == 1 => Some(vec![other.clone()]),
            _ => None,
        }
    }

    fn rewrite_expr(&mut self, expr: &mut HirExpr) {
        for child in expr_children_mut(expr) {
            self.rewrite_expr(child);
        }
        // A list display passed to a numpy function (`np.array([[1, 2],
        // [3, 4]])`, `np.zeros([n, m])`) is built as a CPython list, which
        // is what CPython passes too.
        let numpy_call = |e: &HirExpr| matches!(e, HirExpr::MethodCall { base, .. } if is_numpy_module(self.ctx, self.facts, base));
        let direct = numpy_call(expr);
        match expr {
            HirExpr::MethodCall { args, .. } if direct => {
                args.iter_mut().for_each(object_list_display);
            }
            HirExpr::KeywordCall { call, keywords, .. } if numpy_call(call) => {
                if let HirExpr::MethodCall { args, .. } = call.as_mut() {
                    args.iter_mut().for_each(object_list_display);
                }
                keywords
                    .iter_mut()
                    .for_each(|(_, v)| object_list_display(v));
            }
            _ => {}
        }
        let replacement = match expr {
            HirExpr::Subscript { base, index } => {
                if let HirExpr::Subscript {
                    base: inner_base,
                    index: inner_index,
                } = base.as_ref()
                    && is_scalar_index(inner_index)
                    && is_scalar_index(index)
                    && let Some(nd) = self.nd(inner_base)
                    && nd.ndim == 2
                {
                    Some(self.op(
                        NdOp::Get { chained: true },
                        (**inner_base).clone(),
                        vec![(**inner_index).clone(), (**index).clone()],
                        nd,
                    ))
                } else if let HirExpr::Nd(shape) = base.as_ref()
                    && shape.op == NdOp::Shape
                    && let HirExpr::IntLiteral(k) = index.as_ref()
                {
                    let ndim = i64::from(shape.nd.ndim);
                    let k = if *k < 0 { *k + ndim } else { *k };
                    (0..ndim).contains(&k).then(|| {
                        self.op(
                            NdOp::Dim(u8::try_from(k).unwrap_or(0)),
                            shape.base.clone(),
                            Vec::new(),
                            shape.nd,
                        )
                    })
                } else if let Some(nd) = self.nd(base) {
                    self.element_indices(index, nd.ndim).map(|indices| {
                        self.op(NdOp::Get { chained: false }, (**base).clone(), indices, nd)
                    })
                } else {
                    None
                }
            }
            HirExpr::AttrGet { base, attr } => self.nd(base).and_then(|nd| {
                // A `bytearray` has none of these attributes.
                if nd.dtype == NdDtype::Byte {
                    return None;
                }
                let op = match attr.as_str() {
                    "shape" => NdOp::Shape,
                    "size" => NdOp::Size,
                    "ndim" => NdOp::Ndim,
                    _ => return None,
                };
                Some(self.op(op, (**base).clone(), Vec::new(), nd))
            }),
            HirExpr::Call { callee, args }
                if callee == "len"
                    && !self.facts.locals.contains(callee)
                    && !self.ctx.functions.contains(callee)
                    && args.len() == 1 =>
            {
                self.nd(&args[0])
                    .map(|nd| self.op(NdOp::Dim(0), args[0].clone(), Vec::new(), nd))
            }
            // `b.append(x)` / `b.extend(xs)` on a byte buffer: the
            // builtins that store a native `int` (or list of them) into the
            // buffer and otherwise call the method (`crate::builtins`'
            // `BYTEARRAY_APPEND` / `BYTEARRAY_EXTEND`).
            HirExpr::MethodCall { base, method, args }
                if matches!(method.as_str(), "append" | "extend")
                    && args.len() == 1
                    && matches!(base.as_ref(), HirExpr::Name(_))
                    && self.nd(base).is_some_and(|nd| nd.dtype == NdDtype::Byte) =>
            {
                let callee = if method == "append" {
                    crate::builtins::BYTEARRAY_APPEND
                } else {
                    crate::builtins::BYTEARRAY_EXTEND
                };
                Some(HirExpr::Call {
                    callee: callee.to_string(),
                    args: vec![(**base).clone(), args[0].clone()],
                })
            }
            // The same call kept with its container reading too (a module
            // whose class defines `append`): the byte buffer's builtin is
            // the one reading left.
            HirExpr::ReceiverDispatchedCall { call, .. }
                if matches!(call.as_ref(), HirExpr::Call { callee, .. }
                    if callee == crate::builtins::BYTEARRAY_APPEND
                        || callee == crate::builtins::BYTEARRAY_EXTEND) =>
            {
                Some((**call).clone())
            }
            _ => None,
        };
        if let Some(replacement) = replacement {
            *expr = replacement;
        }
    }

    pub(super) fn rewrite_body(&mut self, body: Vec<HirStmt>) -> Vec<HirStmt> {
        let mut out = Vec::with_capacity(body.len());
        let mut iter = body.into_iter().peekable();
        while let Some(stmt) = iter.next() {
            // `a[i][j] = v` reaches HIR as `_pycc_N = a[i]; _pycc_N[j] = v`
            // (desugar binds a computed subscript base first): fuse it back
            // into one element store when the temporary is used nowhere
            // else.
            if let HirStmt::Assign { target, value } = &stmt
                && target.starts_with("_pycc_")
                && let HirExpr::Subscript { base, index } = value
                && is_scalar_index(index)
                && let Some(nd) = self.nd(base)
                && nd.ndim == 2
                && let Some(HirStmt::DictSet { dict, key, .. }) = iter.peek()
                && dict == target
                && is_scalar_index(key)
            {
                let (base, row) = ((**base).clone(), (**index).clone());
                let temp = target.clone();
                let Some(HirStmt::DictSet { key, mut value, .. }) = iter.next() else {
                    unreachable!()
                };
                // The stored value may read the row (`a[i][j] += v`).
                substitute_row(&mut value, &temp, &base, &row);
                if !mentions_name(&value, &temp) {
                    let mut stmt = HirStmt::ExprStmt(self.op(
                        NdOp::Set { chained: true },
                        base,
                        vec![row, key, value],
                        nd,
                    ));
                    self.rewrite_stmt_exprs(&mut stmt);
                    out.push(stmt);
                    continue;
                }
                // Not fusible after all: keep both statements.
                out.extend(self.rewrite_stmt(stmt));
                out.extend(self.rewrite_stmt(HirStmt::DictSet {
                    dict: temp,
                    key,
                    value,
                }));
                continue;
            }
            out.extend(self.rewrite_stmt(stmt));
        }
        out
    }

    /// Rewrites the expressions of `stmt` itself (not its nested bodies).
    fn rewrite_stmt_exprs(&mut self, stmt: &mut HirStmt) {
        let mut exprs: Vec<&mut HirExpr> = Vec::new();
        match stmt {
            HirStmt::ExprStmt(e) | HirStmt::Assign { value: e, .. } => exprs.push(e),
            HirStmt::AnnAssign { value, .. } | HirStmt::Return(value) => {
                exprs.extend(value.as_mut())
            }
            HirStmt::If { test, .. } | HirStmt::While { test, .. } => exprs.push(test),
            HirStmt::ForRange {
                start, stop, step, ..
            } => exprs.extend([start, stop, step]),
            HirStmt::ForObject { iter, .. } => exprs.push(iter),
            HirStmt::DictSet { key, value, .. } => exprs.extend([key, value]),
            HirStmt::ListCompAssign {
                iter, cond, elt, ..
            }
            | HirStmt::SetCompAssign {
                iter, cond, elt, ..
            } => {
                comp_iter_exprs_mut(iter, &mut exprs);
                exprs.extend(cond.as_deref_mut());
                exprs.push(elt);
            }
            HirStmt::DictCompAssign {
                iter,
                cond,
                key,
                value,
                ..
            } => {
                comp_iter_exprs_mut(iter, &mut exprs);
                exprs.extend(cond.as_deref_mut());
                exprs.push(key);
                exprs.push(value);
            }
            HirStmt::AttrSet { base, value, .. } => exprs.extend([base, value]),
            HirStmt::Match { subject, cases } => {
                exprs.push(subject);
                for case in cases {
                    exprs.extend(case.guard.as_mut());
                }
            }
            HirStmt::Raise { exc, cause } => {
                exprs.extend(exc.as_mut());
                exprs.extend(cause.as_mut());
            }
            HirStmt::DeleteSlice {
                base,
                start,
                stop,
                step,
                ..
            } => {
                exprs.push(base);
                exprs.extend([start, stop, step].into_iter().flatten().map(AsMut::as_mut));
            }
            HirStmt::SetSlice {
                base,
                start,
                stop,
                step,
                value,
                ..
            } => {
                exprs.push(value);
                exprs.push(base);
                exprs.extend([start, stop, step].into_iter().flatten().map(AsMut::as_mut));
            }
            HirStmt::DeleteAttr { base, .. } => exprs.push(base),
            HirStmt::DeleteItem { base, key, .. } => {
                exprs.push(base);
                exprs.push(key);
            }
            HirStmt::ForList { .. }
            | HirStmt::Try { .. }
            | HirStmt::TryStar { .. }
            | HirStmt::Delete { .. }
            | HirStmt::ForeignImport { .. }
            | HirStmt::Break
            | HirStmt::Continue
            | HirStmt::LoopElse { .. } => {}
        }
        for e in exprs {
            self.rewrite_expr(e);
        }
    }

    fn rewrite_stmt(&mut self, mut stmt: HirStmt) -> Vec<HirStmt> {
        // Nested bodies first.
        match &mut stmt {
            HirStmt::If { body, orelse, .. } => {
                *body = self.rewrite_body(std::mem::take(body));
                *orelse = self.rewrite_body(std::mem::take(orelse));
            }
            HirStmt::While { body, .. }
            | HirStmt::ForRange { body, .. }
            | HirStmt::ForList { body, .. }
            | HirStmt::ForObject { body, .. } => *body = self.rewrite_body(std::mem::take(body)),
            HirStmt::Match { cases, .. } => {
                for case in cases {
                    case.body = self.rewrite_body(std::mem::take(&mut case.body));
                }
            }
            HirStmt::Try {
                body,
                handlers,
                orelse,
                finalbody,
            }
            | HirStmt::TryStar {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                *body = self.rewrite_body(std::mem::take(body));
                for handler in handlers {
                    handler.body = self.rewrite_body(std::mem::take(&mut handler.body));
                }
                *orelse = self.rewrite_body(std::mem::take(orelse));
                *finalbody = self.rewrite_body(std::mem::take(finalbody));
            }
            HirStmt::LoopElse { loop_stmt, orelse } => {
                let mut lowered =
                    self.rewrite_stmt(std::mem::replace(loop_stmt.as_mut(), HirStmt::Break));
                if lowered.len() == 1 {
                    **loop_stmt = lowered.pop().unwrap_or(HirStmt::Break);
                } else {
                    // A loop statement always lowers to one statement.
                    unreachable!("a loop lowers to one statement");
                }
                *orelse = self.rewrite_body(std::mem::take(orelse));
            }
            _ => {}
        }
        // Statement-level shapes, decided on the statement's original
        // expressions (before their own rewrite).
        match stmt {
            HirStmt::Assign { target, mut value } => {
                let bound = self.facts.nd.get(&target).copied();
                self.rewrite_expr(&mut value);
                let mut out = vec![HirStmt::Assign {
                    target: target.clone(),
                    value,
                }];
                if let Some(nd) = bound {
                    out.push(self.check_name(target, nd));
                }
                out
            }
            HirStmt::AnnAssign {
                target,
                mut annotation,
                mut value,
                is_final,
            } => {
                let bound = match (&annotation, &value) {
                    (Ty::NdArray(nd), Some(_)) => Some(*nd),
                    (_, Some(_)) => self.facts.nd.get(&target).copied(),
                    _ => None,
                };
                erase(&mut annotation);
                if let Some(v) = value.as_mut() {
                    self.rewrite_expr(v);
                }
                let mut out = vec![HirStmt::AnnAssign {
                    target: target.clone(),
                    annotation,
                    value,
                    is_final,
                }];
                if let Some(nd) = bound {
                    out.push(self.check_name(target, nd));
                }
                out
            }
            HirStmt::DictSet { dict, key, value } => {
                if let Some(nd) = nd_of_name(self.ctx, self.facts, &dict)
                    && let Some(mut indices) = self.element_indices(&key, nd.ndim)
                {
                    let mut value = value;
                    for index in &mut indices {
                        self.rewrite_expr(index);
                    }
                    self.rewrite_expr(&mut value);
                    indices.push(value);
                    return vec![HirStmt::ExprStmt(self.op(
                        NdOp::Set { chained: false },
                        HirExpr::Name(dict),
                        indices,
                        nd,
                    ))];
                }
                let mut stmt = HirStmt::DictSet { dict, key, value };
                self.rewrite_stmt_exprs(&mut stmt);
                vec![stmt]
            }
            HirStmt::ForList { var, list, body } => {
                if let Some(nd) = nd_of_name(self.ctx, self.facts, &list)
                    && nd.ndim == 1
                    && nd.dtype != NdDtype::Byte
                {
                    let index = self.fresh();
                    let mut new_body = vec![HirStmt::Assign {
                        target: var.clone(),
                        value: self.op(
                            NdOp::Get { chained: false },
                            HirExpr::Name(list.clone()),
                            vec![HirExpr::Name(index.clone())],
                            nd,
                        ),
                    }];
                    // An element bound to a name typed as an array fails
                    // that name's check, as any other binding would.
                    if let Some(var_nd) = self.facts.nd.get(&var).copied() {
                        new_body.push(self.check_name(var, var_nd));
                    }
                    new_body.extend(body);
                    return vec![HirStmt::ForRange {
                        var: index,
                        start: HirExpr::IntLiteral(0),
                        stop: self.op(NdOp::Dim(0), HirExpr::Name(list), Vec::new(), nd),
                        step: HirExpr::IntLiteral(1),
                        body: new_body,
                    }];
                }
                // A byte buffer: `for x in b` is `i = 0; while i < len(b):
                // x = b[i]; i += 1; ...`, the steps CPython's `bytearray`
                // iterator takes, which reads the size again at each step
                // (the body may resize the buffer).
                if let Some(nd) = nd_of_name(self.ctx, self.facts, &list)
                    && nd.dtype == NdDtype::Byte
                {
                    let index = self.fresh();
                    let mut new_body = vec![
                        HirStmt::Assign {
                            target: var.clone(),
                            value: self.op(
                                NdOp::Get { chained: false },
                                HirExpr::Name(list.clone()),
                                vec![HirExpr::Name(index.clone())],
                                nd,
                            ),
                        },
                        HirStmt::Assign {
                            target: index.clone(),
                            value: HirExpr::BinOp {
                                op: crate::BinOpKind::Add,
                                left: Box::new(HirExpr::Name(index.clone())),
                                right: Box::new(HirExpr::IntLiteral(1)),
                            },
                        },
                    ];
                    new_body.extend(body);
                    return vec![
                        HirStmt::Assign {
                            target: index.clone(),
                            value: HirExpr::IntLiteral(0),
                        },
                        HirStmt::While {
                            test: HirExpr::Compare {
                                op: crate::CmpOpKind::Lt,
                                left: Box::new(HirExpr::Name(index)),
                                right: Box::new(self.op(
                                    NdOp::Dim(0),
                                    HirExpr::Name(list),
                                    Vec::new(),
                                    nd,
                                )),
                            },
                            body: new_body,
                        },
                    ];
                }
                // The rows of a two-dimensional array: `for row in a` is
                // `for i in range(len(a)): row = a[i]`, each row the view
                // numpy's own subscript makes (a CPython `for` over an object
                // is not compiled inside a function).
                if let Some(nd) = nd_of_name(self.ctx, self.facts, &list)
                    && nd.ndim == 2
                {
                    let index = self.fresh();
                    let mut new_body = vec![HirStmt::Assign {
                        target: var.clone(),
                        value: HirExpr::Subscript {
                            base: Box::new(HirExpr::Name(list.clone())),
                            index: Box::new(HirExpr::Name(index.clone())),
                        },
                    }];
                    // Checked against the name's own type, which is the
                    // row type unless an annotation says otherwise.
                    if let Some(var_nd) = self.facts.nd.get(&var).copied() {
                        new_body.push(self.check_name(var, var_nd));
                    }
                    new_body.extend(body);
                    return vec![HirStmt::ForRange {
                        var: index,
                        start: HirExpr::IntLiteral(0),
                        stop: self.op(NdOp::Dim(0), HirExpr::Name(list), Vec::new(), nd),
                        step: HirExpr::IntLiteral(1),
                        body: new_body,
                    }];
                }
                vec![HirStmt::ForList { var, list, body }]
            }
            HirStmt::AttrSet {
                mut base,
                attr,
                mut value,
            } => {
                let slot = class_of(self.ctx, self.facts, &base)
                    .and_then(|class| self.ctx.class_attrs.get(&(class, attr.clone())).copied());
                let check = slot;
                self.rewrite_expr(&mut base);
                self.rewrite_expr(&mut value);
                match check {
                    Some(nd) => {
                        let temp = self.fresh();
                        vec![
                            HirStmt::Assign {
                                target: temp.clone(),
                                value,
                            },
                            check_stmt(HirExpr::Name(temp.clone()), nd, false),
                            HirStmt::AttrSet {
                                base,
                                attr,
                                value: HirExpr::Name(temp),
                            },
                        ]
                    }
                    None => vec![HirStmt::AttrSet { base, attr, value }],
                }
            }
            HirStmt::Return(Some(mut value)) => {
                let check = self.ret;
                self.rewrite_expr(&mut value);
                match check {
                    Some(nd) => {
                        let temp = self.fresh();
                        vec![
                            HirStmt::Assign {
                                target: temp.clone(),
                                value,
                            },
                            check_stmt(HirExpr::Name(temp.clone()), nd, false),
                            HirStmt::Return(Some(HirExpr::Name(temp))),
                        ]
                    }
                    None => vec![HirStmt::Return(Some(value))],
                }
            }
            mut other => {
                self.rewrite_stmt_exprs(&mut other);
                vec![other]
            }
        }
    }
}

pub(super) fn comp_iter_exprs_mut<'a>(
    iter: &'a mut crate::CompIter,
    out: &mut Vec<&'a mut HirExpr>,
) {
    match iter {
        crate::CompIter::Range { start, stop, step } => out.extend([start, stop, step]),
        crate::CompIter::Iterable(e) => out.push(e),
        crate::CompIter::Name(_) => {}
    }
}

/// Turns a (nested) list display into a CPython list display.
pub(super) fn object_list_display(expr: &mut HirExpr) {
    if let HirExpr::ListLiteral(elements) = expr {
        let mut elements = std::mem::take(elements);
        elements.iter_mut().for_each(object_list_display);
        *expr = HirExpr::ObjectList(elements);
    }
}

/// Replaces every `temp[k]` in `expr` by `base[row][k]`.
pub(super) fn substitute_row(expr: &mut HirExpr, temp: &str, base: &HirExpr, row: &HirExpr) {
    if let HirExpr::Subscript { base: b, .. } = expr
        && matches!(b.as_ref(), HirExpr::Name(n) if n == temp)
    {
        **b = HirExpr::Subscript {
            base: Box::new(base.clone()),
            index: Box::new(row.clone()),
        };
    }
    for child in expr_children_mut(expr) {
        substitute_row(child, temp, base, row);
    }
}

pub(super) fn mentions_name(expr: &HirExpr, name: &str) -> bool {
    matches!(expr, HirExpr::Name(n) if n == name)
        || expr_children(expr).iter().any(|c| mentions_name(c, name))
}
