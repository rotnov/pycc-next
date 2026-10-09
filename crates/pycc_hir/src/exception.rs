//! HIR exception-class metadata and handler shape (PEP 3110, #382).

use super::{HirClassDef, HirItem, HirStmt, Ty};
use crate::ImportBinding;
use pycc_ast::visitor::{self, Visitor};
use pycc_ast::{Expr, ExprStringLiteral, ModModule, Stmt};

pub const BUILTIN_EXCEPTION_CLASSES: [&str; 30] = [
    "Exception",
    "ValueError",
    "TypeError",
    "KeyError",
    "IndexError",
    "ZeroDivisionError",
    "RuntimeError",
    // Part 2 of #543 (#739): PEP 3151 `OSError` hierarchy, tags 7..=22.
    "OSError",
    "BlockingIOError",
    "ChildProcessError",
    "ConnectionError",
    "FileExistsError",
    "FileNotFoundError",
    "InterruptedError",
    "IsADirectoryError",
    "NotADirectoryError",
    "PermissionError",
    "ProcessLookupError",
    "TimeoutError",
    "BrokenPipeError",
    "ConnectionAbortedError",
    "ConnectionRefusedError",
    "ConnectionResetError",
    // Part 3 of #382 (#542): PEP 654 exception groups, tags 23..=24. Both
    // carry a fixed tag exactly like the `OSError` family above -- they land
    // past `FIRST_OSERROR_FAMILY_TAG` by construction, so
    // `builtin_exception_class_defs`'s `(index >= FIRST_OSERROR_FAMILY_TAG)`
    // test already covers them with no change to that function. `D-202`
    // records the deliberate simplification of parenting `BaseExceptionGroup`
    // under `Exception` rather than modeling a second, `BaseException`-only
    // hierarchy root.
    "BaseExceptionGroup",
    "ExceptionGroup",
    // Part A of #1038 (#1063): the `OverflowError` tag, appended so every
    // existing tag keeps its value. Parented under `Exception` rather than
    // CPython's own `ArithmeticError`, which pycc does not model -- the same
    // deliberate hierarchy simplification `D-202` records for
    // `BaseExceptionGroup`. Raised by `pycc_rt`'s `float_pow` when a finite
    // base and exponent produce a non-finite result.
    "OverflowError",
    // #1292 (Part 2 of #1282): `ImportError` and `ModuleNotFoundError`,
    // tags 26..=27, appended so every existing tag keeps its value. Unlike
    // `OverflowError` and `BaseExceptionGroup` these carry CPython's real
    // parentage (`ModuleNotFoundError` -> `ImportError` -> `Exception`), so
    // this is not a `D-202`-style hierarchy simplification. The base goes
    // before its subclass, as `ConnectionError` does before its children.
    "ImportError",
    "ModuleNotFoundError",
    // #1369: `AssertionError`, tag 28, appended so every existing tag keeps
    // its value -- the class a failing `assert` statement raises. Parented
    // under `Exception`, CPython's real parentage, so this is not a
    // `D-202`-style hierarchy simplification.
    "AssertionError",
    // #1388: `AttributeError`, tag 29, appended so every existing tag keeps
    // its value -- the class a read of a not-yet-assigned instance slot
    // raises. Parented under `Exception`, CPython's real parentage.
    "AttributeError",
];

/// Part 2 of #541 (D-189): the number of names in [`BUILTIN_EXCEPTION_CLASSES`]
/// that predate Part 2 of #543 (#739) and are resolved by name, independent of
/// the class table, through `pycc_mir::exception::resolve_exception_tag`'s
/// hardcoded `match`. Everything from this index onward carries a fixed
/// `Some(index as u8)` tag on its
/// [`HirClassDef`] instead (see [`builtin_exception_class_defs`]) and has no
/// entry in that `match`.
///
/// This is deliberately a separate, smaller constant from
/// [`FIRST_USER_EXCEPTION_TYPE_TAG`] (7 vs. the array length) and the two must never be
/// conflated: this one names "the first index in `BUILTIN_EXCEPTION_CLASSES`
/// that is *not* one of the original flat seven", while
/// `FIRST_USER_EXCEPTION_TYPE_TAG` names "the first tag available to a user's
/// own classes" (one past the *whole* builtin array, including the `OSError`
/// family).
const FIRST_OSERROR_FAMILY_TAG: usize = 7;

/// Part 2 of #543 (#739): whether `name` is one of the original seven
/// builtin exception classes -- the ones resolved by name through
/// `pycc_mir::exception::resolve_exception_tag`'s hardcoded `match`,
/// independent of whether a `HirClassDef` for it is present in the class
/// table. Used by `pycc_types::exception::is_unshadowed_builtin_exception` to
/// decide which builtin names may be treated as "unshadowed" even when the
/// class table withheld seeding (see `shadowed_builtin_exception_name`)
/// -- the flat seven may be, by the same table-independent-resolution
/// property that keeps `resolve_exception_tag` correct for them; every
/// builtin past the flat seven (array index `>= FIRST_OSERROR_FAMILY_TAG`)
/// carries no such fallback and may not.
pub fn is_flat_builtin_exception_class(name: &str) -> bool {
    BUILTIN_EXCEPTION_CLASSES[..FIRST_OSERROR_FAMILY_TAG].contains(&name)
}

