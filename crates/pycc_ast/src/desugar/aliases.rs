//! `sys` stream names, the module-level aliases of them, and module-level
//! lambdas turned into `def`s.

use super::INFER_MARKER;
use super::build::{call, load, subscript};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{
    AtomicNodeIndex, Expr, ExprContext, Identifier, Parameter, ParameterWithDefault, Parameters,
    Stmt, StmtFunctionDef, StmtReturn,
};
use ruff_text_size::TextRange;
use std::collections::HashMap;

/// The local names a module binds to `sys` and to `sys.stdin`/`sys.stdout`
/// by a top-level import and never rebinds.
#[derive(Default)]
pub(crate) struct SysNames {
    pub(crate) sys: Option<String>,
    pub(crate) stdin: Option<String>,
    pub(crate) stdout: Option<String>,
    /// Whether `sys.stdin.buffer` reads as text: only through an alias
    /// whose every call is converted to numbers
    /// (`super::stdin_bytes::buffer_reads_as_text`).
    pub(crate) buffer_text: bool,
}

/// A `sys` stream operation the desugaring replaces with a native builtin.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SysFn {
    Readline,
    Read,
    Readlines,
    Write,
    Flush,
    SetRecursionLimit,
}

impl SysFn {
    /// The native builtin a call of this operation becomes.
    pub(crate) fn builtin(self) -> &'static str {
        match self {
            SysFn::Readline => "__pycc_stdin_readline",
            SysFn::Read => "__pycc_stdin_read",
            SysFn::Readlines => "__pycc_stdin_readlines",
            SysFn::Write => "__pycc_stdout_write",
            SysFn::Flush => "__pycc_stdout_flush",
            SysFn::SetRecursionLimit => "__pycc_setrecursionlimit",
        }
    }

    /// The annotation of the alias `def`'s one parameter, if it takes one.
    fn param_annotation(self) -> Option<&'static str> {
        match self {
            SysFn::Write => Some("str"),
            SysFn::SetRecursionLimit => Some("int"),
            _ => None,
        }
    }

    /// The alias `def`'s return annotation: the operation's own result.
    fn returns(self, range: TextRange) -> Expr {
        match self {
            SysFn::Readline | SysFn::Read => load("str", range),
            SysFn::Readlines => subscript(
                load("list", range),
                load("str", range),
                ExprContext::Load,
                range,
            ),
            SysFn::Write => load("int", range),
            SysFn::Flush | SysFn::SetRecursionLimit => load("None", range),
        }
    }
}

/// Every name a statement in `body` binds other than by an import.
pub(crate) fn rebound_names(body: &[Stmt]) -> HashMap<String, usize> {
    struct Collect(HashMap<String, usize>);
    impl Collect {
        fn add(&mut self, name: &str) {
            *self.0.entry(name.to_string()).or_default() += 1;
        }
    }
    impl<'a> Visitor<'a> for Collect {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => {
                    self.add(def.name.as_str());
                    for p in def.parameters.iter() {
                        self.add(p.name().as_str());
                    }
                }
                Stmt::ClassDef(def) => self.add(def.name.as_str()),
                Stmt::Global(global) => {
                    for name in &global.names {
                        self.add(name.as_str());
                    }
                }
                _ => {}
            }
            visitor::walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Name(name) if name.ctx != ExprContext::Load => self.add(name.id.as_str()),
                Expr::Lambda(lambda) => {
                    for p in lambda.parameters.iter().flat_map(|p| p.iter()) {
                        self.add(p.name().as_str());
                    }
                }
                _ => {}
            }
            visitor::walk_expr(self, expr);
        }
        fn visit_comprehension(&mut self, comp: &'a ruff_python_ast::Comprehension) {
            visitor::walk_comprehension(self, comp);
        }
    }
    let mut collect = Collect(HashMap::new());
    for stmt in body {
        collect.visit_stmt(stmt);
    }
    collect.0
}

