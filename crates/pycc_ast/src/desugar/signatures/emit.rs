//! The code a bound call becomes: an in-place call with the flat argument
//! list, or a helper (CPython's binding at run time, in Python).

use super::{Arg, Param, Sig};
use crate::desugar::Fresh;
use crate::desugar::build::{attribute, call, call_name, load, string};
use ruff_python_ast::{AtomicNodeIndex, DictItem, Expr, ExprContext, ExprDict, ExprList, Stmt};
use ruff_text_size::TextRange;
use std::fmt::Write as _;

/// How the callee receives the receiver of a method call.
#[derive(Clone)]
pub(super) enum Receiver {
    /// A function: there is none.
    None,
    /// `value.m(...)` of a method taking it as its first parameter.
    Bound(Expr),
    /// `value.m(...)` of a static method: evaluated, not passed.
    Unbound(Expr),
    /// `C(...)`: `__init__`'s receiver, filled by the constructor.
    Implicit,
    /// `C.m(...)` of a static method through its class: nothing is
    /// evaluated or passed.
    Class(String),
}

/// One call to bind.
pub(super) struct Emitter<'a> {
    pub(super) fresh: &'a Fresh,
    pub(super) sig: &'a Sig,
    pub(super) range: TextRange,
    /// The callee: the function or class name, or the method name.
    pub(super) callee: String,
    pub(super) receiver: Receiver,
    /// The callee's static result type, for a helper that only raises.
    pub(super) returns: Option<Expr>,
    /// The module-qualified name of the callee (`__main__.f`), for the
    /// message of a keyword a call passes twice.
    pub(super) module_qualname: String,
}

/// Where a bound parameter's value comes from.
#[derive(Clone)]
enum Source {
    /// Value `i` of the call (in evaluation order, the receiver first).
    Value(usize),
    Default(Expr),
}

/// A statically bound call.
struct Bound {
    positional: Vec<Source>,
    extras: Vec<usize>,
    kwonly: Vec<Source>,
    extra_keywords: Vec<(String, usize)>,
}