/// Part 2 of #541 (D-189): the first runtime exception type tag available to
/// a user-defined exception class. Tags `0..FIRST_USER_EXCEPTION_TYPE_TAG` are
/// permanently reserved for
/// [`BUILTIN_EXCEPTION_CLASSES`], in that array's order: `0..=6` for the
/// original flat seven, resolved by name; `7..=22` for the PEP 3151
/// `OSError` family added by Part 2 of #543 (#739); `23..=24` for
/// `BaseExceptionGroup`/`ExceptionGroup` added by Part 3 of #382 (#542); `25`
/// for `OverflowError` added by Part A of #1038 (#1063); `26`/`27` for
/// `ImportError`/`ModuleNotFoundError` added by #1292; `28` for
/// `AssertionError` added by #1369; `29` for `AttributeError` added by
/// #1388 -- everything past the
/// flat seven resolved by a fixed tag stored on each class's own
/// [`HirClassDef`].
pub const FIRST_USER_EXCEPTION_TYPE_TAG: u8 = BUILTIN_EXCEPTION_CLASSES.len() as u8;

/// #1316: the runtime tag reserved for a CPython exception the `ext` shim's
/// foreign-operation bridge translates when it is not an `Exception` at all
/// (`SystemExit`, `KeyboardInterrupt`, a non-`Exception`
/// `BaseExceptionGroup`). It is `pycc_rt`'s `EXCEPTION_TYPE_FOREIGN_BASE`,
/// the one tag `except Exception` does not match. It names no class a
/// program can spell, so it sits at the top of the `u8` range, past every
/// user class's tag, and [`MAX_USER_EXCEPTION_CLASSES`] leaves it out.
pub const FOREIGN_BASE_EXCEPTION_TYPE_TAG: u8 = u8::MAX;

/// Part 2 of #541 (D-189): how many user-defined exception classes one module
/// may declare. The runtime carries the type tag as a `u8`, so the whole
/// hierarchy is capped at 256 types; the builtins take the low
/// [`FIRST_USER_EXCEPTION_TYPE_TAG`] tags,
/// [`FOREIGN_BASE_EXCEPTION_TYPE_TAG`] takes the top one (#1316), and the
/// rest, up to 254, are available to the module's own classes.
/// Exceeding this is rejected with `C0001` during HIR lowering.
pub const MAX_USER_EXCEPTION_CLASSES: usize =
    FOREIGN_BASE_EXCEPTION_TYPE_TAG as usize - BUILTIN_EXCEPTION_CLASSES.len();

const fn const_str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Part 3 of #382 (#542, PEP 654, D-202): the fixed builtin runtime type tag
/// for the `ExceptionGroup` class, *derived* from its position in
/// [`BUILTIN_EXCEPTION_CLASSES`] rather than hand-maintained -- the same
/// derivation discipline D-194 established for [`FIRST_USER_EXCEPTION_TYPE_TAG`]
/// and [`MAX_USER_EXCEPTION_CLASSES`] itself, so inserting a new builtin ahead
/// of `ExceptionGroup` in the array shifts this constant automatically instead
/// of silently desynchronizing a hand-copied literal. `pycc_codegen::exception::
/// emit_try_star` uses this tag (and the class name) whenever it needs to
/// build a fresh reconstructed subgroup, regardless of the original raised
/// object's own dynamic class (D-202).
pub const EXCEPTION_GROUP_TYPE_TAG: u8 = {
    let mut i = 0;
    let mut tag: Option<u8> = None;
    while i < BUILTIN_EXCEPTION_CLASSES.len() {
        if const_str_eq(BUILTIN_EXCEPTION_CLASSES[i], "ExceptionGroup") {
            tag = Some(i as u8);
        }
        i += 1;
    }
    match tag {
        Some(t) => t,
        None => panic!("ExceptionGroup must be present in BUILTIN_EXCEPTION_CLASSES"),
    }
};

pub fn is_builtin_exception_class(name: &str) -> bool {
    BUILTIN_EXCEPTION_CLASSES.contains(&name)
}

/// Returns the builtin exception class's parent, or `None` for the root and
/// unknown names.
///
/// Part 1 of #541 supported only a flat, two-level hierarchy (every
/// non-`Exception` name's parent was `Exception`). Part 2 of #543 (#739)
/// added the real PEP 3151 `OSError` tree: `OSError` and the other six
/// original names are still direct children of `Exception`, as is Part A of
/// #1038 (#1063)'s `OverflowError`, #1369's `AssertionError`, #1388's
/// `AttributeError`, and #1292's
/// `ImportError`, whose own child
/// `ModuleNotFoundError` is one level deeper (CPython's real parentage); ten
/// more names
/// are direct children of `OSError`; and four more (`BrokenPipeError`,
/// `ConnectionAbortedError`, `ConnectionRefusedError`, `ConnectionResetError`)
/// are children of `ConnectionError`, itself a direct child of `OSError`.
pub fn builtin_exception_parent(name: &str) -> Option<&'static str> {
    match name {
        "Exception" => None,
        "BrokenPipeError"
        | "ConnectionAbortedError"
        | "ConnectionRefusedError"
        | "ConnectionResetError" => Some("ConnectionError"),
        // Part 3 of #382 (#542, D-202): `BaseExceptionGroup` parents to
        // `Exception` rather than a separate `BaseException`-only root, and
        // `ExceptionGroup` parents to `BaseExceptionGroup` -- a deliberate
        // simplification, not a CPython-accurate hierarchy. See D-202.
        // Part A of #1038 (#1063): `OverflowError` parents to `Exception`
        // rather than CPython's own `ArithmeticError`, which pycc does not
        // model -- the same deliberate simplification as `BaseExceptionGroup`
        // above. Omitting it here would make the new class a second hierarchy
        // root that `except Exception:` silently stops catching.
        // #1292: `ImportError` parents to `Exception` and `ModuleNotFoundError`
        // to `ImportError` -- CPython's real hierarchy, not a simplification.
        // #1369: `AssertionError` parents to `Exception`, as in CPython, and
        // so does #1388's `AttributeError`.
        "OSError" | "ValueError" | "TypeError" | "KeyError" | "IndexError"
        | "ZeroDivisionError" | "RuntimeError" | "BaseExceptionGroup" | "OverflowError"
        | "ImportError" | "AssertionError" | "AttributeError" => Some("Exception"),
        "ExceptionGroup" => Some("BaseExceptionGroup"),
        "ModuleNotFoundError" => Some("ImportError"),
        "BlockingIOError" | "ChildProcessError" | "ConnectionError" | "FileExistsError"
        | "FileNotFoundError" | "InterruptedError" | "IsADirectoryError" | "NotADirectoryError"
        | "PermissionError" | "ProcessLookupError" | "TimeoutError" => Some("OSError"),
        _ => None,
    }
}

