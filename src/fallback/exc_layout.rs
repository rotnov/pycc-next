//! v0.10 accept 5: exception classes with a native layout.
//!
//! An exception class with members of its own runs in CPython with its
//! methods lifted (`lift`): CPython makes the class from its statement, so
//! `args`, the traceback, `__str__`, pickling and every class-statement
//! hook are CPython's own. What it adds is a *layout*: the attributes its
//! methods store from a parameter annotated `int`, `float`, `str` or `bool`
//! (`self.code = code`) live in a native instance instead of the object's
//! `__dict__`, and the lifted methods read and store them natively.
//!
//! * A hidden compiled class `__pycc_excl_C__` declares the layout (an
//!   `__init__` storing each attribute, never called); a layout class over
//!   another one derives from that one's hidden class, so a subclass
//!   instance is an instance of its base's layout, as in `lift`'s sibling
//!   form.
//! * The statement's bases become `*__pycc_exc_bases__("__pycc_excl_C__",
//!   b1, b2)`, which the shim answers with `(template, b1, b2)`: the
//!   template is a PEP 697 type over the exception base (`exc_templates.c`),
//!   whose objects carry the native instance and whose descriptors read
//!   and store its attributes.
//! * Each lifted method gets a second parameter, `__pycc_v__`, annotated
//!   with the hidden class: the forwarder passes the receiver for it too,
//!   and the boundary hands the body the native instance the object
//!   carries. The body reads `__pycc_v__.code` where it read `self.code`;
//!   everything else about `self` (`super()`, `self.args`, a method call,
//!   `str(self)`) stays the CPython object.
//!
//! An attribute is in the layout only when every store of it the class
//! makes is of that form, it is no class attribute and its name is not
//! private; a class defining `__setattr__`, `__getattribute__`,
//! `__delattr__` or `__slots__` gets no layout.

use super::*;
use pycc_ast::{Expr, StmtClassDef};

/// The prefix of a hidden layout class's name (`__pycc_excl_C__`).
pub(crate) const LAYOUT_PREFIX: &str = "__pycc_excl_";
/// The view parameter of a lifted method of a layout class.
const VIEW_PARAM: &str = "__pycc_v__";
/// The shim's hook answering a layout class's bases (`exc_templates.c`).
const BASES_HOOK: &str = "__pycc_exc_bases__";

/// What an exception class's instances carry.
#[derive(Debug, Clone)]
enum State {
    /// No native layout in its chain.
    Plain,
    /// The layout `layouts[i]` declares (its own, or its nearest
    /// ancestor's).
    Layout(usize),
    /// An ancestor may have one this class cannot extend (another module's,
    /// or two): it gets none of its own.
    Blocked,
}

/// One exception class with a native layout.
#[derive(Debug, Clone)]
pub(super) struct Layout {
    /// The class statement (a top-level statement index).
    pub(super) owner: usize,
    /// The hidden class declaring it.
    pub(super) hidden: String,
    /// The hidden class of the layout class it derives from.
    base: Option<String>,
    /// Its own attributes and their types, in first-store order.
    own: Vec<(String, String)>,
    /// Every attribute in it, its base's included.
    fields: BTreeSet<String>,
}

/// The name of the hidden class of exception class `class`.
fn hidden_name(class: &str) -> String {
    format!("{LAYOUT_PREFIX}{class}__")
}

