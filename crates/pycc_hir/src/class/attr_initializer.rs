//! Class-attribute initializer shapes, extracted from [`super::attrs`] (D-185:
//! decompose the part a change touches).
//!
//! `super::attrs` owns the declaration-level checks (target shape, reserved
//! names, duplicates, annotation wrappers, collisions). This module owns the
//! right-hand side: inferring an un-annotated attribute's type from its
//! literal, extracting the compile-time constant, and the shared `C0001`s
//! both spellings report for a missing or unsupported initializer.
//!
//! #1345 (Part 1 of #1284) adds [`classify_non_literal`]: the one
//! non-literal shape admitted, `name = staticmethod(<foreign ref>)`, and a
//! precise `C0001` naming the tracking issue for every other shape.

use super::ClassAttrValue;
use super::foreign_static::{ForeignCallableRef, binds_name};
use crate::expr::receiver_dispatch::RECEIVER_DISPATCHED_NAMES;
use crate::hir_module::ForeignImportSite;
use crate::import::import_local_name;
use crate::{ImportBinding, Ty, unsupported};
use pycc_ast::{Expr, Number, Stmt, UnaryOp};
use pycc_diag::Diagnostic;

/// What the non-literal classifier needs to know about the scope a class
/// attribute is written in.
pub(super) struct InitializerScope<'a> {
    /// The module's import table as it stands at this class.
    pub(super) imports: &'a [ImportBinding],
    /// Whether the module body binds `staticmethod` anywhere (a `def`,
    /// assignment, or import), computed once by `module::lower_module`.
    pub(super) staticmethod_rebound: bool,
    /// The statements of this class body that precede the attribute.
    pub(super) earlier: &'a [Stmt],
}

/// Which spelling an attribute was written in: the annotated one
/// (`X: int = ...`, `X: Final = ...`) or the bare one (`X = ...`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Spelling {
    Annotated,
    Unannotated,
}