/// The mangled `HirItem::Function` name of the synthetic `Exception`
/// constructor (Part 1 of #541). Uses the same `<ClassName>.<method>`
/// mangling every user-defined method already uses, so no downstream
/// consumer needs to special-case it.
pub const EXCEPTION_INIT_MANGLED_NAME: &str = "Exception.__init__";

/// Builds one synthetic [`HirClassDef`] per [`BUILTIN_EXCEPTION_CLASSES`]
/// name, giving the builtin
/// exception hierarchy a first-class presence in the class table
/// (Part 1 of #541, extending D-173; widened from 7 to 23 by Part 2 of
/// #543/#739's PEP 3151 `OSError` family, then to 25 by Part 3 of #382/#542's
/// `BaseExceptionGroup`/`ExceptionGroup`, then to 26 by Part A of
/// #1038/#1063's `OverflowError`, then to 28 by #1292's
/// `ImportError`/`ModuleNotFoundError`, then to 29 by #1369's
/// `AssertionError`, then to 30 by #1388's `AttributeError`).
///
/// Before this existed, `Exception`/`ValueError`/... were recognized only
/// by name, through [`is_builtin_exception_class`], with no `HirClassDef`
/// behind them. That left `class MyError(ValueError):` resolving a base
/// that the class table did not contain, and left every consumer of
/// `Environment::classes` unable to tell "not a class" from "a class the
/// frontend never materialized".
///
/// Lowering seeds them only into a module that actually references one of
/// the builtin exception names and shadows none of them -- see this module's
/// (crate-private) `module_references_builtin_exception_name` and
/// `shadowed_builtin_exception_name`.
///
/// The definitions are derived from [`BUILTIN_EXCEPTION_CLASSES`] and
/// [`builtin_exception_parent`], so the hierarchy has exactly one source of
/// truth. `Exception` is the root (no bases); every other builtin whose real
/// parent is `Exception` in [`builtin_exception_parent`] derives from it
/// directly; the `OSError` family
/// (Part 2 of #543/#739) reaches up to three levels deep (e.g.
/// `BrokenPipeError` -> `ConnectionError` -> `OSError` -> `Exception`), so
/// each entry's MRO is built by walking [`builtin_exception_parent`] up to
/// the root rather than a fixed-length push -- the same linearization
/// `compute_c3_mro` would produce, written directly here because the shape
/// is fixed and single-inheritance.
///
/// Only `Exception` carries a method: `__init__(self, message: str)`. Every
/// other class inherits it through its MRO, exactly as a user subclass
/// would. The synthetic classes deliberately carry no attribute slots
/// (`attrs` is empty): D-173 propagates a raised exception through global
/// runtime state rather than through an allocated instance with fields, so
/// there is no storage for a `message` slot to name. See
/// `docs/RUNTIME.md`.
pub fn builtin_exception_class_defs() -> Vec<(String, HirClassDef)> {
    BUILTIN_EXCEPTION_CLASSES
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let parent = builtin_exception_parent(name);
            let bases: Vec<String> = parent.map(|p| vec![p.to_string()]).unwrap_or_default();
            // Part 2 of #543 (#739): the `OSError` family is up to three
            // levels deep (e.g. `BrokenPipeError` -> `ConnectionError` ->
            // `OSError` -> `Exception`), so the MRO is a walk up the parent
            // chain rather than the single push the flat, two-level
            // hierarchy needed.
            let mut mro = vec![(*name).to_string()];
            let mut walk = parent;
            while let Some(p) = walk {
                mro.push(p.to_string());
                walk = builtin_exception_parent(p);
            }
            let methods = if parent.is_none() {
                vec![(
                    "__init__".to_string(),
                    EXCEPTION_INIT_MANGLED_NAME.to_string(),
                )]
            } else {
                Vec::new()
            };
            let def = HirClassDef {
                class_attrs: Vec::new(),
                // Part 2 of #543 (#739): the original flat seven keep `None`
                // (resolved by name, table-independent, through
                // `resolve_exception_tag`); every builtin past the flat seven
                // (array index `>= FIRST_OSERROR_FAMILY_TAG`) gets a fixed
                // `Some(index)` tag stored directly on the `HirClassDef`,
                // since it has no such name-based fallback.
                exception_type_tag: (index >= FIRST_OSERROR_FAMILY_TAG).then_some(index as u8),
                name: (*name).to_string(),
                bases,
                mro,
                attrs: Vec::new(),
                methods,
                properties: Vec::new(),
                static_methods: Vec::new(),
                class_methods: Vec::new(),
                type_param: None,
                is_enum: false,
                implicit_object_init: false,
                method_defaults: Vec::new(),
                enum_members: Vec::new(),
                is_dataclass: false,
                dataclass_fields: Vec::new(),
                is_protocol: false,
                runtime_checkable: false,
                protocol_members: Vec::new(),
                abstract_methods: Vec::new(),
                is_abstract: false,
            };
            ((*name).to_string(), def)
        })
        .collect()
}