impl Plan {
    /// The module's exception classes with a native layout, in statement
    /// order: top-level classes deriving from a builtin exception class or
    /// from another such class, running in CPython with their methods
    /// lifted (split), with at least one attribute of their own in it.
    pub(super) fn exc_layouts(&self) -> Vec<Layout> {
        // The module's exception classes so far, and what each carries.
        let mut states: BTreeMap<String, State> = BTreeMap::new();
        let mut layouts: Vec<Layout> = Vec::new();
        for index in 0..self.top {
            let Kind::Class(name) = &self.stmts[index].kind else {
                continue;
            };
            let range = &self.stmts[index].range;
            let Some(class) = self.class_source(range) else {
                continue;
            };
            // The state each exception base gives: a builtin one none, one
            // of this module its own, another module's `Blocked` when its
            // chain may have a layout. A base of any other kind is no
            // exception class's.
            let mut exception = false;
            let mut inherited: Vec<usize> = Vec::new();
            let mut blocked = false;
            for base in class.bases() {
                let Expr::Name(base) = base else {
                    continue;
                };
                let base = base.id.as_str();
                if let Some(state) = states.get(base) {
                    exception = true;
                    match state {
                        State::Plain => {}
                        State::Layout(at) => inherited.push(*at),
                        State::Blocked => blocked = true,
                    }
                } else if let Some(may_layout) = self.package_exceptions.get(base) {
                    exception = true;
                    blocked |= *may_layout;
                } else if pycc_hir::BUILTIN_EXCEPTION_CLASSES.contains(&base) {
                    exception = true;
                }
            }
            if !exception {
                continue;
            }
            inherited.dedup();
            let state = if blocked || inherited.len() > 1 {
                State::Blocked
            } else {
                let base = inherited.first().copied();
                let fields = base.map(|at| layouts[at].fields.clone()).unwrap_or_default();
                let own: Vec<(String, String)> = if self.is_split(index) {
                    fields_of(&class)
                        .into_iter()
                        .filter(|(field, _)| !fields.contains(field))
                        .collect()
                } else {
                    Vec::new()
                };
                if own.is_empty() {
                    // Its instances are its base's layout's (a class with no
                    // attribute of its own, or CPython's alone).
                    base.map_or(State::Plain, State::Layout)
                } else {
                    let mut all = fields;
                    all.extend(own.iter().map(|(field, _)| field.clone()));
                    layouts.push(Layout {
                        owner: index,
                        hidden: hidden_name(name),
                        base: base.map(|at| layouts[at].hidden.clone()),
                        own,
                        fields: all,
                    });
                    State::Layout(layouts.len() - 1)
                }
            };
            states.insert(name.clone(), state);
        }
        layouts
    }

    /// The class statement at `range` of the source, parsed on its own.
    fn class_source(&self, range: &Range<u32>) -> Option<StmtClassDef> {
        let text = self.source.get(range.start as usize..range.end as usize)?;
        let module = pycc_parser::parse_all(text).ok()?;
        match module.body.into_iter().next()? {
            Stmt::ClassDef(class) => Some(class),
            _ => None,
        }
    }

    /// The layout of statement `owner`, when it has one.
    pub(super) fn layout_of(&self, owner: usize) -> Option<Layout> {
        self.exc_layouts()
            .into_iter()
            .find(|layout| layout.owner == owner)
    }

    /// Whether lift `unit` compiles with a view parameter: a method of a
    /// layout class (directly in its body) taking a receiver.
    pub(super) fn lift_has_view(&self, unit: usize) -> bool {
        let Some(lift) = self.lift(unit) else {
            return false;
        };
        lift.receiver.is_some()
            && !self.fallback.contains_key(&unit)
            && self.layout_of(lift.owner).is_some_and(|layout| {
                let Kind::Class(class) = &self.stmts[layout.owner].kind else {
                    return false;
                };
                lift.qualname
                    .strip_prefix(class.as_str())
                    .and_then(|rest| rest.strip_prefix('.'))
                    .is_some_and(|method| !method.contains('.'))
            })
    }

    /// The edits of lift `unit` with a view parameter: its signature with
    /// the view after the receiver, and each `self.attr` of the layout read
    /// through the view.
    pub(super) fn view_edits(&self, unit: usize) -> Vec<(Range<u32>, String)> {
        let (Some(lift), Some(layout)) = (
            self.lift(unit),
            self.lift(unit).and_then(|lift| self.layout_of(lift.owner)),
        ) else {
            return Vec::new();
        };
        let Some(receiver) = lift.receiver else {
            return Vec::new();
        };
        let (list, at, lines) = &lift.signature;
        let mut list = list.clone();
        list.insert(receiver + 1, format!("{VIEW_PARAM}: {}", layout.hidden));
        let mut edits = lift.edits.clone();
        edits[*at].1 = format!("({}{}) -> object", list.join(", "), "\n".repeat(*lines));
        edits.extend(
            lift.receiver_attrs
                .iter()
                .filter(|(_, attr)| layout.fields.contains(attr))
                .map(|(range, _)| (range.clone(), VIEW_PARAM.to_string())),
        );
        edits
    }