impl SysNames {
    pub(crate) fn collect(body: &[Stmt]) -> SysNames {
        let mut names = SysNames::default();
        for stmt in body {
            match stmt {
                Stmt::Import(import) => {
                    for alias in &import.names {
                        if alias.name.as_str() == "sys" {
                            names.sys =
                                Some(alias.asname.as_ref().unwrap_or(&alias.name).to_string());
                        }
                    }
                }
                Stmt::ImportFrom(import)
                    if import.level == 0
                        && import.module.as_ref().is_some_and(|m| m.as_str() == "sys") =>
                {
                    for alias in &import.names {
                        let bound = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                        match alias.name.as_str() {
                            "stdin" => names.stdin = Some(bound),
                            "stdout" => names.stdout = Some(bound),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        let rebound = rebound_names(body);
        for slot in [&mut names.sys, &mut names.stdin, &mut names.stdout] {
            if slot.as_ref().is_some_and(|n| rebound.contains_key(n)) {
                *slot = None;
            }
        }
        names.buffer_text = super::stdin_bytes::buffer_reads_as_text(body, &names);
        names
    }

    fn is(&self, expr: &Expr, slot: &Option<String>) -> bool {
        matches!((expr, slot), (Expr::Name(n), Some(s)) if n.id.as_str() == s)
    }

    /// `sys.stdin`, `stdin`; `sys.stdin.buffer` and `stdin.buffer` only
    /// when [`SysNames::buffer_text`]: it reads `bytes`, which print,
    /// index and compare unlike `str`.
    pub(crate) fn is_stdin(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Attribute(a) if a.attr.as_str() == "buffer" && self.buffer_text => {
                self.is_stdin(&a.value)
            }
            Expr::Attribute(a) if a.attr.as_str() == "stdin" => self.is(&a.value, &self.sys),
            other => self.is(other, &self.stdin),
        }
    }

    fn is_stdout(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Attribute(a) if a.attr.as_str() == "stdout" => self.is(&a.value, &self.sys),
            other => self.is(other, &self.stdout),
        }
    }

    /// The `sys` operation `expr` (a callee) names.
    pub(crate) fn classify(&self, expr: &Expr) -> Option<SysFn> {
        let Expr::Attribute(attr) = expr else {
            return None;
        };
        let value = attr.value.as_ref();
        match attr.attr.as_str() {
            "readline" if self.is_stdin(value) => Some(SysFn::Readline),
            "read" if self.is_stdin(value) => Some(SysFn::Read),
            "readlines" if self.is_stdin(value) => Some(SysFn::Readlines),
            "write" if self.is_stdout(value) => Some(SysFn::Write),
            "flush" if self.is_stdout(value) => Some(SysFn::Flush),
            "setrecursionlimit" if self.is(value, &self.sys) => Some(SysFn::SetRecursionLimit),
            _ => None,
        }
    }
}

fn parameter(name: &str, annotation: Option<&str>, range: TextRange) -> ParameterWithDefault {
    ParameterWithDefault {
        range,
        node_index: AtomicNodeIndex::default(),
        parameter: Parameter {
            range,
            node_index: AtomicNodeIndex::default(),
            name: Identifier::new(name, range),
            annotation: annotation.map(|a| Box::new(load(a, range))),
        },
        default: None,
    }
}

fn parameters(args: Vec<ParameterWithDefault>, range: TextRange) -> Parameters {
    Parameters {
        range,
        node_index: AtomicNodeIndex::default(),
        posonlyargs: Vec::new().into(),
        args: args.into(),
        vararg: None,
        kwonlyargs: Vec::new().into(),
        kwarg: None,
    }
}

fn inferred_def(name: &str, params: Parameters, body: Expr, range: TextRange) -> Stmt {
    annotated_def(name, params, body, load(INFER_MARKER, range), range)
}

fn annotated_def(
    name: &str,
    params: Parameters,
    body: Expr,
    returns: Expr,
    range: TextRange,
) -> Stmt {
    Stmt::FunctionDef(StmtFunctionDef {
        node_index: AtomicNodeIndex::default(),
        range,
        is_async: false,
        decorator_list: Vec::new().into(),
        name: Identifier::new(name, range),
        type_params: None,
        parameters: Box::new(params),
        returns: Some(Box::new(returns)),
        body: vec![Stmt::Return(StmtReturn {
            node_index: AtomicNodeIndex::default(),
            range,
            value: Some(Box::new(body)),
        })]
        .into(),
    })
}

/// Turns each top-level `name = lambda ...` and `name = sys.stdin.readline`
/// (or another [`SysFn`]) into a `def`, when `name` is bound nowhere else.
pub(crate) fn module_level_defs(body: &mut [Stmt], sys: &SysNames) {
    let rebound = rebound_names(body);
    for stmt in body.iter_mut() {
        let Stmt::Assign(assign) = stmt else {
            continue;
        };
        let [Expr::Name(target)] = assign.targets.as_slice() else {
            continue;
        };
        let name = target.id.to_string();
        if rebound.get(&name).copied().unwrap_or(0) != 1 {
            continue;
        }
        let range = assign.range;
        let replacement = match assign.value.as_ref() {
            Expr::Lambda(lambda) => {
                if lambda.parameters.as_ref().is_some_and(|p| {
                    p.vararg.is_some() || p.kwarg.is_some() || !p.kwonlyargs.is_empty()
                }) {
                    continue;
                }
                let params = lambda
                    .parameters
                    .as_deref()
                    .cloned()
                    .unwrap_or_else(|| parameters(Vec::new(), range));
                inferred_def(&name, params, (*lambda.body).clone(), range)
            }
            callee => {
                let Some(op) = sys.classify(callee) else {
                    continue;
                };
                let annotation = op.param_annotation();
                let args: Vec<_> = annotation.iter().map(|_| "_pycc_arg0").collect();
                let call_expr = call(
                    callee.clone(),
                    args.iter().map(|a| load(a, range)).collect(),
                    range,
                );
                let params = parameters(
                    args.iter()
                        .map(|a| parameter(a, annotation, range))
                        .collect(),
                    range,
                );
                annotated_def(&name, params, call_expr, op.returns(range), range)
            }
        };
        *stmt = replacement;
    }
}

/// Whether any expression in `body` reads the name `id`.
fn reads_name(body: &[Stmt], id: &str) -> bool {
    struct Find<'n>(&'n str, bool);
    impl<'a> Visitor<'a> for Find<'_> {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Name(name) = expr
                && name.id.as_str() == self.0
            {
                self.1 = true;
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut find = Find(id, false);
    for stmt in body {
        find.visit_stmt(stmt);
    }
    find.1
}

/// Drops `import sys` / `from sys import stdin, stdout` bindings the
/// rewrite left unread, so a program that only used them for I/O needs no
/// embedded CPython.
pub(crate) fn drop_unused_sys_imports(body: &mut Vec<Stmt>, sys: &SysNames) {
    // Non-strict fallback code (source run by CPython's `exec`, see the
    // driver's `fallback` module) may read the import where this walk
    // cannot see it.
    if reads_name(body, "__pycc_fallback_ns") {
        return;
    }
    let unread =
        |name: &Option<String>, body: &[Stmt]| name.as_ref().is_some_and(|n| !reads_name(body, n));
    let drop_sys = unread(&sys.sys, body);
    let drop_stdin = unread(&sys.stdin, body);
    let drop_stdout = unread(&sys.stdout, body);
    body.retain_mut(|stmt| match stmt {
        Stmt::Import(import) => {
            import.names.retain(|alias| {
                !(drop_sys
                    && alias.name.as_str() == "sys"
                    && sys.sys.as_deref()
                        == Some(alias.asname.as_ref().unwrap_or(&alias.name).as_str()))
            });
            !import.names.is_empty()
        }
        Stmt::ImportFrom(import)
            if import.level == 0 && import.module.as_ref().is_some_and(|m| m.as_str() == "sys") =>
        {
            import.names.retain(|alias| match alias.name.as_str() {
                "stdin" => !drop_stdin,
                "stdout" => !drop_stdout,
                _ => true,
            });
            !import.names.is_empty()
        }
        _ => true,
    });
}

/// `sys.stdin` itself (iterated by a `for` loop) as the list of its lines.
pub(crate) fn stdin_lines(range: TextRange) -> Expr {
    call(load(SysFn::Readlines.builtin(), range), Vec::new(), range)
}

/// The builtin functions a module may bind a short name to (`P = print`,
/// `rg = range`).
const ALIASABLE_BUILTINS: &[&str] = &[
    "abs",
    "all",
    "any",
    "bin",
    "chr",
    "dict",
    "divmod",
    "enumerate",
    "filter",
    "float",
    "hex",
    "input",
    "int",
    "len",
    "list",
    "map",
    "max",
    "min",
    "oct",
    "ord",
    "pow",
    "print",
    "range",
    "reversed",
    "round",
    "set",
    "sorted",
    "str",
    "sum",
    "tuple",
    "zip",
];

/// Replaces a top-level `X = <builtin>` (both names bound nowhere else)
/// by the builtin itself at every read of `X`.
pub(crate) fn builtin_aliases(body: &mut Vec<Stmt>) {
    use ruff_python_ast::visitor::transformer::{self, Transformer};
    let rebound = rebound_names(body);
    let mut aliases: HashMap<String, String> = HashMap::new();
    body.retain(|stmt| {
        let Stmt::Assign(assign) = stmt else {
            return true;
        };
        let ([Expr::Name(target)], Expr::Name(value)) =
            (assign.targets.as_slice(), assign.value.as_ref())
        else {
            return true;
        };
        let (alias, builtin) = (target.id.as_str(), value.id.as_str());
        if !ALIASABLE_BUILTINS.contains(&builtin)
            || rebound.get(alias).copied().unwrap_or(0) != 1
            || rebound.contains_key(builtin)
            || alias.starts_with("__")
        {
            return true;
        }
        aliases.insert(alias.to_string(), builtin.to_string());
        false
    });
    if aliases.is_empty() {
        return;
    }
    struct Substitute(HashMap<String, String>);
    impl Transformer for Substitute {
        fn visit_expr(&self, expr: &mut Expr) {
            if let Expr::Name(name) = expr
                && let Some(builtin) = self.0.get(name.id.as_str())
            {
                name.id = builtin.as_str().into();
            }
            transformer::walk_expr(self, expr);
        }
    }
    let substitute = Substitute(aliases);
    for stmt in body.iter_mut() {
        substitute.visit_stmt(stmt);
    }
}

/// `class C(object):` is `class C:` (every class derives from `object`),
/// unless the module binds `object` itself.
pub(crate) fn drop_object_bases(body: &mut [Stmt]) {
    if rebound_names(body).contains_key("object") {
        return;
    }
    for stmt in body.iter_mut() {
        if let Stmt::ClassDef(class) = stmt
            && let Some(arguments) = class.arguments.as_mut()
            && arguments.keywords.is_empty()
        {
            let mut args: Vec<Expr> = arguments.args.to_vec();
            args.retain(|base| !matches!(base, Expr::Name(n) if n.id.as_str() == "object"));
            if args.len() != arguments.args.len() {
                arguments.args = args.into();
            }
        }
    }
}