/// Builds the synthetic `Exception.__init__(self, message: str)` item that
/// [`builtin_exception_class_defs`]'s `Exception` entry names in its method
/// table (Part 1 of #541).
///
/// It is emitted as an ordinary mangled `HirItem::Function`, so the type
/// checker registers its signature through the same per-item signature pass
/// every user method already goes through, and instantiation of a user
/// subclass resolves it through the MRO with no new lookup path.
///
/// Its body is a bare `return`: D-173 stores the raised exception in global
/// runtime state at the `raise` site, so the constructor has no field to
/// initialize. The signature is deliberately narrower than CPython's
/// `Exception(*args)` -- this compiler has no variadic-argument support,
/// and the existing `raise ValueError("...")` surface already accepts
/// exactly one `str` message. `docs/RUNTIME.md` records that divergence.
pub fn builtin_exception_init_item() -> HirItem {
    HirItem::Function {
        name: EXCEPTION_INIT_MANGLED_NAME.to_string(),
        params: vec![
            (
                "self".to_string(),
                Ty::Instance(Box::new("Exception".to_string())),
            ),
            ("message".to_string(), Ty::Str),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    }
}

/// Whether `module` spells any of the [`BUILTIN_EXCEPTION_CLASSES`] names
/// anywhere at all (Part 1 of #541; the array grew from 7 to 23 entries in
/// Part 2 of #543/#739, this gate's own logic is name-count-agnostic and
/// unchanged).
///
/// HIR lowering seeds the synthetic class definitions only into a module
/// that can actually observe them. Seeding unconditionally put the whole
/// `HirClassDef` array into *every* module's class table, and the frontend's
/// per-item work is proportional to the size of that table: the original seven
/// entries cost `benches/check_bench.rs`'s class-free fixture roughly 3x its
/// whole `parse` + `lower_checked` + `check` time, which the
/// `frontend-perf-gate` rejected. A module that never names a builtin
/// exception cannot tell a seeded class table from an unseeded one, so it
/// pays nothing.
///
/// Absence must never be read as "the user shadowed this name". It is not:
/// `pycc_types::exception::is_user_defined_class` is
/// `classes.contains_key(name) && !is_synthetic_class(name)`, so an absent
/// name is *not* user-defined, which is exactly the pre-#541 reading.
///
/// Four consumers instead need a seeded class to be *present*: base
/// resolution (`class MyError(ValueError):`), `class::expect_class` behind a
/// `Ty::Instance` (`except ValueError as e: e.args`), `annotation_to_ty`
/// projection (`e: ValueError`), and `isinstance`/`issubclass`. Each of them
/// requires a builtin exception name to be spelled in the module, so *this* gate
/// never withholds a definition from a module that could have reached one:
/// a module it refuses to seed cannot name a builtin exception at all.
///
/// That is a property of this gate alone, not of the pair. The
/// all-or-nothing shadow gate below can still withhold every builtin from a
/// module that spells one of them, whenever the module's top level binds a
/// *different* one -- and `class Exception: ...` plus
/// `except ValueError as e: print(e.args)` still reaches
/// `class::expect_class`'s internal-error panic for exactly that reason.
/// That gap predates this gate (it reproduces identically at Part 1 of
/// #541's own commit, before the reference gate existed) and is unchanged
/// by it; closing it belongs to Part 2, which has to decide what a partially
/// shadowed builtin hierarchy means.
///
/// The walk is [`pycc_ast::visitor::Visitor`], ruff's own generic AST
/// traversal, rather than a hand-rolled `Stmt`/`Expr` match. That is
/// deliberate: a hand-rolled match needs a `_ =>` arm or exhaustive
/// enumeration of every upstream node, and a spelling missed there fails
/// *silently* -- as a spurious `C0001` or an internal-compiler-error abort,
/// not a diagnostic. Overriding only `visit_expr` on the generic walker
/// makes every name-bearing position -- class bases, decorators, `raise`
/// operands, `except` types, annotations, call arguments, attribute values,
/// comprehensions, `match` patterns, f-string interpolations -- reachable by
/// construction, and keeps new upstream AST nodes covered automatically.
/// A string annotation counts wherever annotation lowering may resolve it
/// (Part 1 of #889): a top-level one (`x: "ValueError"`) arrives already
/// unquoted from `pycc_parser::parse_all`, and a string nested in an
/// annotation or a type alias value (`list["ValueError"]`) is parsed here.
/// That over-approximates harmlessly -- a string lowering discards, such as
/// `Literal["ValueError"]`, also counts, which only seeds the classes. A
/// string anywhere else (`x = "ValueError"`) is only a string.
///
/// The one non-name reference is an `assert` statement (#1369), which
/// raises `AssertionError` without spelling it; a `visit_stmt` override
/// counts it and otherwise leaves the generic walk untouched.
pub(crate) fn module_references_builtin_exception_name(module: &ModModule) -> bool {
    let mut scan = ReferenceScan { found: false };
    scan.visit_body(&module.body);
    scan.found
}

/// The walk behind [`module_references_builtin_exception_name`].
struct ReferenceScan {
    found: bool,
}

/// Whether a string nested in `annotation` (`list["ValueError"]`), once
/// parsed the way `func::annotation_to_ty` parses it, spells a builtin
/// exception class. A string that does not parse spells nothing.
fn nested_string_annotation_references(annotation: &Expr) -> bool {
    struct Strings<'s>(Vec<&'s ExprStringLiteral>);
    impl<'s> Visitor<'s> for Strings<'s> {
        fn visit_expr(&mut self, expr: &'s Expr) {
            if let Expr::StringLiteral(literal) = expr {
                self.0.push(literal);
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut strings = Strings(Vec::new());
    strings.visit_expr(annotation);
    strings.0.into_iter().any(|literal| {
        pycc_ast::parse_string_annotation(literal).is_ok_and(|parsed| {
            let mut scan = ReferenceScan { found: false };
            scan.visit_annotation(&parsed);
            scan.found
        })
    })
}

/// The value of a type alias statement, in either spelling: `type X =
/// <value>` or the legacy `X: TypeAlias = <value>`, the two shapes
/// `import::type_alias` resolves as a type.
fn type_alias_value(stmt: &Stmt) -> Option<&Expr> {
    match stmt {
        Stmt::TypeAlias(alias) => Some(&alias.value),
        Stmt::AnnAssign(assign)
            if assign
                .annotation
                .as_name_expr()
                .is_some_and(|name| name.id.as_str() == "TypeAlias") =>
        {
            assign.value.as_deref()
        }
        _ => None,
    }
}

impl<'a> Visitor<'a> for ReferenceScan {
    fn visit_expr(&mut self, expr: &'a Expr) {
        // Once one spelling is seen the answer cannot change, so stop
        // descending rather than walking the rest of the module.
        if self.found {
            return;
        }
        if let Expr::Name(name) = expr
            && is_builtin_exception_class(name.id.as_str())
        {
            self.found = true;
            return;
        }
        visitor::walk_expr(self, expr);
    }
    // #1369: an `assert` statement raises `AssertionError` without
    // spelling it, and its lowering names that class by spelling, so it
    // needs the seeded definitions exactly as `raise AssertionError(..)`
    // would.
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        if self.found {
            return;
        }
        if matches!(stmt, Stmt::Assert(_)) {
            self.found = true;
            return;
        }
        // Part 1 of #889: a type alias's value is resolved as a type, so a
        // string nested in it (`type E = list["ValueError"]`) is resolved
        // too, although the generic walk visits the value as an expression.
        if type_alias_value(stmt).is_some_and(nested_string_annotation_references) {
            self.found = true;
            return;
        }
        visitor::walk_stmt(self, stmt);
    }
    // Part 1 of #889: a string nested in an annotation is resolved by
    // annotation lowering, so a builtin exception name inside it is a
    // reference too.
    fn visit_annotation(&mut self, annotation: &'a Expr) {
        if self.found {
            return;
        }
        visitor::walk_annotation(self, annotation);
        if !self.found && nested_string_annotation_references(annotation) {
            self.found = true;
        }
    }
}

/// The range of the first live `assert` statement anywhere in `body`,
/// nested bodies included, or `None` when it has none (#1369).
///
/// `module::lower_module` asks this, one top-level item at a time, only when
/// [`shadowed_builtin_exception_name`] withheld the builtin classes: an
/// `assert` needs the builtin `AssertionError`, which such a module does not
/// have, so the first top-level item containing one is refused with one
/// `C0001` naming the binding responsible rather than at each `assert` with a
/// message about the call it was rewritten into.
///
/// "Live" means lowering keeps it: every `if`/`elif` chain is walked through
/// `dunder_name::walk_live_if`, so an `assert` in a body the #790
/// `TYPE_CHECKING` fold discards is never lowered and never counted, while
/// the `else` (and any other arm) of such a guard is walked in full. A
/// class body is walked only through its methods' bodies, the only part of
/// it `class::body` hands to `lower_stmt`: a statement directly in a class
/// body keeps the class lowering's own refusal. (An `Enum` body accepts no
/// method at all, and a `Protocol` method accepts only a stub body, so an
/// `assert` in either is refused here rather than by that class's own
/// lowering -- a refusal either way, never an admission.)
/// `imports` must be the whole-module slice `dunder_name::scan_imports`
/// builds, so an aliased `t.TYPE_CHECKING` guard folds here exactly as it
/// folds in `lower_stmt`; the one mismatch that slice admits (a guard above
/// its own `import typing as t`) is a read of an unbound name that is
/// refused anyway, and an `assert` skipped there would still only reach an
/// unresolved `AssertionError`, never another class.
pub(crate) fn first_assert_statement_range(
    body: &[Stmt],
    imports: &[ImportBinding],
) -> Option<std::ops::Range<u32>> {
    struct AssertScan<'i> {
        found: Option<std::ops::Range<u32>>,
        imports: &'i [ImportBinding],
    }
    impl<'a> Visitor<'a> for AssertScan<'_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if self.found.is_some() {
                return;
            }
            match stmt {
                Stmt::Assert(_) => self.found = Some(pycc_ast::stmt_range(stmt)),
                Stmt::If(if_stmt) => {
                    let imports = self.imports;
                    crate::dunder_name::walk_live_if(self, if_stmt, imports);
                }
                // A class body reaches `lower_stmt` only through its method
                // bodies: `class::body` refuses every other class-body
                // statement kind with its own diagnostic, a bare `assert`
                // and a nested `class` included. Only a method's own
                // statements are scanned, so that refusal is never masked.
                Stmt::ClassDef(class_def) => {
                    for method in class_def.body.iter().filter_map(Stmt::as_function_def_stmt) {
                        self.visit_body(&method.body);
                    }
                }
                _ => visitor::walk_stmt(self, stmt),
            }
        }
    }
    let mut scan = AssertScan {
        found: None,
        imports,
    };
    scan.visit_body(body);
    scan.found
}

