//! The exception tags of a package's members.
//!
//! One registry in the shim maps every user exception class of the
//! extension to its tag and back (`bridge.c`), so no two members may give a
//! class the same tag. A member's classes take consecutive tags from its
//! base (`pycc build --exception-tag-base`); the package build gives each
//! member a range at least as large as the number of exception classes it
//! can declare, counted from the source before anything is compiled: every
//! class statement whose base names a builtin exception class, a name that
//! reads as an exception class (`...Error`, `...Exception`, `...Warning`),
//! or another class so counted anywhere in the package -- an
//! over-approximation, since a base's name is all the source says -- plus
//! one for a module that declares a frozen dataclass, whose
//! `FrozenInstanceError` the frontend synthesizes. Each member reports the
//! highest tag it took and the package build checks it against the next
//! member's base, so a range that was too small fails the build instead of
//! two classes sharing a tag.

use pycc_ast::visitor::{Visitor, walk_stmt};
use pycc_ast::{Expr, Stmt};
use std::collections::BTreeSet;

/// One class statement: its name and the names its bases end in.
struct ClassStmt {
    name: String,
    bases: Vec<String>,
}

struct Classes(Vec<ClassStmt>);

/// The name a base expression ends in: `E`, `mod.E`, `Generic[T]` -> `Generic`.
fn base_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Name(name) => Some(name.id.to_string()),
        Expr::Attribute(attr) => Some(attr.attr.to_string()),
        Expr::Subscript(sub) => base_name(&sub.value),
        _ => None,
    }
}

impl<'a> Visitor<'a> for Classes {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        if let Stmt::ClassDef(class) = stmt {
            self.0.push(ClassStmt {
                name: class.name.to_string(),
                bases: class.bases().iter().filter_map(base_name).collect(),
            });
        }
        walk_stmt(self, stmt);
    }
}

/// Whether a base's name reads as an exception class.
fn names_an_exception(name: &str) -> bool {
    pycc_hir::BUILTIN_EXCEPTION_CLASSES.contains(&name)
        || [
            "Error",
            "Exception",
            "Warning",
            "BaseException",
            "Exit",
            "Interrupt",
        ]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

/// The first exception tag of each module whose source is in `sources` (in
/// order), and the first tag past them all; `Err` when the package can
/// declare more exception classes than the tag space holds.
pub(crate) fn exception_tag_bases(sources: &[String]) -> Result<(Vec<u8>, u16), String> {
    let classes: Vec<Classes> = sources
        .iter()
        .map(|source| {
            let mut found = Classes(Vec::new());
            // A module that does not parse fails its own member build with
            // its own diagnostics; it declares no class here.
            if let Ok(module) = pycc_parser::parse_all(source) {
                for stmt in &module.body {
                    found.visit_stmt(stmt);
                }
            }
            found
        })
        .collect();
    // Every name some class of the package declares that derives, by the
    // names of its bases, from an exception class: grown to a fixpoint.
    let mut exceptional: BTreeSet<&str> = BTreeSet::new();
    loop {
        let before = exceptional.len();
        for module in &classes {
            for class in &module.0 {
                if class
                    .bases
                    .iter()
                    .any(|base| names_an_exception(base) || exceptional.contains(base.as_str()))
                {
                    exceptional.insert(&class.name);
                }
            }
        }
        if exceptional.len() == before {
            break;
        }
    }
    let mut next = u16::from(pycc_hir::FIRST_USER_EXCEPTION_TYPE_TAG);
    let mut bases = Vec::with_capacity(sources.len());
    for (module, source) in classes.iter().zip(sources) {
        bases.push(next as u8);
        let declared = module
            .0
            .iter()
            .filter(|class| exceptional.contains(class.name.as_str()))
            .count();
        let frozen = usize::from(source.contains("frozen"));
        next += (declared + frozen) as u16;
        if next > u16::from(pycc_hir::FOREIGN_BASE_EXCEPTION_TYPE_TAG) {
            return Err(format!(
                "the package can declare more exception classes than one extension supports \
                 ({}); build it as separate modules",
                pycc_hir::FOREIGN_BASE_EXCEPTION_TYPE_TAG - pycc_hir::FIRST_USER_EXCEPTION_TYPE_TAG
            ));
        }
    }
    Ok((bases, next))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_module_gets_a_range_as_large_as_its_exception_classes() {
        let first = pycc_hir::FIRST_USER_EXCEPTION_TYPE_TAG;
        let sources = [
            "class E(ValueError):\n    pass\nclass Plain:\n    pass\n".to_string(),
            "import a\nclass F(a.E):\n    pass\nclass G(F, Generic[T]):\n    pass\n".to_string(),
            "def f():\n    class Local(LookupError):\n        pass\n".to_string(),
            "@dataclass(frozen=True)\nclass D:\n    x: int\n".to_string(),
            "x = 1\n".to_string(),
        ];
        let (bases, next) = exception_tag_bases(&sources).unwrap();
        assert_eq!(bases, [first, first + 1, first + 3, first + 4, first + 5]);
        assert_eq!(next, u16::from(first) + 5);
    }

    #[test]
    fn a_package_past_the_tag_space_is_refused() {
        let source: String = (0..300)
            .map(|i| format!("class E{i}(ValueError):\n    pass\n"))
            .collect();
        assert!(
            exception_tag_bases(&[source])
                .unwrap_err()
                .contains("separate modules")
        );
    }
}