/// #1345: classifies a class-attribute initializer that is not a literal
/// [`infer_class_attr_ty`] recognizes.
///
/// The un-annotated `name = staticmethod(<ref>)` whose `<ref>` is a bare
/// name or dotted attribute chain rooted at an unconditional module-level
/// foreign import is admitted and returned. Every other shape is a `C0001`
/// that names what is unsupported and the issue tracking it; a shape this
/// classifier does not name keeps [`bad_class_attr_shape`]'s literal-only
/// wording. Both spellings go through here, so they report identically
/// except that the annotated spelling of an otherwise admitted shape is
/// refused with its own message.
pub(super) fn classify_non_literal(
    value: &Expr,
    attr_name: &str,
    spelling: Spelling,
    scope: &InitializerScope<'_>,
    range: std::ops::Range<u32>,
) -> Result<ForeignCallableRef, Diagnostic> {
    let refuse = |reason: String| {
        Err(unsupported(
            format!("class attribute `{attr_name}` {reason}"),
            range.clone(),
        ))
    };
    let Expr::Call(call) = value else {
        if let Some((root, path)) = reference_chain(value) {
            let hint = if foreign_item_import(scope.imports, &root) {
                format!(
                    "; to bind a CPython callable, write `{attr_name} = staticmethod({})`",
                    render(&root, &path)
                )
            } else {
                String::new()
            };
            return refuse(format!(
                "is initialized with a name or attribute reference, which is not supported yet \
                 (#1348) -- a class attribute must be a literal or \
                 `staticmethod(<foreign import>)`{hint}"
            ));
        }
        if matches!(
            value,
            Expr::List(_)
                | Expr::Tuple(_)
                | Expr::Set(_)
                | Expr::Dict(_)
                | Expr::ListComp(_)
                | Expr::SetComp(_)
                | Expr::DictComp(_)
                | Expr::Generator(_)
        ) {
            return refuse(
                "is initialized with a container, which is not supported yet (#1348) -- a class \
                 attribute must be a literal or `staticmethod(<foreign import>)`"
                    .to_string(),
            );
        }
        return Err(bad_class_attr_shape(attr_name, range));
    };
    let callee = match call.func.as_ref() {
        Expr::Name(name) => name.id.as_str(),
        _ => "",
    };
    if callee == "classmethod" {
        return refuse("uses `classmethod(...)`, which is not supported yet (#1347)".to_string());
    }
    if callee != "staticmethod" {
        return refuse(
            "is initialized with a call, which is not supported yet (#1348) -- a class attribute \
             must be a literal or `staticmethod(<foreign import>)`"
                .to_string(),
        );
    }
    if scope.staticmethod_rebound || binds_name(scope.earlier, scope.imports, "staticmethod") {
        return refuse(
            "calls `staticmethod`, which is rebound in this module or class body, so it is not \
             the builtin `staticmethod`"
                .to_string(),
        );
    }
    let [argument] = &*call.arguments.args else {
        return refuse(
            "uses `staticmethod(...)`, which takes exactly one positional argument".to_string(),
        );
    };
    if !call.arguments.keywords.is_empty() || matches!(argument, Expr::Starred(_)) {
        return refuse(
            "uses `staticmethod(...)`, which takes exactly one positional argument".to_string(),
        );
    }
    let Some((root, path)) = reference_chain(argument) else {
        return refuse(
            "uses `staticmethod(...)` whose argument is not a name or dotted attribute of a \
             foreign (CPython) import, which is not supported yet (#1348)"
                .to_string(),
        );
    };
    let written = render(&root, &path);
    if binds_name(scope.earlier, scope.imports, &root) {
        return refuse(format!(
            "uses `staticmethod({written})`, where `{root}` refers to this class's own earlier \
             binding, not a foreign import -- `staticmethod` of a class-body binding is not \
             supported yet (#1347)"
        ));
    }
    let binding = scope
        .imports
        .iter()
        .rev()
        .find(|binding| import_local_name(binding) == root);
    match binding {
        Some(ImportBinding::Foreign {
            site: ForeignImportSite::Item(_),
            ..
        }) => {}
        Some(ImportBinding::Foreign { .. }) => {
            return refuse(format!(
                "uses `staticmethod({written})`, but the import of `{root}` is conditional (inside \
                 a module-level `if` or `try`), which is not supported yet (#1348)"
            ));
        }
        _ => {
            return refuse(format!(
                "uses `staticmethod({written})`, but `{root}` is not a foreign (CPython) import \
                 defined above this class -- `staticmethod` of a pycc function, class, or module \
                 is not supported yet (#1347)"
            ));
        }
    }
    if attr_name.starts_with("__") && attr_name.ends_with("__") {
        return refuse(
            "is a special (dunder) name, which CPython looks up implicitly and which is not \
             supported for a `staticmethod` class attribute (#1348)"
                .to_string(),
        );
    }
    if attr_name.starts_with("__") {
        return refuse(
            "is a class-private name, which CPython mangles and pycc does not model yet (#1348) \
             -- a `staticmethod` class attribute must not start with `__`"
                .to_string(),
        );
    }
    if RECEIVER_DISPATCHED_NAMES.contains(&attr_name) {
        return refuse(
            "is dispatched as a container method (`append`, `pop`, `get`, `add`), which a \
             `staticmethod` class attribute cannot use yet (#1348) -- choose another attribute \
             name"
                .to_string(),
        );
    }
    if spelling == Spelling::Annotated {
        return refuse(format!(
            "is annotated -- only the un-annotated spelling `{attr_name} = staticmethod(...)` is \
             supported for a foreign `staticmethod` class attribute yet (#1348)"
        ));
    }
    Ok(ForeignCallableRef { root, path })
}

/// A bare name or a chain of attribute accesses ending in one, as
/// `(root, path)`; `None` for any other expression shape.
fn reference_chain(expr: &Expr) -> Option<(String, Vec<String>)> {
    match expr {
        Expr::Name(name) => Some((name.id.to_string(), Vec::new())),
        Expr::Attribute(attribute) => {
            let (root, mut path) = reference_chain(&attribute.value)?;
            path.push(attribute.attr.to_string());
            Some((root, path))
        }
        _ => None,
    }
}