/// Whether `module`'s own top level binds any
/// [`BUILTIN_EXCEPTION_CLASSES`] name (Part 1 of #541).
///
/// This is the second of lowering's two seeding gates: a module is seeded
/// only when [`module_references_builtin_exception_name`] holds *and* this
/// does not. The two ask deliberately different questions and are kept
/// separate rather than fused into one pass -- shadowing is a property of a
/// module's *top level* only (a `class ValueError:` nested inside a function
/// shadows nothing at module scope), while a reference counts at any depth.
///
/// The seeding is all-or-nothing: when this returns `true`, *no* synthetic
/// class is seeded, and the builtin exception names keep exactly the pre-#541 behavior
/// of being recognized by name alone. Two reasons for all-or-nothing rather
/// than per-name:
///
/// * Seeding a subset would make the hierarchy incoherent -- a synthetic
///   `ValueError` whose `bases`/`mro` name an `Exception` that a user's own
///   `class Exception:` has replaced resolves its inherited `__init__`
///   against the wrong class.
/// * It makes `HirModule::seeded_builtin_exception_classes` -- the single
///   flag `module::lower_all` records when it seeds -- an exact provenance
///   record. Because a module containing a user binding of any builtin
///   exception name carries no synthetic definitions at all, "the flag is set
///   and the name is a builtin exception name" identifies precisely the compiler-produced
///   entries, with no user class among them. Provenance is never re-derived
///   from a definition's shape: a user can author a class structurally
///   identical to a synthetic one (see D-188).
///
/// The scan is deliberately conservative: it reports `true` for any
/// top-level `class`/`def`/`type`-alias/annotated-assignment/assignment/
/// augmented-assignment target spelling a builtin exception name (`ValueError += 1`
/// binds the name as surely as `ValueError = ...` does, #1209), whether or not that particular
/// spelling would go on to collide with a seeded definition. Over-reporting
/// only costs the module its synthetic classes; under-reporting would let a
/// user definition collide with a compiler-synthesized one and surface as a
/// spurious `C0001`.
///
/// `import`/`from ... import ...` are not scanned: D-136/D-137 resolve every
/// import against `pycc_std`'s registry, which contains none of the builtin
/// exception names, so an `import ValueError` is rejected as an unresolvable module
/// before any class-table collision could be reached.
/// The first builtin exception name the module's top level
/// binds, in source order, or `None` when it binds none -- the same scan
/// that used to answer only "does the module shadow one", but naming it so
/// `program::link` can report which name a module defines when it cannot
/// be linked with a module that was seeded (#898).
pub(crate) fn shadowed_builtin_exception_name(module: &ModModule) -> Option<String> {
    module.body.iter().find_map(|stmt| match stmt {
        Stmt::ClassDef(class_def) => builtin_exception_name(class_def.name.as_str()),
        Stmt::FunctionDef(function_def) => builtin_exception_name(function_def.name.as_str()),
        Stmt::TypeAlias(type_alias) => expr_bound_builtin_exception_name(&type_alias.name),
        Stmt::AnnAssign(ann_assign) => expr_bound_builtin_exception_name(&ann_assign.target),
        Stmt::Assign(assign) => assign
            .targets
            .iter()
            .find_map(expr_bound_builtin_exception_name),
        Stmt::AugAssign(aug_assign) => expr_bound_builtin_exception_name(&aug_assign.target),
        _ => None,
    })
}