    /// The compiled source declaring every layout (the preamble's).
    pub(super) fn layout_classes(&self) -> String {
        let mut out = String::new();
        for layout in self.exc_layouts() {
            let base = layout
                .base
                .as_ref()
                .map(|base| format!("({base})"))
                .unwrap_or_default();
            let params: Vec<String> = layout
                .own
                .iter()
                .map(|(field, ty)| format!("{field}: {ty}"))
                .collect();
            let stores: Vec<String> = layout
                .own
                .iter()
                .map(|(field, _)| format!("self.{field} = {field}"))
                .collect();
            out.push_str(&format!(
                "class {}{base}:\n    def __init__(self, {}) -> None: {}\n",
                layout.hidden,
                params.join(", "),
                stores.join("; ")
            ));
        }
        out
    }

    /// The replacement of layout class `owner`'s positional bases in its
    /// CPython statement: `(start, end, text)` source offsets.
    pub(super) fn layout_bases_edit(&self, owner: usize) -> Option<(usize, usize, String)> {
        let layout = self.layout_of(owner)?;
        let range = &self.stmts[owner].range;
        let class = self.class_source(range)?;
        let bases = class.bases();
        let first = pycc_ast::expr_range(bases.first()?);
        let last = pycc_ast::expr_range(bases.last()?);
        let (start, end) = (
            range.start as usize + first.start as usize,
            range.start as usize + last.end as usize,
        );
        Some((
            start,
            end,
            format!(
                "*{BASES_HOOK}(\"{}\", {})",
                layout.hidden,
                &self.source[start..end]
            ),
        ))
    }
}

/// The scalar annotations a layout attribute may take.
const SCALARS: [&str; 4] = ["int", "float", "str", "bool"];

/// The attributes of exception class `class` that go in its layout, with
/// their types, in first-store order (the module doc's rule).
pub(super) fn fields_of(class: &StmtClassDef) -> Vec<(String, String)> {
    let mut excluded: BTreeSet<String> = BTreeSet::new();
    for stmt in &class.body {
        match stmt {
            Stmt::FunctionDef(def) => {
                if matches!(
                    def.name.as_str(),
                    "__setattr__" | "__getattribute__" | "__delattr__"
                ) {
                    return Vec::new();
                }
                excluded.insert(def.name.as_str().to_string());
            }
            Stmt::ClassDef(inner) => {
                excluded.insert(inner.name.as_str().to_string());
            }
            _ => {
                let bound = names::bound_names(std::slice::from_ref(stmt));
                if bound.contains("__slots__") {
                    return Vec::new();
                }
                excluded.extend(bound);
            }
        }
    }
    let mut found: Vec<(String, String)> = Vec::new();
    let mut poisoned: BTreeSet<String> = BTreeSet::new();
    for stmt in &class.body {
        let Stmt::FunctionDef(def) = stmt else {
            continue;
        };
        let receiverless = def.decorator_list.iter().any(|decorator| {
            matches!(&decorator.expression, Expr::Name(n)
                if matches!(n.id.as_str(), "staticmethod" | "classmethod"))
        });
        let parameters = &def.parameters;
        let mut ordered = parameters.posonlyargs.iter().chain(&parameters.args);
        let Some(receiver) = ordered.next() else {
            continue;
        };
        if receiverless {
            continue;
        }
        let receiver = receiver.parameter.name.as_str();
        let typed: BTreeMap<&str, &str> = parameters
            .posonlyargs
            .iter()
            .chain(&parameters.args)
            .chain(&parameters.kwonlyargs)
            .skip(1)
            .filter_map(|p| match p.parameter.annotation.as_deref() {
                Some(Expr::Name(ty)) if SCALARS.contains(&ty.id.as_str()) => {
                    Some((p.parameter.name.as_str(), ty.id.as_str()))
                }
                _ => None,
            })
            .collect();
        let rebound = names::bound_names(&def.body);
        let mut scan = Stores {
            receiver,
            typed: &typed,
            rebound: &rebound,
            depth: 0,
            found: &mut found,
            poisoned: &mut poisoned,
        };
        for stmt in &def.body {
            scan.stmt(stmt);
        }
    }
    let mut out: Vec<(String, String)> = Vec::new();
    for (field, ty) in found {
        if poisoned.contains(&field)
            || excluded.contains(&field)
            || (field.starts_with("__") && !field.ends_with("__"))
            || out.iter().any(|(seen, _)| *seen == field)
        {
            continue;
        }
        out.push((field, ty));
    }
    out
}