/// `root.path...` as the user wrote it.
fn render(root: &str, path: &[String]) -> String {
    std::iter::once(root)
        .chain(path.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(".")
}

/// Whether `root` is the local name of an unconditional module-level foreign
/// import, the one root [`classify_non_literal`] admits.
fn foreign_item_import(imports: &[ImportBinding], root: &str) -> bool {
    matches!(
        imports
            .iter()
            .rev()
            .find(|binding| import_local_name(binding) == root),
        Some(ImportBinding::Foreign {
            site: ForeignImportSite::Item(_),
            ..
        })
    )
}

/// #910: The natural type of an un-annotated class attribute's literal
/// right-hand side, or `None` when the shape is not one this pass folds.
///
/// A unary `+`/`-` is unwrapped first, because `X = -1` parses as
/// `UnaryOp(USub, NumberLiteral(1))` and is not a literal at all. Only
/// `USub`/`UAdd` are unwrapped: `~1` and `not True` are constant-foldable in
/// principle but are deliberately out of scope, so they yield `None` here and
/// are reported as an unsupported initializer shape.
///
/// This deliberately mirrors, and never widens, [`class_attr_value`]'s
/// accepted set. A complex literal (`1j`) has no `Ty` in this compiler and
/// yields `None`, so the un-annotated spelling sends it to
/// [`classify_non_literal`]; the annotated spelling reaches the same refusal
/// through `class_attr_value`'s complex arm.
pub(super) fn infer_class_attr_ty(value: &Expr) -> Option<Ty> {
    let inner = match value {
        Expr::UnaryOp(unary) if matches!(unary.op, UnaryOp::USub | UnaryOp::UAdd) => {
            unary.operand.as_ref()
        }
        other => other,
    };
    match inner {
        Expr::NumberLiteral(number) => match number.value {
            Number::Int(_) => Some(Ty::Int),
            Number::Float(_) => Some(Ty::Float),
            Number::Complex { .. } => None,
        },
        // `BooleanLiteral` is matched before `NumberLiteral` cannot apply --
        // Python's `True`/`False` parse as their own node, not as an int.
        Expr::BooleanLiteral(_) => Some(Ty::Bool),
        Expr::StringLiteral(_) => Some(Ty::Str),
        _ => None,
    }
}

/// The `C0001` for a class-attribute declaration with no initializer.
///
/// Shared by the annotated path's own check and #916's bare-`Final` arm,
/// which must reach the same conclusion before it has a type to infer.
pub(super) fn no_class_attr_value(attr_name: &str, range: std::ops::Range<u32>) -> Diagnostic {
    unsupported(
        format!(
            "class attribute `{attr_name}` has no value -- a class attribute is a \
             compile-time constant and must be initialized with a literal \
             (`{attr_name}: int = 1`)"
        ),
        range,
    )
}

/// The `C0001` for a class-attribute initializer whose shape this pass does
/// not fold, shared by both spellings so they report identically. Since
/// #1345 it is the fallback of [`classify_non_literal`], which both spellings
/// call for a non-literal initializer and which reports a more precise
/// message for every shape it names.
pub(super) fn bad_class_attr_shape(attr_name: &str, range: std::ops::Range<u32>) -> Diagnostic {
    unsupported(
        format!(
            "class attribute `{attr_name}` must be initialized with a literal -- only an \
             `int`, `float`, `str`, or `bool` literal (optionally with a unary `+`/`-` on a \
             number) is supported, because a class attribute is a compile-time constant"
        ),
        range,
    )
}

/// #911: Extracts the compile-time constant value of a class attribute from
/// its right-hand side, checking it against the declared annotation.
///
/// Accepted shapes are deliberately wider than the enum-member extractor's
/// (`class/enum_class.rs`): a unary `+`/`-` applied to a numeric literal is
/// accepted too, because the motivating example in #885 is
/// `MIN_WIDTH: int = -1024`, which parses as `UnaryOp(USub,
/// NumberLiteral(1024))` and not as a literal at all.
///
/// A shape it cannot fold is refused through [`classify_non_literal`], so a
/// non-literal initializer gets the same precise message whichever function
/// sees it first. Only the annotated spelling reaches this with a
/// non-literal (the un-annotated one routes those to the classifier itself),
/// and the annotated spelling never admits one.
pub(super) fn class_attr_value(
    value: &Expr,
    attr_ty: &Ty,
    attr_name: &str,
    spelling: Spelling,
    scope: &InitializerScope<'_>,
    range: std::ops::Range<u32>,
) -> Result<ClassAttrValue, Diagnostic> {
    let bad_shape = || {
        classify_non_literal(value, attr_name, spelling, scope, range.clone())
            .expect_err("a non-literal reaching the literal extractor is never admitted")
    };
    let mismatch = |found: &str| {
        unsupported(
            format!(
                "class attribute `{attr_name}` is annotated `{}` but is initialized with a \
                 `{found}` literal",
                attr_ty.name()
            ),
            range.clone(),
        )
    };
    // Unary `+`/`-` on a numeric literal, unwrapped to a signed number.
    let (negate, literal) = match value {
        Expr::UnaryOp(unary) => {
            let sign = match unary.op {
                pycc_ast::UnaryOp::USub => true,
                pycc_ast::UnaryOp::UAdd => false,
                _ => return Err(bad_shape()),
            };
            if !matches!(unary.operand.as_ref(), Expr::NumberLiteral(_)) {
                return Err(bad_shape());
            }
            (sign, unary.operand.as_ref())
        }
        other => (false, other),
    };
    match literal {
        Expr::NumberLiteral(number) => match &number.value {
            Number::Int(i) => {
                let Some(magnitude) = i.as_i64() else {
                    return Err(unsupported(
                        format!(
                            "class attribute `{attr_name}` has an integer value that does not \
                             fit in i64 -- only i64-range values are supported"
                        ),
                        range,
                    ));
                };
                let signed = if negate { -magnitude } else { magnitude };
                match attr_ty {
                    Ty::Int => Ok(ClassAttrValue::Int(signed)),
                    // An `int` literal under a `float` annotation widens,
                    // matching Python's own numeric tower (`x: float = 1`).
                    Ty::Float => Ok(ClassAttrValue::Float(signed as f64)),
                    _ => Err(mismatch("int")),
                }
            }
            Number::Float(f) => {
                let signed = if negate { -*f } else { *f };
                match attr_ty {
                    Ty::Float => Ok(ClassAttrValue::Float(signed)),
                    _ => Err(mismatch("float")),
                }
            }
            Number::Complex { .. } => Err(bad_shape()),
        },
        // `negate` is `true` only when the operand was a `NumberLiteral`
        // (checked above), so no sign can reach these two arms.
        Expr::BooleanLiteral(b) => match attr_ty {
            Ty::Bool => Ok(ClassAttrValue::Bool(b.value)),
            _ => Err(mismatch("bool")),
        },
        Expr::StringLiteral(s) => match attr_ty {
            Ty::Str => Ok(ClassAttrValue::Str(s.value.to_str().to_string())),
            _ => Err(mismatch("str")),
        },
        _ => Err(bad_shape()),
    }
}

#[cfg(test)]
mod tests {
    use super::{ClassAttrValue, Ty};
    use crate::lower_checked;

    /// Asserts `source` is rejected with a `C0001` whose message contains
    /// `needle`. `crate::class::tests::assert_c0001` pins only the code, and
    /// every case here needs the *specific* collision partner named.
    fn assert_collision(source: &str, needle: &str) {
        let module = crate::pycc_parser_test_helper::parse(source);
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001", "source: {source:?}");
        assert!(
            diagnostic.message.contains(needle),
            "expected {needle:?} in {:?}",
            diagnostic.message
        );
    }

    // -- #910: un-annotated class attributes, pure-rejection paths ---------
    //
    // The accepted surface and its constant folding are pinned end to end
    // through the CLI in `tests/issue_910_unannotated_class_attrs.rs`; these
    // cover the shapes that never produce a program to run.

    #[test]
    fn an_unannotated_complex_literal_is_rejected() {
        assert_collision(
            "class C:\n    X = 1j\n",
            "must be initialized with a literal",
        );
    }

    #[test]
    fn an_unannotated_bitwise_not_is_rejected() {
        assert_collision(
            "class C:\n    X = ~1\n",
            "must be initialized with a literal",
        );
    }

    #[test]
    fn an_unannotated_logical_not_is_rejected() {
        assert_collision(
            "class C:\n    X = not True\n",
            "must be initialized with a literal",
        );
    }

    #[test]
    fn an_unannotated_call_initializer_is_rejected() {
        assert_collision(
            "def f() -> int:\n    return 1\n\n\nclass C:\n    X = f()\n",
            "is initialized with a call, which is not supported yet (#1348)",
        );
    }

    #[test]
    fn an_unannotated_negated_call_initializer_is_rejected() {
        assert_collision(
            "def f() -> int:\n    return 1\n\n\nclass C:\n    X = -f()\n",
            "must be initialized with a literal",
        );
    }

    #[test]
    fn an_unannotated_bare_name_initializer_is_rejected() {
        assert_collision(
            "Y: int = 1\n\n\nclass C:\n    X = Y\n",
            "is initialized with a name or attribute reference",
        );
    }

    #[test]
    fn an_unannotated_arithmetic_initializer_is_rejected() {
        assert_collision(
            "class C:\n    X = 1 + 2\n",
            "must be initialized with a literal",
        );
    }

    /// A unary `-` is unwrapped before inference, so `-"a"` infers `str` and
    /// is caught by the literal extractor's own numeric-operand check rather
    /// than by inference. Both report the same message.
    #[test]
    fn an_unannotated_negated_string_is_rejected() {
        assert_collision(
            "class C:\n    X = -\"a\"\n",
            "must be initialized with a literal",
        );
    }

    #[test]
    fn an_unannotated_int_outside_i64_is_rejected() {
        assert_collision(
            "class C:\n    X = 99999999999999999999999\n",
            "does not fit in i64",
        );
    }

    /// The annotated path resolves its type first and then checks the literal
    /// against it, so a well-shaped literal of the wrong type is rejected by
    /// the shared value lowering rather than by annotation resolution.
    #[test]
    fn an_annotated_class_attribute_with_a_mismatched_literal_is_rejected() {
        assert_collision(
            "class C:\n    X: int = \"a\"\n",
            "is annotated `int` but is initialized with a `str` literal",
        );
    }

    /// The annotated spelling reaches the literal extractor with any
    /// initializer: a unary operator other than `+`/`-` and a complex
    /// literal are refused there through the classifier, and a `float` or
    /// `bool` literal under a different annotation is a mismatch.
    #[test]
    fn annotated_literal_refusals_name_the_literal_or_the_shape() {
        for value in ["~1", "1j", "-\"a\""] {
            assert_collision(
                &format!("class C:\n    X: int = {value}\n"),
                "must be initialized with a literal",
            );
        }
        assert_collision(
            "class C:\n    X: int = 1.5\n",
            "is annotated `int` but is initialized with a `float` literal",
        );
        assert_collision(
            "class C:\n    X: int = True\n",
            "is annotated `int` but is initialized with a `bool` literal",
        );
    }

    // -- #910: every inferred literal shape, at the lowering seam ---------

    #[test]
    fn every_unannotated_literal_shape_infers_its_natural_type() {
        let module = crate::pycc_parser_test_helper::parse(
            "class C:\n    I = 1\n    F = 1.5\n    B = True\n    S = \"cfg\"\n    NI = -1024\n    PI = +2048\n    NF = -1.5\n    NB = False\n",
        );
        let hir = lower_checked(&module).expect("the class body must lower");
        let (_, class_def) = hir
            .class_defs
            .iter()
            .find(|(name, _)| name == "C")
            .expect("class `C` must be lowered");

        assert_eq!(
            class_def.class_attrs,
            vec![
                ("I".to_string(), Ty::Int, ClassAttrValue::Int(1)),
                ("F".to_string(), Ty::Float, ClassAttrValue::Float(1.5)),
                ("B".to_string(), Ty::Bool, ClassAttrValue::Bool(true)),
                (
                    "S".to_string(),
                    Ty::Str,
                    ClassAttrValue::Str("cfg".to_string())
                ),
                ("NI".to_string(), Ty::Int, ClassAttrValue::Int(-1024)),
                ("PI".to_string(), Ty::Int, ClassAttrValue::Int(2048)),
                ("NF".to_string(), Ty::Float, ClassAttrValue::Float(-1.5)),
                ("NB".to_string(), Ty::Bool, ClassAttrValue::Bool(false)),
            ]
        );
        // D-154 / D-224: an inferred class attribute is a compile-time
        // constant, so it takes no instance slot -- the same invariant the
        // annotated spelling carries.
        assert!(class_def.attrs.is_empty());
    }

    // -- #1345: the non-literal classifier ---------------------------------
    //
    // Every refusal is also pinned through the CLI in
    // `tests/issue_1284_foreign_static_class_attr.rs`; these pin the
    // admitted value and the hint's presence or absence at the unit level,
    // with every import answered as a foreign module.

    use crate::class::foreign_static::tests::lower_foreign;

    /// `source` fails with a `C0001` containing `needle`.
    fn assert_foreign_c0001(source: &str, needle: &str) {
        let diagnostic = lower_foreign(source).unwrap_err();
        assert_eq!(diagnostic.code, "C0001", "source: {source:?}");
        assert!(
            diagnostic.message.contains(needle),
            "expected {needle:?} in {:?}",
            diagnostic.message
        );
    }

    /// The admitted value of `source`'s class `C`, attribute `x`.
    fn admitted(source: &str) -> ClassAttrValue {
        let hir = lower_foreign(source).expect("the attribute is admitted");
        let (_, class_def) = hir
            .class_defs
            .iter()
            .find(|(name, _)| name == "C")
            .expect("class C");
        let (_, ty, value) = class_def
            .class_attrs
            .iter()
            .find(|(name, _, _)| name == "x")
            .expect("attribute x");
        assert_eq!(*ty, Ty::Object);
        value.clone()
    }

    #[test]
    fn a_dotted_foreign_reference_is_admitted_with_its_path() {
        let value = admitted("import os\n\n\nclass C:\n    x = staticmethod(os.path.exists)\n");
        assert!(
            matches!(&value, ClassAttrValue::ForeignStatic(target)
                if target.root == "os" && target.path == ["path", "exists"]),
            "{value:?}"
        );
    }

    #[test]
    fn a_bare_foreign_name_and_a_module_are_admitted() {
        let value = admitted("from operator import add\n\n\nclass C:\n    x = staticmethod(add)\n");
        assert!(
            matches!(&value, ClassAttrValue::ForeignStatic(target)
                if target.root == "add" && target.path.is_empty()),
            "{value:?}"
        );
        assert!(matches!(
            admitted("import os\n\n\nclass C:\n    x = staticmethod(os)\n"),
            ClassAttrValue::ForeignStatic(_)
        ));
    }

    /// The self-named `mul = staticmethod(mul)`: the value is evaluated
    /// before the target is bound, so the root is still the import.
    #[test]
    fn a_self_named_attribute_is_admitted() {
        assert!(
            lower_foreign("from operator import mul\n\n\nclass C:\n    mul = staticmethod(mul)\n")
                .is_ok()
        );
    }

    #[test]
    fn a_reference_to_a_foreign_import_carries_the_staticmethod_hint() {
        assert_foreign_c0001(
            "import os\n\n\nclass C:\n    x = os.path.exists\n",
            "; to bind a CPython callable, write `x = staticmethod(os.path.exists)`",
        );
        let module = crate::pycc_parser_test_helper::parse("Y: int = 1\n\n\nclass C:\n    X = Y\n");
        let diagnostic = lower_checked(&module).unwrap_err();
        assert!(!diagnostic.message.contains("to bind a CPython callable"));
    }

    #[test]
    fn every_container_shape_gets_the_container_refusal() {
        for value in [
            "(1, 2)",
            "{1, 2}",
            "{1: 2}",
            "{a for a in range(2)}",
            "{a: a for a in range(2)}",
            "(a for a in range(2))",
        ] {
            assert_collision(
                &format!("class C:\n    X = {value}\n"),
                "is initialized with a container",
            );
        }
    }

    #[test]
    fn a_call_of_a_non_name_callee_is_a_call_refusal() {
        assert_foreign_c0001(
            "import os\n\n\nclass C:\n    X = os.getcwd()\n",
            "is initialized with a call",
        );
    }

    #[test]
    fn a_starred_argument_is_an_arity_refusal() {
        assert_foreign_c0001(
            "from operator import add\nxs = [add]\n\n\nclass C:\n    x = staticmethod(*xs)\n",
            "which takes exactly one positional argument",
        );
    }
}
