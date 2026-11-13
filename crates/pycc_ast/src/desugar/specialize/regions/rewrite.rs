//! The copies, guards and region calls [`super::specialize_regions`] adds,
//! planned from the proven signatures ([`plan`]) and then written into the
//! unit ([`apply`]).

use super::super::super::build::{call, call_name, load};
use super::super::super::signatures::parse_helper;
use super::super::Specialized;
use super::prove::{Kind, Sig};
use super::{ArgRef, Edge, Edges, Member, REGION_COPY_PREFIX, arm, intact, kind_annotation, result_annotation};
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{
    AtomicNodeIndex, BoolOp, CmpOp, ElifElseClause, Expr, ExprBoolOp, ExprCall, ExprCompare, ExprIf, ModModule, Stmt,
    StmtFunctionDef, StmtIf,
};
use ruff_text_size::{Ranged, TextRange};
use std::collections::{BTreeMap, BTreeSet};

/// The most regions one module arms (`PYCC_EXT_REGION_MAX`, `regions.c`).
const MAX_REGIONS: usize = 64;

/// One region call to rewrite in a copy.
#[derive(Debug, Clone)]
struct CallRewrite {
    /// The region's index.
    region: usize,
    targets: Vec<Target>,
    /// The local a dispatch calls.
    local: Option<String>,
    /// The conversion the generic arm takes (`__pycc_rconv_<shape>`).
    conv: Option<String>,
    /// The callee as the source spells it (the global, or the dispatch's
    /// local), for the conversion's `TypeError`.
    callee: String,
    /// For a callee returning a tuple with `int` items: which items are
    /// `int`s, for an unpack of the call (`split_sites`).
    ints: Option<Vec<bool>>,
}

/// One callee of a region call.
#[derive(Debug, Clone)]
struct Target {
    copy: String,
    /// The module global a dispatch's local is compared with.
    global: String,
    /// The call's arguments in the callee's parameter order.
    args: Vec<ArgRef>,
    /// The callee's parsed literal defaults, for `ArgRef::Default`.
    defaults: Vec<Option<Expr>>,
}

/// One copy to add.
#[derive(Debug, Clone)]
struct Copy {
    lift: String,
    name: String,
    region: usize,
    params: Vec<(String, Kind)>,
    result: String,
    cyclic: bool,
    report: String,
    calls: BTreeMap<(u32, u32), CallRewrite>,
}

/// What [`apply`] writes.
pub(crate) struct Plan {
    copies: Vec<Copy>,
    /// Each region's index, member globals and watched pattern globals.
    regions: Vec<(usize, Vec<String>, Vec<String>)>,
    convs: BTreeSet<String>,
}

fn key(range: TextRange) -> (u32, u32) {
    (range.start().to_u32(), range.end().to_u32())
}

/// The copy's name of a lift.
fn copy_name(lift: &str) -> String {
    format!("{REGION_COPY_PREFIX}{}", lift.trim_start_matches("__pycc_"))
}