/// The scan of one method's stores into its receiver's attributes.
struct Stores<'a> {
    receiver: &'a str,
    typed: &'a BTreeMap<&'a str, &'a str>,
    rebound: &'a BTreeSet<String>,
    depth: usize,
    found: &'a mut Vec<(String, String)>,
    poisoned: &'a mut BTreeSet<String>,
}

impl Stores<'_> {
    /// `attr` when `target` is `receiver.attr`.
    fn attr_of<'e>(&self, target: &'e Expr) -> Option<&'e str> {
        match target {
            Expr::Attribute(attr) => match &*attr.value {
                Expr::Name(name) if name.id.as_str() == self.receiver => Some(attr.attr.as_str()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Records a store of `value` into `target`.
    fn store(&mut self, target: &Expr, value: Option<&Expr>, annotation: Option<&Expr>) {
        match target {
            Expr::Tuple(tuple) => {
                for elt in &tuple.elts {
                    self.store(elt, None, None);
                }
            }
            Expr::List(list) => {
                for elt in &list.elts {
                    self.store(elt, None, None);
                }
            }
            Expr::Starred(starred) => self.store(&starred.value, None, None),
            _ => {
                let Some(attr) = self.attr_of(target) else {
                    return;
                };
                let ty = match (annotation, value) {
                    (Some(Expr::Name(ty)), _) if SCALARS.contains(&ty.id.as_str()) => {
                        Some(ty.id.as_str())
                    }
                    (None, Some(Expr::Name(param)))
                        if self.depth == 0 && !self.rebound.contains(param.id.as_str()) =>
                    {
                        self.typed.get(param.id.as_str()).copied()
                    }
                    _ => None,
                };
                match ty {
                    Some(ty) if self.depth == 0 => {
                        if let Some((_, seen)) = self.found.iter().find(|(f, _)| f == attr) {
                            if seen != ty {
                                self.poisoned.insert(attr.to_string());
                            }
                        } else {
                            self.found.push((attr.to_string(), ty.to_string()));
                        }
                    }
                    _ => {
                        self.poisoned.insert(attr.to_string());
                    }
                }
            }
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assign(assign) => {
                let single = assign.targets.len() == 1;
                for target in &assign.targets {
                    self.store(target, single.then_some(&*assign.value), None);
                }
            }
            Stmt::AnnAssign(assign) => {
                self.store(&assign.target, assign.value.as_deref(), Some(&assign.annotation));
            }
            Stmt::FunctionDef(def) => {
                self.depth += 1;
                for inner in &def.body {
                    self.stmt(inner);
                }
                self.depth -= 1;
            }
            Stmt::ClassDef(class) => {
                self.depth += 1;
                for inner in &class.body {
                    self.stmt(inner);
                }
                self.depth -= 1;
            }
            Stmt::For(s) => {
                self.store(&s.target, None, None);
                self.body(&s.body);
                self.body(&s.orelse);
            }
            Stmt::While(s) => {
                self.body(&s.body);
                self.body(&s.orelse);
            }
            Stmt::If(s) => {
                self.body(&s.body);
                for clause in &s.elif_else_clauses {
                    self.body(&clause.body);
                }
            }
            Stmt::With(s) => {
                for item in &s.items {
                    if let Some(target) = &item.optional_vars {
                        self.store(target, None, None);
                    }
                }
                self.body(&s.body);
            }
            Stmt::Try(s) => {
                self.body(&s.body);
                for handler in &s.handlers {
                    let pycc_ast::ExceptHandler::ExceptHandler(h) = handler;
                    self.body(&h.body);
                }
                self.body(&s.orelse);
                self.body(&s.finalbody);
            }
            Stmt::Match(s) => {
                for case in &s.cases {
                    self.body(&case.body);
                }
            }
            _ => {}
        }
    }

    fn body(&mut self, body: &[Stmt]) {
        for stmt in body {
            self.stmt(stmt);
        }
    }
}