fn builtin_exception_name(name: &str) -> Option<String> {
    is_builtin_exception_class(name).then(|| name.to_string())
}

/// The first builtin exception name `expr`, used as an
/// assignment target, binds. Recurses through the unpacking-target shapes
/// (`a, b = ...`, `[a, b] = ...`, `*rest`) so a name buried in one is still
/// seen. Any other target shape (an attribute, a subscript) rebinds
/// something other than a bare module-level name, so it cannot shadow a
/// builtin exception name.
fn expr_bound_builtin_exception_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Name(name) => builtin_exception_name(name.id.as_str()),
        Expr::Tuple(tuple) => tuple
            .elts
            .iter()
            .find_map(expr_bound_builtin_exception_name),
        Expr::List(list) => list.elts.iter().find_map(expr_bound_builtin_exception_name),
        Expr::Starred(starred) => expr_bound_builtin_exception_name(&starred.value),
        _ => None,
    }
}

/// A single `except` handler. `exc_type` is `None` for bare `except:`.
/// `Some(names)` names one or more exception types (PEP 758 `except A, B:`
/// and `except (A, B):` both lower to the same shape, `names` always in
/// source order); `names` is never empty -- an empty list (`except ():`) is
/// rejected with `C0001` at the one site that constructs a fresh handler
/// from source (`lower_except_handler`), so every downstream consumer may
/// assume non-emptiness without re-checking. HIR-to-HIR rewrite passes
/// (e.g. `pycc_types::unroll_enum_loops_in_stmts`) only clone this field
/// through unchanged and introduce no second construction site; a future
/// pass that builds a `HirExceptHandler` with new logic rather than a pure
/// clone would need to uphold the same non-empty invariant itself.
/// `name` is the optional `as` binding.
#[derive(Debug, Clone, PartialEq)]
pub struct HirExceptHandler {
    pub exc_type: Option<Vec<String>>,
    pub name: Option<String>,
    pub body: Vec<HirStmt>,
}