/// Plans the copies of `members`, all of them in `regions`, with `sigs`;
/// `cyclic` names the members of a region that reach themselves.
pub(crate) fn plan(
    members: &[Member<'_>],
    edges: &Edges<'_>,
    regions: &[Vec<usize>],
    sigs: &[Sig],
    cyclic: &dyn Fn(&[usize]) -> BTreeSet<usize>,
) -> Plan {
    let mut copies = Vec::new();
    let mut planned = Vec::new();
    let mut convs = BTreeSet::new();
    for (k, region) in regions.iter().enumerate().take(MAX_REGIONS) {
        let cycle = cyclic(region);
        let mut patterns: BTreeSet<String> = BTreeSet::new();
        for &m in region {
            patterns.extend(super::prove::proven_patterns(m, members, edges, sigs));
            let mut calls = BTreeMap::new();
            for (site, edge) in edges.sites.get(&m).map(Vec::as_slice).unwrap_or_default() {
                if !edges.binds(site, edge) {
                    continue;
                }
                let callees = edge.callees();
                let result = &sigs[callees[0]].result;
                if callees.iter().any(|c| sigs[*c].result != *result) {
                    continue;
                }
                let (_, conv) = result_annotation(result);
                if let Some(c) = &conv {
                    convs.insert(c.clone());
                }
                let ints = match result {
                    super::prove::Ty::Tuple(items) => {
                        convs.insert("i".to_string());
                        Some(items.iter().map(|t| *t == super::prove::Ty::Int).collect())
                    }
                    _ => None,
                };
                let local = match (edge, site.func.as_ref()) {
                    (Edge::Dispatch(_), Expr::Name(n)) => Some(n.id.to_string()),
                    _ => None,
                };
                let callee = local.clone().unwrap_or_else(|| members[callees[0]].function.global.clone());
                calls.insert(
                    key(site.range),
                    CallRewrite {
                        region: k,
                        targets: callees
                            .iter()
                            .map(|c| Target {
                                copy: copy_name(&members[*c].function.lift),
                                global: members[*c].function.global.clone(),
                                args: edges.bound[&(*site as *const _, *c)].clone(),
                                defaults: edges.defaults[*c].clone(),
                            })
                            .collect(),
                        local,
                        conv,
                        callee,
                        ints,
                    },
                );
            }
            let (result, _) = result_annotation(&sigs[m].result);
            copies.push(Copy {
                lift: members[m].function.lift.clone(),
                name: copy_name(&members[m].function.lift),
                region: k,
                params: members[m].params.iter().cloned().zip(sigs[m].params.iter().copied()).collect(),
                result,
                cyclic: cycle.contains(&m),
                report: members[m].function.report.clone(),
                calls,
            });
        }
        planned.push((
            k,
            region.iter().map(|m| members[*m].function.global.clone()).collect(),
            patterns.into_iter().collect(),
        ));
    }
    Plan { copies, regions: planned, convs }
}

/// Where the names a region call's rewrite adds start: the call's closing
/// parenthesis. Later passes find a name by the offset its range starts at
/// (`webs` renames a variable's reads and writes that way), and no name of
/// the source starts at a `)`.
fn added(site: &ExprCall) -> TextRange {
    TextRange::empty(site.range.end() - ruff_text_size::TextSize::from(1))
}

/// `site`'s arguments in `target`'s parameter order.
fn target_args(site: &ExprCall, target: &Target) -> Vec<Expr> {
    target
        .args
        .iter()
        .map(|r| match r {
            ArgRef::Positional(i) => site.arguments.args[*i].clone(),
            ArgRef::Keyword(i) => site.arguments.keywords[*i].value.clone(),
            ArgRef::Default(p) => {
                let mut e = target.defaults[*p].clone().expect("a bound default is a literal");
                ruff_python_ast::relocate::relocate_expr(&mut e, added(site));
                e
            }
        })
        .collect()
}

/// The call of `target`'s copy for `site`.
fn native_call(site: &ExprCall, target: &Target) -> Expr {
    call(load(&target.copy, added(site)), target_args(site, target), site.range)
}

/// When `site` may call `target`'s copy: its region is intact and, for a
/// dispatch, the local holds the target's function.
fn arm_test(site: &ExprCall, rewrite: &CallRewrite, target: &Target) -> Expr {
    let at = added(site);
    match &rewrite.local {
        None => intact(rewrite.region, at),
        Some(local) => Expr::BoolOp(ExprBoolOp {
            node_index: AtomicNodeIndex::default(),
            range: site.range,
            op: BoolOp::And,
            values: vec![
                intact(rewrite.region, at),
                Expr::Compare(ExprCompare {
                    node_index: AtomicNodeIndex::default(),
                    range: site.range,
                    // The local's own range: `webs` renames it with the call's.
                    left: Box::new(load(local, site.func.range())),
                    ops: vec![CmpOp::Is].into(),
                    comparators: vec![super::super::super::build::subscript(
                        load(super::NS, at),
                        super::super::super::build::string(&target.global, at),
                        ruff_python_ast::ExprContext::Load,
                        at,
                    )]
                    .into(),
                }),
            ],
        }),
    }
}

/// Rewrites the region calls of a copy's body that [`split_sites`] left in
/// expressions: `(COPY(...) if intact else conv(f(...)))`.
struct Calls<'p> {
    calls: &'p BTreeMap<(u32, u32), CallRewrite>,
    /// The calls [`split_sites`] rewrote as statements.
    skip: &'p BTreeSet<(u32, u32)>,
}