impl Emitter<'_> {
    /// The call's values in evaluation order: the receiver (when there is
    /// a value), then every argument.
    fn values(&self, args: &[Arg]) -> Vec<Expr> {
        let mut values = Vec::new();
        if let Receiver::Bound(e) | Receiver::Unbound(e) = &self.receiver {
            values.push(e.clone());
        }
        values.extend(args.iter().map(|a| a.value().clone()));
        values
    }

    /// The index of the first argument value.
    fn first_arg(&self) -> usize {
        usize::from(matches!(
            self.receiver,
            Receiver::Bound(_) | Receiver::Unbound(_)
        ))
    }

    /// Whether the receiver fills the first positional parameter.
    fn receiver_fills(&self) -> bool {
        matches!(self.receiver, Receiver::Bound(_) | Receiver::Implicit)
    }

    fn qual(&self) -> &str {
        &self.sig.qualname
    }

    /// Binds a call without `*seq`/`**mapping`, or reports CPython's
    /// message.
    fn bind_static(&self, args: &[Arg]) -> Result<Bound, String> {
        let sig = self.sig;
        let positional: Vec<&Param> = sig.positional().collect();
        let count = positional.len();
        let mut slots: Vec<Option<Source>> = vec![None; count];
        let mut kwonly: Vec<Option<Source>> = vec![None; sig.kwonly.len()];
        let mut extras = Vec::new();
        let mut extra_keywords = Vec::new();
        let mut given = 0;
        if self.receiver_fills() && count > 0 {
            // The receiver itself is never an argument value of the flat
            // call; a placeholder marks the slot filled.
            slots[0] = Some(Source::Value(usize::MAX));
            given = 1;
        }
        let first = self.first_arg();
        for (i, arg) in args.iter().enumerate() {
            if let Arg::Positional(_) = arg {
                if given < count {
                    slots[given] = Some(Source::Value(first + i));
                } else {
                    extras.push(first + i);
                }
                given += 1;
            }
        }
        let keyword_names: Vec<&str> = args
            .iter()
            .filter_map(|a| match a {
                Arg::Keyword(name, _) => Some(name.as_str()),
                _ => None,
            })
            .collect();
        for (i, arg) in args.iter().enumerate() {
            let Arg::Keyword(name, _) = arg else { continue };
            if let Some(index) = positional
                .iter()
                .skip(sig.posonly.len())
                .position(|p| p.name == *name)
                .map(|k| k + sig.posonly.len())
            {
                if slots[index].is_some() {
                    return Err(multiple(self.qual(), name));
                }
                slots[index] = Some(Source::Value(first + i));
            } else if let Some(index) = sig.kwonly.iter().position(|p| p.name == *name) {
                kwonly[index] = Some(Source::Value(first + i));
            } else if sig.kwarg.is_some() {
                extra_keywords.push((name.clone(), first + i));
            } else {
                let posonly: Vec<&str> = keyword_names
                    .iter()
                    .copied()
                    .filter(|k| sig.posonly.iter().any(|p| p.name == *k))
                    .collect();
                if !posonly.is_empty() {
                    return Err(posonly_message(self.qual(), &posonly.join(", ")));
                }
                return Err(unexpected(self.qual(), name));
            }
        }
        if given > count && sig.vararg.is_none() {
            let kwonly_given = kwonly.iter().filter(|s| s.is_some()).count();
            return Err(too_many(sig, given, kwonly_given));
        }
        let defaults = positional.iter().filter(|p| p.default.is_some()).count();
        let mut missing = Vec::new();
        for (slot, param) in slots.iter_mut().zip(&positional) {
            if slot.is_none() {
                match &param.default {
                    Some(d) => *slot = Some(Source::Default(d.clone())),
                    None => missing.push(param.name.as_str()),
                }
            }
        }
        let _ = defaults;
        if !missing.is_empty() {
            return Err(missing_message(self.qual(), &missing, "positional"));
        }
        let mut missing = Vec::new();
        for (slot, param) in kwonly.iter_mut().zip(&sig.kwonly) {
            if slot.is_none() {
                match &param.default {
                    Some(d) => *slot = Some(Source::Default(d.clone())),
                    None => missing.push(param.name.as_str()),
                }
            }
        }
        if !missing.is_empty() {
            return Err(missing_message(self.qual(), &missing, "keyword-only"));
        }
        Ok(Bound {
            positional: slots.into_iter().map(|s| s.expect("bound")).collect(),
            extras,
            kwonly: kwonly.into_iter().map(|s| s.expect("bound")).collect(),
            extra_keywords,
        })
    }

    /// A call without `*seq`/`**mapping`: `(replacement, helpers)`.
    pub(super) fn fixed(&self, args: &[Arg]) -> (Expr, Vec<Stmt>) {
        let values = self.values(args);
        let bound = match self.bind_static(args) {
            Ok(bound) => bound,
            Err(message) => return self.raising(&values, &message),
        };
        // The flat call's arguments as value indices (defaults: none), in
        // flat order, to tell whether writing it in place keeps the
        // evaluation order.
        let mut order: Vec<usize> = Vec::new();
        let receiver_value = matches!(self.receiver, Receiver::Bound(_) | Receiver::Unbound(_));
        if receiver_value {
            order.push(0);
        }
        let skip = usize::from(self.receiver_fills());
        let index = |s: &Source| match s {
            Source::Value(i) if *i != usize::MAX => Some(*i),
            _ => None,
        };
        order.extend(bound.positional.iter().skip(skip).filter_map(index));
        order.extend(&bound.extras);
        order.extend(bound.kwonly.iter().filter_map(index));
        order.extend(bound.extra_keywords.iter().map(|(_, i)| *i));
        let in_order = order.windows(2).all(|w| w[0] < w[1])
            || values
                .iter()
                .skip(usize::from(receiver_value))
                .all(crate::desugar::is_simple);
        if in_order {
            let flat = self.flat_args(&bound, &|i| values[i].clone());
            return (self.target_call(values.first().cloned(), flat), Vec::new());
        }
        let names: Vec<String> = (0..values.len()).map(|_| self.fresh.name()).collect();
        let flat = self.flat_args(&bound, &|i| load(&names[i], self.range));
        let receiver = receiver_value.then(|| load(&names[0], self.range));
        let body = vec![Stmt::Return(ruff_python_ast::StmtReturn {
            node_index: AtomicNodeIndex::default(),
            range: self.range,
            value: Some(Box::new(self.target_call(receiver, flat))),
        })];
        let name = self.helper_name("call");
        let helper = self.helper(&name, &names, body, None);
        (call_name(&name, values, self.range), vec![helper])
    }

    /// The flat argument list of `bound`, each value read through `value`.
    fn flat_args(&self, bound: &Bound, value: &dyn Fn(usize) -> Expr) -> Vec<Expr> {
        let range = self.range;
        let read = |s: &Source| match s {
            Source::Value(i) => value(*i),
            Source::Default(d) => d.clone(),
        };
        let mut flat: Vec<Expr> = bound
            .positional
            .iter()
            .skip(usize::from(self.receiver_fills()))
            .map(read)
            .collect();
        if self.sig.vararg.is_some() {
            flat.push(Expr::List(ExprList {
                node_index: AtomicNodeIndex::default(),
                range,
                elts: bound.extras.iter().map(|i| value(*i)).collect(),
                ctx: ExprContext::Load,
            }));
        }
        flat.extend(bound.kwonly.iter().map(read));
        if self.sig.kwarg.is_some() {
            flat.push(Expr::Dict(ExprDict {
                node_index: AtomicNodeIndex::default(),
                range,
                items: bound
                    .extra_keywords
                    .iter()
                    .map(|(k, i)| DictItem {
                        key: Some(string(k, range)),
                        value: value(*i),
                    })
                    .collect(),
            }));
        }
        flat
    }

    /// The call of the callee with `flat`, through `receiver` (the
    /// receiver's value) for a method.
    fn target_call(&self, receiver: Option<Expr>, flat: Vec<Expr>) -> Expr {
        let func = match (&self.receiver, receiver) {
            (Receiver::Bound(_) | Receiver::Unbound(_), Some(r)) => {
                attribute(r, &self.callee, self.range)
            }
            (Receiver::Class(class), _) => {
                attribute(load(class, self.range), &self.callee, self.range)
            }
            _ => load(&self.callee, self.range),
        };
        call(func, flat, self.range)
    }

    fn helper_name(&self, kind: &str) -> String {
        format!(
            "_pycc_{kind}{}",
            self.fresh.name().trim_start_matches("_pycc")
        )
    }

    /// A module-level helper `name(params) -> returns` (inferred when
    /// `returns` is `None`).
    fn helper(
        &self,
        name: &str,
        params: &[String],
        body: Vec<Stmt>,
        returns: Option<Expr>,
    ) -> Stmt {
        let range = self.range;
        Stmt::FunctionDef(ruff_python_ast::StmtFunctionDef {
            node_index: AtomicNodeIndex::default(),
            range,
            is_async: false,
            decorator_list: Vec::new().into(),
            name: ruff_python_ast::Identifier::new(name, range),
            type_params: None,
            parameters: Box::new(ruff_python_ast::Parameters {
                range,
                node_index: AtomicNodeIndex::default(),
                posonlyargs: Vec::new().into(),
                args: params
                    .iter()
                    .map(|p| ruff_python_ast::ParameterWithDefault {
                        range,
                        node_index: AtomicNodeIndex::default(),
                        parameter: ruff_python_ast::Parameter {
                            range,
                            node_index: AtomicNodeIndex::default(),
                            name: ruff_python_ast::Identifier::new(p, range),
                            annotation: None,
                        },
                        default: None,
                    })
                    .collect::<Vec<_>>()
                    .into(),
                vararg: None,
                kwonlyargs: Vec::new().into(),
                kwarg: None,
            }),
            returns: Some(Box::new(
                returns.unwrap_or_else(|| load(crate::desugar::INFER_MARKER, range)),
            )),
            body: body.into(),
        })
    }

    /// A call that cannot bind: a helper evaluating every value and raising
    /// `TypeError(message)`.
    fn raising(&self, values: &[Expr], message: &str) -> (Expr, Vec<Stmt>) {
        let names: Vec<String> = (0..values.len()).map(|_| self.fresh.name()).collect();
        let body = vec![raise_type_error(string(message, self.range), self.range)];
        let name = self.helper_name("badcall");
        let helper = self.helper(&name, &names, body, self.returns.clone());
        (call_name(&name, values.to_vec(), self.range), vec![helper])
    }

    /// A call with `*seq` and/or `**mapping`: a helper binding at run time.
    pub(super) fn dynamic(&self, args: &[Arg]) -> (Expr, Vec<Stmt>) {
        let values = self.values(args);
        let name = self.helper_name("bind");
        let (source, raises) = self.dynamic_source(&name, args, values.len());
        let mut helper = parse_helper(&source, self.range);
        if let Some(Stmt::FunctionDef(def)) = helper.first_mut() {
            def.name = ruff_python_ast::Identifier::new(&name, self.range);
            // A helper that always raises has no result to infer.
            if raises && let Some(returns) = &self.returns {
                def.returns = Some(Box::new(returns.clone()));
            }
        }
        (call_name(&name, values, self.range), helper)
    }

    /// The Python source of the run-time binding helper `name` (see the
    /// module documentation of `super`), and whether it always raises (a
    /// parameter nothing in the call can fill: its missing-argument check
    /// raised, and the source ends there).
    fn dynamic_source(&self, name: &str, args: &[Arg], value_count: usize) -> (String, bool) {
        let sig = self.sig;
        let first = self.first_arg();
        let qual = self.qual();
        let v = |i: usize| format!("_pycc_a{i}");
        let positional: Vec<&Param> = sig.positional().collect();
        let count = positional.len();
        let mut out = String::new();
        let params: Vec<String> = (0..value_count).map(v).collect();
        let _ = writeln!(
            out,
            "def {name}({}) -> {}:",
            params.join(", "),
            crate::desugar::INFER_MARKER
        );
        // Positionals: a static prefix, then the run-time rest.
        let receiver_slots = usize::from(self.receiver_fills());
        let mut pre = receiver_slots;
        let mut prefix: Vec<usize> = Vec::new();
        let mut rest_parts: Vec<String> = Vec::new();
        let mut starred = false;
        for (i, arg) in args.iter().enumerate() {
            match arg {
                Arg::Positional(_) if !starred && pre < count => {
                    prefix.push(first + i);
                    pre += 1;
                }
                Arg::Positional(_) => rest_parts.push(format!("[{}]", v(first + i))),
                Arg::Star(_) => {
                    starred = true;
                    // A comprehension rather than `list(...)`: it takes a
                    // tuple or any iterator as well as a list.
                    rest_parts.push(format!("[_pycc_x for _pycc_x in {}]", v(first + i)));
                }
                _ => {}
            }
        }
        let has_rest = !rest_parts.is_empty();
        if has_rest {
            let _ = writeln!(out, "    _pycc_rest = {}", rest_parts.join(" + "));
            let _ = writeln!(out, "    _pycc_n = {pre} + len(_pycc_rest)");
        }
        // `n > s` as Python, for a positional slot `s` past the prefix.
        let filled_by_rest =
            |s: usize| -> Option<String> { has_rest.then(|| format!("_pycc_n > {s}")) };
        let mappings: Vec<usize> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| matches!(a, Arg::DoubleStar(_)))
            .map(|(i, _)| first + i)
            .collect();
        let keywords: Vec<(String, usize)> = args
            .iter()
            .enumerate()
            .filter_map(|(i, a)| match a {
                Arg::Keyword(name, _) => Some((name.clone(), first + i)),
                _ => None,
            })
            .collect();
        let has_map = !mappings.is_empty();
        let module_qual = &self.module_qualname;
        if has_map {
            let m0 = v(mappings[0]);
            let _ = writeln!(
                out,
                "    _pycc_kw = {{_pycc_k: {m0}[_pycc_k] for _pycc_k in {m0}}}"
            );
            for m in &mappings[1..] {
                let m = v(*m);
                let _ = writeln!(out, "    for _pycc_k in {m}:");
                let _ = writeln!(out, "        if _pycc_k in _pycc_kw:");
                let _ = writeln!(
                    out,
                    "            raise TypeError(\"{module_qual}() got multiple values for keyword argument '\" + _pycc_k + \"'\")"
                );
                let _ = writeln!(out, "        _pycc_kw[_pycc_k] = {m}[_pycc_k]");
            }
            for (name, _) in &keywords {
                let _ = writeln!(out, "    if {} in _pycc_kw:", py_str(name));
                let _ = writeln!(
                    out,
                    "        raise TypeError({})",
                    py_str(&format!(
                        "{module_qual}() got multiple values for keyword argument '{name}'"
                    ))
                );
            }
        }
        let regular_index = |name: &str| -> Option<usize> {
            sig.regular
                .iter()
                .position(|p| p.name == name)
                .map(|k| k + sig.posonly.len())
        };
        let is_kwonly = |name: &str| sig.kwonly.iter().any(|p| p.name == name);
        let is_posonly = |name: &str| sig.posonly.iter().any(|p| p.name == name);
        // Keyword errors, in the order CPython meets them.
        let static_posonly: Vec<&str> = keywords
            .iter()
            .map(|(n, _)| n.as_str())
            .filter(|n| is_posonly(n))
            .collect();
        for (name, _) in &keywords {
            if let Some(s) = regular_index(name) {
                if s < pre {
                    let _ = writeln!(
                        out,
                        "    raise TypeError({})",
                        py_str(&multiple(qual, name))
                    );
                } else if let Some(cond) = filled_by_rest(s) {
                    let _ = writeln!(out, "    if {cond}:");
                    let _ = writeln!(
                        out,
                        "        raise TypeError({})",
                        py_str(&multiple(qual, name))
                    );
                }
            } else if is_kwonly(name) || sig.kwarg.is_some() {
            } else if !static_posonly.is_empty() {
                let _ = writeln!(
                    out,
                    "    raise TypeError({})",
                    py_str(&posonly_message(qual, &static_posonly.join(", ")))
                );
            } else {
                let _ = writeln!(
                    out,
                    "    raise TypeError({})",
                    py_str(&unexpected(qual, name))
                );
            }
        }
        let named: Vec<&str> = sig
            .regular
            .iter()
            .chain(&sig.kwonly)
            .map(|p| p.name.as_str())
            .collect();
        if has_map && (!named.is_empty() || sig.kwarg.is_none()) {
            let _ = writeln!(out, "    for _pycc_k in _pycc_kw:");
            let mut first_branch = true;
            for name in &named {
                let kw = if first_branch { "if" } else { "elif" };
                first_branch = false;
                let _ = writeln!(out, "        {kw} _pycc_k == {}:", py_str(name));
                match regular_index(name) {
                    Some(s) if s < pre => {
                        let _ = writeln!(
                            out,
                            "            raise TypeError({})",
                            py_str(&multiple(qual, name))
                        );
                    }
                    Some(s) if has_rest => {
                        let _ = writeln!(out, "            if _pycc_n > {s}:");
                        let _ = writeln!(
                            out,
                            "                raise TypeError({})",
                            py_str(&multiple(qual, name))
                        );
                    }
                    _ => {
                        let _ = writeln!(out, "            pass");
                    }
                }
            }
            if sig.kwarg.is_none() {
                let indent = if first_branch {
                    "        "
                } else {
                    "            "
                };
                if !first_branch {
                    let _ = writeln!(out, "        else:");
                }
                if !sig.posonly.is_empty() {
                    let test = sig
                        .posonly
                        .iter()
                        .map(|p| format!("_pycc_j == {}", py_str(&p.name)))
                        .collect::<Vec<_>>()
                        .join(" or ");
                    let _ = writeln!(out, "{indent}if {}:", test.replace("_pycc_j", "_pycc_k"));
                    let _ = writeln!(
                        out,
                        "{indent}    raise TypeError({} + \", \".join([_pycc_j for _pycc_j in _pycc_kw if {test}]) + \"'\")",
                        py_str(&format!(
                            "{qual}() got some positional-only arguments passed as keyword arguments: '"
                        ))
                    );
                }
                let _ = writeln!(
                    out,
                    "{indent}raise TypeError({} + _pycc_k + \"'\")",
                    py_str(&format!("{qual}() got an unexpected keyword argument '"))
                );
            }
        }
        // Too many positional arguments.
        if sig.vararg.is_none() {
            let static_kwonly = keywords.iter().filter(|(n, _)| is_kwonly(n)).count();
            let given = if has_rest {
                "_pycc_n".to_string()
            } else {
                pre.to_string()
            };
            let check = if has_rest {
                Some(format!("_pycc_n > {count}"))
            } else if pre > count {
                Some("True".to_string())
            } else {
                None
            };
            if let Some(check) = check {
                let defaults = positional.iter().filter(|p| p.default.is_some()).count();
                let (takes, plural) = if defaults > 0 {
                    (format!("from {} to {count}", count - defaults), "s")
                } else {
                    (count.to_string(), if count == 1 { "" } else { "s" })
                };
                let head = format!("{qual}() takes {takes} positional argument{plural} but ");
                let _ = writeln!(out, "    if {check}:");
                let mut kwo = static_kwonly.to_string();
                if has_map {
                    for p in &sig.kwonly {
                        let _ = write!(kwo, " + (1 if {} in _pycc_kw else 0)", py_str(&p.name));
                    }
                }
                let _ = writeln!(out, "        _pycc_kwo = {kwo}");
                let _ = writeln!(out, "        if _pycc_kwo > 0:");
                let _ = writeln!(
                    out,
                    "            raise TypeError({} + str({given}) + \" positional argument\" + (\"s\" if {given} != 1 else \"\") + \" (and \" + str(_pycc_kwo) + \" keyword-only argument\" + (\"s\" if _pycc_kwo != 1 else \"\") + \") were given\")",
                    py_str(&head)
                );
                let _ = writeln!(
                    out,
                    "        raise TypeError({} + str({given}) + (\" was\" if {given} == 1 else \" were\") + \" given\")",
                    py_str(&head)
                );
            }
        }
        // Missing arguments.
        let provided = |name: &str, slot: Option<usize>| -> Option<String> {
            // `None`: always provided.
            if slot.is_some_and(|s| s < pre)
                || keywords.iter().any(|(n, _)| n == name && !is_posonly(n))
            {
                return None;
            }
            let mut terms = Vec::new();
            if let Some(s) = slot
                && let Some(cond) = filled_by_rest(s)
            {
                terms.push(cond);
            }
            if has_map && !is_posonly(name) {
                terms.push(format!("{} in _pycc_kw", py_str(name)));
            }
            Some(if terms.is_empty() {
                "False".to_string()
            } else {
                terms.join(" or ")
            })
        };
        for (kind, params) in [
            (
                "positional",
                positional
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(s, p)| (Some(s), p))
                    .collect::<Vec<_>>(),
            ),
            (
                "keyword-only",
                sig.kwonly.iter().map(|p| (None, p)).collect(),
            ),
        ] {
            let required: Vec<(Option<usize>, &Param)> = params
                .into_iter()
                .filter(|(_, p)| p.default.is_none())
                .collect();
            let checks: Vec<(String, &Param)> = required
                .iter()
                .filter_map(|(s, p)| provided(&p.name, *s).map(|c| (c, *p)))
                .collect();
            if checks.is_empty() {
                continue;
            }
            let _ = writeln!(out, "    _pycc_miss = []");
            for (cond, p) in &checks {
                if cond == "False" {
                    let _ = writeln!(
                        out,
                        "    _pycc_miss.append({})",
                        py_str(&format!("'{}'", p.name))
                    );
                } else {
                    let _ = writeln!(out, "    if not ({cond}):");
                    let _ = writeln!(
                        out,
                        "        _pycc_miss.append({})",
                        py_str(&format!("'{}'", p.name))
                    );
                }
            }
            let head = format!("{qual}() missing ");
            let _ = writeln!(out, "    if len(_pycc_miss) == 1:");
            let _ = writeln!(
                out,
                "        raise TypeError({} + _pycc_miss[0])",
                py_str(&format!("{head}1 required {kind} argument: "))
            );
            let _ = writeln!(out, "    if len(_pycc_miss) == 2:");
            let _ = writeln!(
                out,
                "        raise TypeError({} + _pycc_miss[0] + \" and \" + _pycc_miss[1])",
                py_str(&format!("{head}2 required {kind} arguments: "))
            );
            let _ = writeln!(out, "    if len(_pycc_miss) > 2:");
            let _ = writeln!(
                out,
                "        raise TypeError({} + str(len(_pycc_miss)) + {} + \", \".join(_pycc_miss[:-1]) + \", and \" + _pycc_miss[-1])",
                py_str(&head),
                py_str(&format!(" required {kind} arguments: "))
            );
        }
        // The flat arguments.
        let mut flat: Vec<String> = Vec::new();
        let mut prefix_iter = prefix.iter();
        for (s, p) in positional.iter().enumerate() {
            if s < receiver_slots {
                continue;
            }
            if s < pre {
                flat.push(v(*prefix_iter.next().expect("the prefix fills the slot")));
                continue;
            }
            let var = format!("_pycc_p{s}");
            let keyword = (!is_posonly(&p.name))
                .then(|| keywords.iter().find(|(n, _)| *n == p.name))
                .flatten();
            if let Some((_, i)) = keyword {
                flat.push(v(*i));
                continue;
            }
            let mut branches: Vec<(String, String)> = Vec::new();
            if let Some(cond) = filled_by_rest(s) {
                branches.push((cond, format!("_pycc_rest[{}]", s - pre)));
            }
            if has_map && !is_posonly(&p.name) {
                branches.push((
                    format!("{} in _pycc_kw", py_str(&p.name)),
                    format!("_pycc_kw[{}]", py_str(&p.name)),
                ));
            }
            let fallback = match &p.default {
                Some(_) => Some(format!("_pycc_default{s}")),
                None => None,
            };
            if !write_branches(&mut out, &var, &branches, fallback.as_deref()) {
                return (self.with_defaults(out), true);
            }
            flat.push(var);
        }
        if sig.vararg.is_some() {
            if has_rest {
                flat.push(format!("_pycc_rest[{}:]", count.saturating_sub(pre)));
            } else {
                flat.push("[]".to_string());
            }
        }
        for (k, p) in sig.kwonly.iter().enumerate() {
            if let Some((_, i)) = keywords.iter().find(|(n, _)| *n == p.name) {
                flat.push(v(*i));
                continue;
            }
            let var = format!("_pycc_k{k}");
            let mut branches = Vec::new();
            if has_map {
                branches.push((
                    format!("{} in _pycc_kw", py_str(&p.name)),
                    format!("_pycc_kw[{}]", py_str(&p.name)),
                ));
            }
            let fallback = p.default.as_ref().map(|_| format!("_pycc_kwdefault{k}"));
            if !write_branches(&mut out, &var, &branches, fallback.as_deref()) {
                return (self.with_defaults(out), true);
            }
            flat.push(var);
        }
        if sig.kwarg.is_some() {
            let extra: Vec<String> = keywords
                .iter()
                .filter(|(n, _)| !named.contains(&n.as_str()))
                .map(|(n, i)| format!("{}: {}", py_str(n), v(*i)))
                .collect();
            let _ = writeln!(out, "    _pycc_extra = {{{}}}", extra.join(", "));
            if has_map {
                let test = if named.is_empty() {
                    "True".to_string()
                } else {
                    named
                        .iter()
                        .map(|n| format!("_pycc_k != {}", py_str(n)))
                        .collect::<Vec<_>>()
                        .join(" and ")
                };
                let _ = writeln!(out, "    for _pycc_k in _pycc_kw:");
                let _ = writeln!(out, "        if {test}:");
                let _ = writeln!(out, "            _pycc_extra[_pycc_k] = _pycc_kw[_pycc_k]");
            }
            flat.push("_pycc_extra".to_string());
        }
        let target = match &self.receiver {
            Receiver::Bound(_) | Receiver::Unbound(_) => format!("{}.{}", v(0), self.callee),
            Receiver::Class(class) => format!("{class}.{}", self.callee),
            _ => self.callee.clone(),
        };
        let _ = writeln!(out, "    return {target}({})", flat.join(", "));
        (self.with_defaults(out), false)
    }

    /// Replaces the `_pycc_default{s}`/`_pycc_kwdefault{k}` placeholders
    /// with the defaults' text. A default is a literal or a hoisted global
    /// read, both of which render exactly.
    fn with_defaults(&self, mut source: String) -> String {
        let positional: Vec<&Param> = self.sig.positional().collect();
        for (s, p) in positional.iter().enumerate().rev() {
            if let Some(d) = &p.default {
                source = source.replace(&format!("_pycc_default{s}"), &render(d));
            }
        }
        for (k, p) in self.sig.kwonly.iter().enumerate().rev() {
            if let Some(d) = &p.default {
                source = source.replace(&format!("_pycc_kwdefault{k}"), &render(d));
            }
        }
        source
    }
}