/// The representative exception type name for a single- or multi-type
/// `except` handler's `as` binding, generic-call rewriting, and constraint
/// collection. All four binding sites (MIR lowering, type checking,
/// monomorphization, constraint collection) must agree on the same name,
/// so they all call this one helper rather than inlining a name choice
/// independently.
///
/// A single-name handler (`names.len() == 1`, including `except (A,):` and
/// bare-comma `except A,:`) binds to that exact name, matching `except A:`.
///
/// A genuinely multi-type handler (`names.len() > 1`) binds to
/// `"Exception"`, the universal builtin root, rather than to the
/// first-listed name. An earlier version of this helper always picked
/// `names[0]` (see the D-195 decision entry's now-superseded rationale);
/// that choice was unsound for `isinstance()`, not just imprecise.
/// `isinstance()` in this compiler folds entirely at compile time from the
/// object's *static* `Ty::Instance` type's MRO
/// (`pycc_mir::class::lower_isinstance`,
/// `pycc_hir::typecheck::eval_isinstance_single`) -- there is no runtime
/// type check. Binding `except (ValueError, TypeError) as e:` to
/// `"ValueError"` made `isinstance(e, ValueError)` fold to a constant
/// `True` even when the handler actually caught a `TypeError`: a false
/// *positive*, worse than the pre-existing, already-accepted imprecision
/// for single-type handlers (which can only produce false *negatives* on
/// subclass-narrowing queries, since a single declared type is always a
/// genuine ancestor of anything that handler can catch). Binding to
/// `"Exception"` instead means `"Exception"`'s own MRO contains no
/// descendant name, so `isinstance(e, AnySpecificType)` on a multi-type
/// handler's binding is never folded to a wrong `True`. Since #1337 such a
/// query (any target outside the static MRO) is no longer folded at all: it
/// reads the caught object's runtime type tag (`MirExpr::ExceptionTypeTest`),
/// so both the single- and the multi-type binding answer exactly as CPython
/// does, and the `"Exception"` binding costs nothing. `raise e` (bare-name re-raise) is unaffected by
/// this choice either way: it lowers to `MirExceptionValue::Existing`,
/// which preserves the actual runtime exception object and tag rather than
/// reconstructing from this static binding type
/// (`pycc_mir::exception::lower_exception_value`).
///
/// # Panics
/// Panics if `names` is empty. `HirExceptHandler::exc_type` is documented
/// as never containing an empty `Vec`, so callers passing that field's
/// contents never trigger this.
pub fn except_handler_binding_type_name(names: &[String]) -> String {
    match names {
        [] => panic!("except handler type list must not be empty"),
        [only] => only.clone(),
        _ => "Exception".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_exception_table_and_tree_shape_are_consistent() {
        // Every name in the table is recognized, and every name's declared
        // parent (if any) is itself in the table -- the tree has no dangling
        // edges. Part 2 of #543 (#739) replaced the flat, every-parent-is-
        // `Exception` shape with a real tree, so this no longer asserts a
        // single fixed parent for every non-root name; see the two
        // OSError-family tests below for the parts of the shape that matter.
        for name in BUILTIN_EXCEPTION_CLASSES {
            assert!(is_builtin_exception_class(name));
            if let Some(parent) = builtin_exception_parent(name) {
                assert!(
                    is_builtin_exception_class(parent),
                    "`{name}`'s parent `{parent}` must itself be in the table"
                );
            } else {
                assert_eq!(name, "Exception", "only `Exception` has no parent");
            }
        }
        assert!(!is_builtin_exception_class("NotAnException"));
        assert_eq!(builtin_exception_parent("NotAnException"), None);
    }

    #[test]
    fn const_str_eq_matches_equal_and_rejects_unequal_or_different_length_strs() {
        // `const_str_eq` is declared `const fn` so `EXCEPTION_GROUP_TYPE_TAG`'s
        // derivation can call it at compile time, but the derivation's own
        // `const` block is evaluated entirely during compilation and folded
        // to a literal -- `cargo-llvm-cov`'s runtime instrumentation cannot
        // observe that compile-time execution. Calling the same `const fn`
        // again here, from an ordinary (non-const) test body, exercises its
        // body at runtime so the success, mismatch, and differing-length
        // paths are all covered by the D-014 gate.
        assert!(const_str_eq("ExceptionGroup", "ExceptionGroup"));
        assert!(!const_str_eq("ExceptionGroup", "BaseExceptionGroup"));
        assert!(!const_str_eq("ExceptionGroup", "ValueError"));
        // Same length, differing content: exercises the per-character
        // mismatch branch inside the loop, distinct from the length-mismatch
        // early return above -- neither prior assertion has equal-length
        // operands, so that inner `if a[i] != b[i] { return false; }` branch
        // was never taken before this case.
        assert!(!const_str_eq("ExceptionGroup", "ExceptionGrouq"));
    }

    #[test]
    fn exception_group_type_tag_matches_its_position_in_the_builtin_table() {
        let position = BUILTIN_EXCEPTION_CLASSES
            .iter()
            .position(|&name| name == "ExceptionGroup")
            .expect("ExceptionGroup must be present in BUILTIN_EXCEPTION_CLASSES");
        assert_eq!(EXCEPTION_GROUP_TYPE_TAG, position as u8);
    }

    #[test]
    fn the_original_flat_seven_still_derive_directly_from_exception() {
        for name in [
            "ValueError",
            "TypeError",
            "KeyError",
            "IndexError",
            "ZeroDivisionError",
            "RuntimeError",
        ] {
            assert_eq!(builtin_exception_parent(name), Some("Exception"));
        }
    }

    #[test]
    fn the_oserror_family_has_the_real_pep_3151_tree_shape() {
        // `OSError` itself and its ten non-`ConnectionError` direct children
        // are direct children of `Exception`/`OSError` respectively;
        // `ConnectionError`'s own four children are one level deeper still.
        assert_eq!(builtin_exception_parent("OSError"), Some("Exception"));
        for name in [
            "BlockingIOError",
            "ChildProcessError",
            "ConnectionError",
            "FileExistsError",
            "FileNotFoundError",
            "InterruptedError",
            "IsADirectoryError",
            "NotADirectoryError",
            "PermissionError",
            "ProcessLookupError",
            "TimeoutError",
        ] {
            assert_eq!(builtin_exception_parent(name), Some("OSError"));
        }
        for name in [
            "BrokenPipeError",
            "ConnectionAbortedError",
            "ConnectionRefusedError",
            "ConnectionResetError",
        ] {
            assert_eq!(builtin_exception_parent(name), Some("ConnectionError"));
        }
    }

    /// Work item 4 of the Part 2 of #543 (#739) implementation plan: pins the
    /// exact tag-to-name mapping so a future edit cannot silently reorder
    /// [`BUILTIN_EXCEPTION_CLASSES`] and renumber every tag with no test
    /// catching it.
    #[test]
    fn builtin_exception_tags_are_pinned_to_their_array_index() {
        let defs = builtin_exception_class_defs();
        // `.expect` rather than `.unwrap_or_else(|| panic!(...))`: every name
        // this closure is called with is drawn from `BUILTIN_EXCEPTION_CLASSES`
        // itself, so `.find` always succeeds and a user-code panic closure
        // would be an uncovered region under the 100% coverage gate. `.expect`
        // keeps the same failure message without adding an unreachable-in-practice
        // branch to this file's own instrumented code.
        let tag_of = |name: &str| {
            defs.iter()
                .find(|(n, _)| n == name)
                .expect("name must be synthesized")
                .1
                .exception_type_tag
        };
        // The original flat seven carry no tag of their own -- they are
        // resolved by name, independent of the class table.
        for name in [
            "Exception",
            "ValueError",
            "TypeError",
            "KeyError",
            "IndexError",
            "ZeroDivisionError",
            "RuntimeError",
        ] {
            assert_eq!(tag_of(name), None, "`{name}` must carry no assigned tag");
        }
        // Every tag past the flat seven is pinned to the array order declared
        // in `BUILTIN_EXCEPTION_CLASSES`.
        for (name, tag) in [
            ("OSError", 7),
            ("BlockingIOError", 8),
            ("ChildProcessError", 9),
            ("ConnectionError", 10),
            ("FileExistsError", 11),
            ("FileNotFoundError", 12),
            ("InterruptedError", 13),
            ("IsADirectoryError", 14),
            ("NotADirectoryError", 15),
            ("PermissionError", 16),
            ("ProcessLookupError", 17),
            ("TimeoutError", 18),
            ("BrokenPipeError", 19),
            ("ConnectionAbortedError", 20),
            ("ConnectionRefusedError", 21),
            ("ConnectionResetError", 22),
            ("BaseExceptionGroup", 23),
            ("ExceptionGroup", 24),
            ("OverflowError", 25),
            ("ImportError", 26),
            ("ModuleNotFoundError", 27),
            ("AssertionError", 28),
            ("AttributeError", 29),
        ] {
            assert_eq!(tag_of(name), Some(tag), "`{name}` must carry tag {tag}");
        }
    }

    /// Part 3 of #382 (#542, D-202): pins the deliberate simplification that
    /// `BaseExceptionGroup` parents directly to `Exception` (not a separate
    /// `BaseException`-only root) and `ExceptionGroup` parents to
    /// `BaseExceptionGroup`.
    #[test]
    fn exception_group_hierarchy_parents_base_exception_group_to_exception() {
        assert_eq!(
            builtin_exception_parent("BaseExceptionGroup"),
            Some("Exception")
        );
        assert_eq!(
            builtin_exception_parent("ExceptionGroup"),
            Some("BaseExceptionGroup")
        );
        assert!(is_builtin_exception_class("BaseExceptionGroup"));
        assert!(is_builtin_exception_class("ExceptionGroup"));
        assert!(!is_flat_builtin_exception_class("BaseExceptionGroup"));
        assert!(!is_flat_builtin_exception_class("ExceptionGroup"));
    }

    #[test]
    fn is_flat_builtin_exception_class_recognizes_exactly_the_original_seven() {
        for name in [
            "Exception",
            "ValueError",
            "TypeError",
            "KeyError",
            "IndexError",
            "ZeroDivisionError",
            "RuntimeError",
        ] {
            assert!(
                is_flat_builtin_exception_class(name),
                "`{name}` must be flat"
            );
        }
        for name in [
            "OSError",
            "FileNotFoundError",
            "ConnectionResetError",
            "NotAnException",
        ] {
            assert!(
                !is_flat_builtin_exception_class(name),
                "`{name}` must not be flat"
            );
        }
    }

    #[test]
    fn except_handler_binding_type_name_uses_exception_for_multiple_names() {
        // A genuinely multi-type handler binds to the universal root, not
        // the first-listed name -- binding to `names[0]` made
        // `isinstance(e, names[0])` fold to a compile-time-constant `True`
        // even when the handler actually caught a different named type, a
        // false positive (chatgpt-codex-connector[bot] finding on #740).
        assert_eq!(
            except_handler_binding_type_name(&["ValueError".to_string(), "TypeError".to_string(),]),
            "Exception"
        );
        assert_eq!(
            except_handler_binding_type_name(&[
                "ValueError".to_string(),
                "TypeError".to_string(),
                "KeyError".to_string(),
            ]),
            "Exception"
        );
    }

    #[test]
    fn except_handler_binding_type_name_keeps_the_exact_name_for_a_single_type() {
        // A single-name handler -- including `except (A,):` and bare-comma
        // `except A,:`, both of which lower to a one-element `names` -- binds
        // to that exact name, matching plain `except A:` exactly.
        assert_eq!(
            except_handler_binding_type_name(&["ValueError".to_string()]),
            "ValueError"
        );
    }

    #[test]
    #[should_panic(expected = "except handler type list must not be empty")]
    fn except_handler_binding_type_name_panics_on_empty_slice() {
        // `HirExceptHandler::exc_type` is documented as never `Some(vec![])`
        // (enforced at `lower_except_handler`, the one production site), so
        // this panic is unreachable through normal lowering -- but the
        // invariant is pinned here, at the point it is actually consumed,
        // independent of whichever HIR lowering path currently happens to
        // prevent it from firing (deep-reviewer finding on #740).
        except_handler_binding_type_name(&[]);
    }

    /// #1209: a module-level augmented assignment binds its target name, so
    /// `ValueError += 1` shadows the builtin exactly as `ValueError = 1` does.
    #[test]
    fn a_module_level_augmented_assignment_shadows_a_builtin_exception() {
        let module = pycc_parser::parse("x = 1\nValueError += 1\n").expect("parses");
        assert_eq!(
            shadowed_builtin_exception_name(&module),
            Some("ValueError".to_string())
        );
        let module = pycc_parser::parse("x = 1\nx += 1\n").expect("parses");
        assert_eq!(shadowed_builtin_exception_name(&module), None);
    }
}

#[cfg(test)]
mod synthetic_class_tests;

#[cfg(test)]
mod tag_tests;