impl Transformer for Calls<'_> {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        if !matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_)) {
            transformer::walk_stmt(self, stmt);
        }
    }

    fn visit_expr(&self, expr: &mut Expr) {
        if matches!(
            expr,
            Expr::Lambda(_) | Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_)
        ) {
            return;
        }
        transformer::walk_expr(self, expr);
        let Expr::Call(site) = expr else {
            return;
        };
        if self.skip.contains(&key(site.range)) {
            return;
        }
        let Some(rewrite) = self.calls.get(&key(site.range)) else {
            return;
        };
        let range = site.range;
        let mut chosen = Expr::Call(site.clone());
        if let Some(conv) = &rewrite.conv {
            chosen = converted(conv, chosen, &rewrite.callee, site);
        }
        for target in rewrite.targets.iter().rev() {
            chosen = Expr::If(ExprIf {
                node_index: AtomicNodeIndex::default(),
                range,
                test: Box::new(arm_test(site, rewrite, target)),
                body: Box::new(native_call(site, target)),
                orelse: Box::new(chosen),
            });
        }
        *expr = chosen;
    }
}

/// What a statement [`split_sites`] rewrites does with its region call.
enum Use {
    /// `a, b = f(...)`, one name per item of the callee's tuple.
    Unpack,
    /// `return f(...)`.
    Return,
}