/// `var = value` chosen by `branches` (`if cond: var = value`, then
/// `elif`s), else `fallback` -- or, with none, an unreachable raise (every
/// missing argument was reported before). `false` when nothing can fill
/// `var`: the helper raised already, and its source must end here.
fn write_branches(
    out: &mut String,
    var: &str,
    branches: &[(String, String)],
    fallback: Option<&str>,
) -> bool {
    if branches.is_empty() {
        match fallback {
            Some(f) => {
                let _ = writeln!(out, "    {var} = {f}");
            }
            None => {
                let _ = writeln!(out, "    raise TypeError(\"\")");
                return false;
            }
        }
        return true;
    }
    for (i, (cond, value)) in branches.iter().enumerate() {
        let kw = if i == 0 { "if" } else { "elif" };
        let _ = writeln!(out, "    {kw} {cond}:");
        let _ = writeln!(out, "        {var} = {value}");
    }
    let _ = writeln!(out, "    else:");
    match fallback {
        Some(f) => {
            let _ = writeln!(out, "        {var} = {f}");
        }
        None => {
            let _ = writeln!(out, "        raise TypeError(\"\")");
        }
    }
    true
}

/// A default as Python source.
fn render(default: &Expr) -> String {
    match default {
        Expr::Name(n) => n.id.to_string(),
        Expr::NumberLiteral(n) => match &n.value {
            ruff_python_ast::Number::Int(i) => i.to_string(),
            ruff_python_ast::Number::Float(f) => format!("{f:?}"),
            ruff_python_ast::Number::Complex { .. } => "0".to_string(),
        },
        Expr::StringLiteral(s) => py_str(s.value.to_str()),
        Expr::BooleanLiteral(b) => if b.value { "True" } else { "False" }.to_string(),
        Expr::NoneLiteral(_) => "None".to_string(),
        Expr::UnaryOp(u) => {
            let op = match u.op {
                ruff_python_ast::UnaryOp::USub => "-",
                ruff_python_ast::UnaryOp::UAdd => "+",
                ruff_python_ast::UnaryOp::Not => "not ",
                ruff_python_ast::UnaryOp::Invert => "~",
            };
            format!("{op}{}", render(&u.operand))
        }
        _ => "None".to_string(),
    }
}

