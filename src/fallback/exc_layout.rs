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
//! makes is of that form, no class along its MRO binds the name at class
//! level (`OSError.errno`, a base's property: `exc_names` for the builtin
//! classes) and the name is neither private nor a dunder.
//!
//! Conservative throughout: a class gets or keeps a layout only when every
//! class of its MRO up to the builtin exception base is known to the
//! planner -- a class of the module or of the package whose body is on
//! `class_names`'s allowlist, over a builtin base that does not reduce
//! itself (`exc_names::builtin_reduces_itself`) -- and it has one base. A
//! layout some class carrying it would show otherwise than CPython (a
//! subclass binding one of its attributes, listing several bases or off the
//! allowlist, here or in another module of the package) is dropped with
//! the chain it extends, to a fixpoint.

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
    ///
    /// Conservative: a layout must be provably what CPython would show. A
    /// class carrying one (its own or a base's) whose MRO binds one of its
    /// attributes at class level (a subclass's property, a mixin's class
    /// attribute) or holds a class of unknown body makes the whole chain of
    /// layouts it carries CPython's again, and the planning starts over
    /// without them.
    pub(super) fn exc_layouts(&self) -> Vec<Layout> {
        let mut denied: BTreeSet<usize> = BTreeSet::new();
        loop {
            match self.layouts_denying(&denied) {
                Ok(layouts) => return layouts,
                Err(owners) => denied.extend(owners),
            }
        }
    }

    /// [`Self::exc_layouts`] with the classes at `denied` given no layout
    /// of their own: the layouts, or the owners of a chain of layouts a
    /// class cannot carry as CPython would (never one of `denied`).
    fn layouts_denying(&self, denied: &BTreeSet<usize>) -> Result<Vec<Layout>, Vec<usize>> {
        // The module's exception classes so far, and what each carries.
        let mut states: BTreeMap<String, State> = BTreeMap::new();
        // Every top-level class so far -> the names bound at class level
        // along its MRO, `None` when a class in it is of unknown body.
        let mut chains: BTreeMap<String, Option<BTreeSet<String>>> = BTreeMap::new();
        let mut layouts: Vec<Layout> = Vec::new();
        for index in 0..self.top {
            let Kind::Class(name) = &self.stmts[index].kind else {
                continue;
            };
            let range = &self.stmts[index].range;
            let Some(class) = self.class_source(range) else {
                chains.insert(name.clone(), None);
                continue;
            };
            // The state each exception base gives: a builtin one none, one
            // of this module its own, another module's `Blocked` when its
            // chain may have a layout. A base of any other kind is no
            // exception class's. Each base also gives the names its MRO
            // binds, which the layout leaves to CPython.
            let mut exception = false;
            let mut inherited: Vec<usize> = Vec::new();
            let mut blocked = false;
            // `None` for a body off the allowlist (a class keyword such as
            // `metaclass=`, which may refuse the template, included).
            let mut bound: Option<BTreeSet<String>> = class_names(&class);
            for base in class.bases() {
                let Expr::Name(base) = base else {
                    bound = None;
                    continue;
                };
                let base = base.id.as_str();
                let names: Option<BTreeSet<String>> = if let Some(chain) = chains.get(base) {
                    chain.clone()
                } else if let Some(package) = self.package_exceptions.get(base) {
                    package.names.clone()
                } else if pycc_hir::BUILTIN_EXCEPTION_CLASSES.contains(&base) {
                    // One reducing itself (`OSError.__reduce__`) is no base
                    // the template's `__reduce__` is known to extend.
                    (!exc_names::builtin_reduces_itself(base)).then(|| {
                        exc_names::builtin_exception_names(base)
                            .map(str::to_string)
                            .collect()
                    })
                } else if base == "object" {
                    Some(BTreeSet::new())
                } else {
                    None
                };
                match (&mut bound, names) {
                    (Some(bound), Some(names)) => bound.extend(names),
                    _ => bound = None,
                }
                if let Some(state) = states.get(base) {
                    exception = true;
                    match state {
                        State::Plain => {}
                        State::Layout(at) => inherited.push(*at),
                        State::Blocked => blocked = true,
                    }
                } else if let Some(package) = self.package_exceptions.get(base) {
                    exception = true;
                    blocked |= package.may_layout;
                } else if pycc_hir::BUILTIN_EXCEPTION_CLASSES.contains(&base) {
                    exception = true;
                }
            }
            chains.insert(name.clone(), bound.clone());
            if !exception {
                continue;
            }
            inherited.dedup();
            // A class of several bases over a layout: another base's layout
            // (or its solid base) may conflict with it, which CPython
            // refuses -- every layout it inherits goes.
            if class.bases().len() > 1 && !inherited.is_empty() {
                return Err(self.chains_of(&layouts, &inherited));
            }
            let state = if blocked || inherited.len() > 1 {
                State::Blocked
            } else {
                let base = inherited.first().copied();
                let fields = base.map(|at| layouts[at].fields.clone()).unwrap_or_default();
                // A class whose MRO holds a class of unknown body gets no
                // layout of its own: a binding there could take its
                // attributes' stores.
                // Nor does one of several bases: a template over the first
                // exception base may not share a solid base with the others
                // (`class C(KeyError, OSError)`).
                let own: Vec<(String, String)> = if let (true, false, true, Some(bound)) = (
                    self.is_split(index),
                    denied.contains(&index),
                    class.bases().len() == 1,
                    &bound,
                ) {
                    fields_of(&class, bound)
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
            // The layout its instances carry must be the first binding of
            // each of its attributes along its MRO.
            if let State::Layout(at) = state {
                // Its own MRO, and the subclasses other modules of the
                // package derive from it.
                let overrides = self.package_overrides.get(name);
                let shadowed = self.package_multi.contains(name.as_str())
                    || match &bound {
                    Some(bound) => layouts[at].fields.iter().any(|field| {
                        bound.contains(field) || overrides.is_some_and(|names| names.contains(field))
                    }),
                    None => true,
                };
                if shadowed {
                    return Err(self.chains_of(&layouts, &[at]));
                }
            }
            states.insert(name.clone(), state);
        }
        Ok(layouts)
    }

    /// The owners of layouts `at` and of every layout they extend.
    fn chains_of(&self, layouts: &[Layout], at: &[usize]) -> Vec<usize> {
        let mut owners = Vec::new();
        for &at in at {
            let mut next = Some(at);
            while let Some(at) = next {
                owners.push(layouts[at].owner);
                next = layouts[at].base.as_ref().and_then(|hidden| {
                    layouts.iter().position(|layout| &layout.hidden == hidden)
                });
            }
        }
        owners
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
                // Not `__new__`, `__init_subclass__` or `__class_getitem__`:
                // CPython makes them static or class methods, so their first
                // parameter is no instance.
                lift.qualname
                    .strip_prefix(class.as_str())
                    .and_then(|rest| rest.strip_prefix('.'))
                    .is_some_and(|method| {
                        !method.contains('.')
                            && !matches!(
                                method,
                                "__new__" | "__init_subclass__" | "__class_getitem__"
                            )
                    })
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

/// The names class `class` binds in its own body, or `None` when the body
/// is not one a layout is known to be equivalent under -- an allowlist,
/// not a list of hooks: no class keyword or decorator, and a body of plain
/// functions, static and class methods (no other decorator), no dunder but
/// `__init__`, `__str__` and `__repr__`, and class attributes bound to plain
/// literals (a docstring and `pass` aside). `None` makes neither the class
/// nor a subclass get a layout.
pub(super) fn class_names(class: &StmtClassDef) -> Option<BTreeSet<String>> {
    if !class.keywords().is_empty() || !class.decorator_list.is_empty() {
        return None;
    }
    let mut names: BTreeSet<String> = BTreeSet::new();
    for stmt in &class.body {
        match stmt {
            Stmt::FunctionDef(def) => {
                let name = def.name.as_str();
                let dunder = name.starts_with("__") && name.ends_with("__");
                if dunder && !ALLOWED_DUNDERS.contains(&name) {
                    return None;
                }
                let plain = def.decorator_list.iter().all(|decorator| {
                    matches!(&decorator.expression, Expr::Name(n)
                        if matches!(n.id.as_str(), "staticmethod" | "classmethod"))
                });
                if !plain {
                    return None;
                }
                names.insert(name.to_string());
            }
            Stmt::Pass(_) => {}
            Stmt::Expr(expr) if matches!(&*expr.value, Expr::StringLiteral(_)) => {}
            Stmt::Assign(assign) if members::is_literal(&assign.value) => {
                for target in &assign.targets {
                    names.insert(plain_name(target)?);
                }
            }
            Stmt::AnnAssign(assign)
                if assign.value.as_deref().is_none_or(members::is_literal) =>
            {
                names.insert(plain_name(&assign.target)?);
            }
            _ => return None,
        }
    }
    Some(names)
}

/// The dunder methods a class with a layout may define: none of them
/// reaches an attribute, the object's state or its making.
const ALLOWED_DUNDERS: [&str; 3] = ["__init__", "__str__", "__repr__"];

/// `name` when `target` is a plain name that is no dunder.
fn plain_name(target: &Expr) -> Option<String> {
    match target {
        Expr::Name(name) if !(name.id.starts_with("__") && name.id.ends_with("__")) => {
            Some(name.id.to_string())
        }
        _ => None,
    }
}

/// The attributes of exception class `class` that go in its layout, with
/// their types, in first-store order (the module doc's rule). `inherited`
/// holds every name its bases bind at class level along its MRO (a
/// builtin's `errno`, a base's property): a store of one goes through
/// CPython's binding, so it stays on the CPython object.
pub(super) fn fields_of(
    class: &StmtClassDef,
    inherited: &BTreeSet<String>,
) -> Vec<(String, String)> {
    let Some(excluded) = class_names(class) else {
        return Vec::new();
    };
    let mut found: Vec<(String, String)> = Vec::new();
    let mut poisoned: BTreeSet<String> = BTreeSet::new();
    for stmt in &class.body {
        let Stmt::FunctionDef(def) = stmt else {
            continue;
        };
        let receiverless = matches!(
            def.name.as_str(),
            "__new__" | "__init_subclass__" | "__class_getitem__"
        ) || def.decorator_list.iter().any(|decorator| {
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
            || inherited.contains(&field)
            // A private name (CPython mangles it) or a dunder one (the
            // object's own machinery: `__notes__`, `__cause__`, ...).
            || field.starts_with("__")
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