/// Rewrites, as statements, each `a, b = f(...)` and `return f(...)` of
/// `body` whose call is a region call to a callee returning a tuple with
/// `int` items:
///
/// ```text
/// if intact:              # and, for a dispatch, `local is NS['g']`
///     a, b_tmp = COPY(...)
/// else:
///     a_tmp, b_tmp = f(...)
///     a = __pycc_rconv_i(a_tmp)
/// b = b_tmp
/// ```
///
/// so the copy's call is unpacked where it is made and takes the copy's
/// multi-value twin (`pycc_mir::multi_return`): no tuple is built. The
/// generic arm unpacks CPython's object as before and converts each `int`
/// item; a `return` converts the whole tuple (`__pycc_rconv_<shape>`). An
/// object item is bound once, after the `if`, where the statement was: a
/// generator expression reading it is lifted only then
/// (`helpers::genexpr`). `temps` numbers the temporaries, one per item, so
/// each has one definition. Records the calls rewritten in `done`.
fn split_sites(
    body: &mut ruff_python_ast::Suite,
    calls: &BTreeMap<(u32, u32), CallRewrite>,
    temps: &mut usize,
    done: &mut BTreeSet<(u32, u32)>,
) {
    let old = std::mem::take(body);
    for mut stmt in old {
        match &mut stmt {
            Stmt::If(s) => {
                split_sites(&mut s.body, calls, temps, done);
                for clause in &mut s.elif_else_clauses {
                    split_sites(&mut clause.body, calls, temps, done);
                }
            }
            Stmt::While(s) => {
                split_sites(&mut s.body, calls, temps, done);
                split_sites(&mut s.orelse, calls, temps, done);
            }
            Stmt::For(s) => {
                split_sites(&mut s.body, calls, temps, done);
                split_sites(&mut s.orelse, calls, temps, done);
            }
            Stmt::With(s) => split_sites(&mut s.body, calls, temps, done),
            Stmt::Try(s) => {
                split_sites(&mut s.body, calls, temps, done);
                for handler in &mut s.handlers {
                    let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                    split_sites(&mut h.body, calls, temps, done);
                }
                split_sites(&mut s.orelse, calls, temps, done);
                split_sites(&mut s.finalbody, calls, temps, done);
            }
            _ => {}
        }
        let found = match &stmt {
            Stmt::Assign(a) => match (a.targets.as_slice(), a.value.as_ref()) {
                ([Expr::Tuple(t)], Expr::Call(site)) => calls
                    .get(&key(site.range))
                    .filter(|c| {
                        c.ints.as_ref().is_some_and(|ints| ints.len() == t.elts.len())
                            && t.elts.iter().all(|e| matches!(e, Expr::Name(_)))
                    })
                    .map(|c| (Use::Unpack, site.clone(), c)),
                _ => None,
            },
            Stmt::Return(r) => match r.value.as_deref() {
                Some(Expr::Call(site)) => calls
                    .get(&key(site.range))
                    .filter(|c| c.ints.is_some() && c.conv.is_some())
                    .map(|c| (Use::Return, site.clone(), c)),
                _ => None,
            },
            _ => None,
        };
        let Some((usage, site, rewrite)) = found else {
            body.push(stmt);
            continue;
        };
        done.insert(key(site.range));
        // Each object item goes to a temporary both arms bind.
        let mut objects: Vec<(String, TextRange, String)> = Vec::new();
        if let (Use::Unpack, Stmt::Assign(a)) = (&usage, &mut stmt)
            && let [Expr::Tuple(t)] = a.targets.as_mut_slice()
        {
            let ints = rewrite.ints.as_ref().expect("checked above");
            for (i, elt) in t.elts.iter_mut().enumerate() {
                if ints[i] {
                    continue;
                }
                let Expr::Name(name) = elt else { unreachable!("checked above") };
                let temp = format!("__pycc_rk{temps}");
                *temps += 1;
                objects.push((name.id.to_string(), name.range, temp.clone()));
                name.id = temp.into();
                name.range = TextRange::empty(name.range.end());
            }
        }
        let with_value = |value: Expr| -> Stmt {
            let mut s = stmt.clone();
            match &mut s {
                Stmt::Assign(a) => *a.value = value,
                Stmt::Return(r) => r.value = Some(Box::new(value)),
                _ => unreachable!("an assignment or a return"),
            }
            s
        };
        let generic: Vec<Stmt> = match usage {
            Use::Unpack => {
                let ints = rewrite.ints.as_ref().expect("checked above");
                generic_unpack(with_value(Expr::Call(site.clone())), ints, &rewrite.callee, temps)
            }
            Use::Return => {
                let conv = rewrite.conv.as_ref().expect("checked above");
                vec![with_value(converted(conv, Expr::Call(site.clone()), &rewrite.callee, &site))]
            }
        };
        let mut chosen: Vec<Stmt> = generic;
        for (i, target) in rewrite.targets.iter().enumerate().rev() {
            // An unpack's temporary is named by its statement's start
            // offset (`pycc_hir`): each native arm starts at its own offset
            // inside the call, where no statement starts, so its temporary
            // keeps the copy's native tuple type instead of sharing the
            // generic arm's object one.
            let mut native = with_value(native_call(&site, target));
            let at = site.range.end() - ruff_text_size::TextSize::from(1 + i as u32);
            if at > stmt.range().start()
                && let Stmt::Assign(a) = &mut native
            {
                a.range = TextRange::new(at, a.range.end());
            }
            chosen = vec![Stmt::If(StmtIf {
                node_index: AtomicNodeIndex::default(),
                range: stmt.range(),
                test: Box::new(arm_test(&site, rewrite, target)),
                body: vec![native].into(),
                elif_else_clauses: vec![ElifElseClause {
                    node_index: AtomicNodeIndex::default(),
                    range: stmt.range(),
                    test: None,
                    body: chosen.into(),
                }],
            })];
        }
        body.extend(chosen);
        for (name, at, temp) in objects {
            let mut assign = parse_helper(&format!("{name} = {temp}\n"), stmt.range());
            if let Some(Stmt::Assign(a)) = assign.first_mut() {
                if let [Expr::Name(target)] = a.targets.as_mut_slice() {
                    target.range = at;
                }
                if let Expr::Name(value) = a.value.as_mut() {
                    value.range = TextRange::empty(at.end());
                }
            }
            body.extend(assign);
        }
    }
}

/// `value`, the generic arm's call of `site`, converted by
/// `__pycc_rconv_<conv>`, which names `callee` in its `TypeError`.
fn converted(conv: &str, value: Expr, callee: &str, site: &ExprCall) -> Expr {
    let at = added(site);
    let what = super::super::super::build::string(&format!("{callee}() returned "), at);
    let mut converted = call_name(&format!("__pycc_rconv_{conv}"), vec![value, what], at);
    if let Expr::Call(c) = &mut converted {
        c.range = site.range;
    }
    converted
}