/// A Python string literal of `text`.
fn py_str(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Parses a generated helper, placing it at `range` (its expressions take
/// the call's range; so do its statements and parameters).
pub(crate) fn parse_helper(source: &str, range: TextRange) -> Vec<Stmt> {
    let padding = "\n".repeat(range.start().to_usize());
    let text = format!("{padding}{source}");
    let parsed = ruff_python_parser::parse_module(&text)
        .expect("a generated binding helper is valid Python");
    let mut body: Vec<Stmt> = parsed.into_syntax().body.into_iter().collect();
    struct Relocate(TextRange);
    impl ruff_python_ast::visitor::transformer::Transformer for Relocate {
        fn visit_expr(&self, expr: &mut Expr) {
            ruff_python_ast::relocate::relocate_expr(expr, self.0);
        }
        // A diagnostic about a helper parameter (`_pycc_a0`) points at the
        // call too: its parsed range lies in the padded text, past the end
        // of the real source when the call is near it.
        fn visit_stmt(&self, stmt: &mut Stmt) {
            match stmt {
                Stmt::FunctionDef(s) => s.range = self.0,
                Stmt::ClassDef(s) => s.range = self.0,
                Stmt::Return(s) => s.range = self.0,
                Stmt::Delete(s) => s.range = self.0,
                Stmt::Assign(s) => s.range = self.0,
                Stmt::AugAssign(s) => s.range = self.0,
                Stmt::AnnAssign(s) => s.range = self.0,
                Stmt::TypeAlias(s) => s.range = self.0,
                Stmt::For(s) => s.range = self.0,
                Stmt::While(s) => s.range = self.0,
                Stmt::If(s) => s.range = self.0,
                Stmt::With(s) => s.range = self.0,
                Stmt::Match(s) => s.range = self.0,
                Stmt::Raise(s) => s.range = self.0,
                Stmt::Try(s) => s.range = self.0,
                Stmt::Assert(s) => s.range = self.0,
                Stmt::Import(s) => s.range = self.0,
                Stmt::ImportFrom(s) => s.range = self.0,
                Stmt::Global(s) => s.range = self.0,
                Stmt::Nonlocal(s) => s.range = self.0,
                Stmt::Expr(s) => s.range = self.0,
                Stmt::Pass(s) => s.range = self.0,
                Stmt::Break(s) => s.range = self.0,
                Stmt::Continue(s) => s.range = self.0,
                Stmt::IpyEscapeCommand(s) => s.range = self.0,
            }
            ruff_python_ast::visitor::transformer::walk_stmt(self, stmt);
        }
        fn visit_parameter(&self, parameter: &mut ruff_python_ast::Parameter) {
            parameter.range = self.0;
            parameter.name.range = self.0;
            ruff_python_ast::visitor::transformer::walk_parameter(self, parameter);
        }
    }
    let relocate = Relocate(range);
    for stmt in &mut body {
        ruff_python_ast::visitor::transformer::Transformer::visit_stmt(&relocate, stmt);
    }
    body
}

/// `raise TypeError(message)`.
fn raise_type_error(message: Expr, range: TextRange) -> Stmt {
    Stmt::Raise(ruff_python_ast::StmtRaise {
        node_index: AtomicNodeIndex::default(),
        range,
        exc: Some(Box::new(call_name("TypeError", vec![message], range))),
        cause: None,
    })
}

fn multiple(qual: &str, name: &str) -> String {
    format!("{qual}() got multiple values for argument '{name}'")
}

fn unexpected(qual: &str, name: &str) -> String {
    format!("{qual}() got an unexpected keyword argument '{name}'")
}

fn posonly_message(qual: &str, names: &str) -> String {
    format!("{qual}() got some positional-only arguments passed as keyword arguments: '{names}'")
}

/// CPython's `too_many_positional`.
fn too_many(sig: &Sig, given: usize, kwonly_given: usize) -> String {
    let count = sig.positional_count();
    let defaults = sig.positional().filter(|p| p.default.is_some()).count();
    let (takes, plural) = if defaults > 0 {
        (format!("from {} to {count}", count - defaults), "s")
    } else {
        (count.to_string(), if count == 1 { "" } else { "s" })
    };
    let qual = &sig.qualname;
    if kwonly_given > 0 {
        format!(
            "{qual}() takes {takes} positional argument{plural} but {given} positional argument{} \
             (and {kwonly_given} keyword-only argument{}) were given",
            if given == 1 { "" } else { "s" },
            if kwonly_given == 1 { "" } else { "s" }
        )
    } else {
        format!(
            "{qual}() takes {takes} positional argument{plural} but {given} {} given",
            if given == 1 { "was" } else { "were" }
        )
    }
}

/// CPython's `missing_arguments`.
fn missing_message(qual: &str, names: &[&str], kind: &str) -> String {
    let quoted: Vec<String> = names.iter().map(|n| format!("'{n}'")).collect();
    let list = match quoted.as_slice() {
        [one] => one.clone(),
        [a, b] => format!("{a} and {b}"),
        [init @ .., last] => format!("{}, and {last}", init.join(", ")),
        [] => String::new(),
    };
    format!(
        "{qual}() missing {} required {kind} argument{}: {list}",
        names.len(),
        if names.len() == 1 { "" } else { "s" }
    )
}