/// The generic arm of an unpack `assign` (`a, b = f(...)`, `f` spelled
/// `callee`): each `int` item (`ints`) is unpacked into a temporary and
/// then converted. The target keeps the element's range; the temporary
/// starts where the element ends (a comma, a parenthesis, a space or `=`,
/// where no name starts), and the conversion's name where the statement
/// ends.
fn generic_unpack(mut assign: Stmt, ints: &[bool], callee: &str, temps: &mut usize) -> Vec<Stmt> {
    let Stmt::Assign(a) = &mut assign else {
        unreachable!("an unpacking assignment")
    };
    let range = a.range;
    let mut converts = Vec::new();
    if let [Expr::Tuple(t)] = a.targets.as_mut_slice() {
        for (i, elt) in t.elts.iter_mut().enumerate() {
            if !ints[i] {
                continue;
            }
            let temp = format!("__pycc_ru{temps}");
            *temps += 1;
            let Expr::Name(name) = elt else { unreachable!("checked by split_sites") };
            converts.push((name.id.to_string(), name.range, temp.clone(), i));
            name.id = temp.into();
            name.range = TextRange::empty(name.range.end());
        }
    }
    let mut out = vec![assign];
    for (name, at, temp, item) in converts {
        let mut assign = parse_helper(
            &format!("{name} = __pycc_rconv_i({temp}, '{callee}() returned a tuple whose item {item} is ')\n"),
            range,
        );
        if let Some(Stmt::Assign(a)) = assign.first_mut() {
            if let [Expr::Name(target)] = a.targets.as_mut_slice() {
                target.range = at;
            }
            if let Expr::Call(c) = a.value.as_mut() {
                if let Expr::Name(f) = c.func.as_mut() {
                    f.range = TextRange::empty(range.end());
                }
                if let Some(Expr::Name(arg)) = c.arguments.args.first_mut() {
                    arg.range = TextRange::empty(at.end());
                }
                if let Some(text) = c.arguments.args.get_mut(1) {
                    ruff_python_ast::relocate::relocate_expr(text, TextRange::empty(range.end()));
                }
            }
        }
        out.extend(assign);
    }
    out
}

/// The copy of `lift` that `copy` plans.
fn make_copy(lift: &StmtFunctionDef, copy: &Copy) -> StmtFunctionDef {
    let mut clone = lift.clone();
    let range = lift.name.range();
    clone.name = ruff_python_ast::Identifier::new(copy.name.clone(), range);
    for (arg, (_, kind)) in clone.parameters.args.iter_mut().zip(&copy.params) {
        arg.parameter.annotation = Some(Box::new(load(kind_annotation(*kind), arg.parameter.range())));
    }
    let returns = parse_helper(&format!("x: {}\n", copy.result), range);
    if let Some(Stmt::AnnAssign(a)) = returns.into_iter().next() {
        clone.returns = Some(a.annotation);
    }
    let mut done = BTreeSet::new();
    split_sites(&mut clone.body, &copy.calls, &mut 0, &mut done);
    let calls = Calls { calls: &copy.calls, skip: &done };
    for stmt in &mut clone.body {
        calls.visit_stmt(stmt);
    }
    if copy.cyclic {
        let mut wrapped = parse_helper(
            "if not __pycc_region_enter():\n    raise __pycc_fallback_builtins.RecursionError('maximum recursion depth exceeded')\ntry:\n    pass\nfinally:\n    __pycc_region_leave()\n",
            range,
        );
        if let Some(Stmt::Try(t)) = wrapped.last_mut() {
            t.body = std::mem::take(&mut clone.body);
        }
        clone.body = wrapped.into();
    }
    clone
}

/// The entry guard of a member's lift: when its region is intact and the
/// copy's `int` and `str` parameters hold exactly those types, the lift
/// returns its copy's result.
fn guard(lift: &mut StmtFunctionDef, copy: &Copy) {
    let mut tests = vec![format!("__pycc_region_intact({})", copy.region)];
    for (name, kind) in &copy.params {
        match kind {
            Kind::Int => tests.push(format!("type({name}) is __pycc_specialize_int")),
            Kind::Str => tests.push(format!("type({name}) is __pycc_specialize_str")),
            _ => {}
        }
    }
    let args: Vec<&str> = copy.params.iter().map(|(n, _)| n.as_str()).collect();
    let source = format!("if {}:\n    return {}({})\n", tests.join(" and "), copy.name, args.join(", "));
    let mut guard = parse_helper(&source, lift.name.range());
    super::super::mark_tail_call(&mut guard);
    let at = usize::from(matches!(lift.body.first(), Some(Stmt::Expr(e)) if e.value.is_string_literal_expr()));
    for (offset, stmt) in guard.into_iter().enumerate() {
        lift.body.insert(at + offset, stmt);
    }
}

/// The conversion of the generic arm of a region call whose callee's copy
/// returns `shape` (`i` an `int`, `o` an object, per item of a tuple). Its
/// second argument says what returned the value (`skip() returned `); a
/// value of another type raises `TypeError` naming it (`skip() returned
/// str, the compiled caller needs int`, docs/design/selected-regions.md
/// section 4.4). An `int` (or `tuple`) subclass is converted to the plain
/// type.
fn conversion(shape: &str) -> String {
    let b = super::super::BUILTINS;
    let fail = |needs: &str| {
        format!("    raise {b}.TypeError(what + {b}.type(x).__name__ + ', the compiled caller needs {needs}')\n")
    };
    if shape == "i" {
        return format!(
            "def __pycc_rconv_i(x: object, what: str) -> int:\n    if {b}.isinstance(x, {b}.int):\n        return x\n{}",
            fail("int")
        );
    }
    let names: Vec<String> = (0..shape.len()).map(|i| format!("a{i}")).collect();
    let types: Vec<&str> = shape.chars().map(|c| if c == 'i' { "int" } else { "object" }).collect();
    let items: Vec<String> = shape
        .chars()
        .enumerate()
        .map(|(i, c)| {
            if c == 'i' {
                format!("__pycc_rconv_i(a{i}, what + 'a tuple whose item {i} is ')")
            } else {
                format!("a{i}")
            }
        })
        .collect();
    let tuple = format!("tuple[{}]", types.join(", "));
    format!(
        "def __pycc_rconv_{shape}(x: object, what: str) -> {tuple}:\n    if {b}.isinstance(x, {b}.tuple) and {b}.len(x) == {n}:\n        {} = x\n        return {}\n{}",
        names.join(", "),
        items.join(", "),
        fail(&tuple),
        n = shape.len(),
    )
}

/// Writes `plan` into `module`.
pub(crate) fn apply(module: &mut ModModule, plan: Plan) -> Vec<Specialized> {
    if plan.copies.is_empty() {
        return Vec::new();
    }
    let by_lift: BTreeMap<&str, &Copy> = plan.copies.iter().map(|c| (c.lift.as_str(), c)).collect();
    let mut out = Vec::new();
    let old = std::mem::take(&mut module.body);
    let mut body: Vec<Stmt> = Vec::with_capacity(old.len() + plan.copies.len());
    for stmt in old {
        let Stmt::FunctionDef(mut def) = stmt else {
            body.push(stmt);
            continue;
        };
        match by_lift.get(def.name.as_str()) {
            Some(copy) => {
                let clone = make_copy(&def, copy);
                out.push(Specialized {
                    qualname: format!("region {}", copy.report),
                    range: def.range(),
                    params: copy
                        .params
                        .iter()
                        .filter_map(|(n, k)| match k {
                            Kind::Int => Some((n.clone(), "int")),
                            Kind::Str => Some((n.clone(), "str")),
                            _ => None,
                        })
                        .collect(),
                    restored_builtins: false,
                    copy: None,
                });
                guard(&mut def, copy);
                body.push(Stmt::FunctionDef(def));
                body.push(Stmt::FunctionDef(clone));
            }
            None => body.push(Stmt::FunctionDef(def)),
        }
    }
    let range = TextRange::default();
    for shape in &plan.convs {
        body.extend(parse_helper(&conversion(shape), range));
    }
    super::super::insert_guard_types(&mut body, &["int", "str"]);
    for (k, globals, patterns) in &plan.regions {
        let globals: Vec<&str> = globals.iter().map(String::as_str).collect();
        let patterns: Vec<&str> = patterns.iter().map(String::as_str).collect();
        body.push(arm(*k, &globals, &patterns, range));
    }
    module.body = body.into();
    out
}
