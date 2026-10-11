//! Generators: the builtins that consume a generator argument.
//!
//! `pycc_types` types `next(g)` as the generator's element type and the
//! consumers `pycc_types::generator::is_generator_consumer` names as if
//! given the list of the generator's values. `next`, `list`, `any` and `all`
//! get their own nodes; every other consumer receives that list
//! (`MirExpr::GenToList`), which it consumes entirely anyway.

use crate::{MirExpr, Ty};

/// The lowering of builtin call `callee(args)` when its first argument is a
/// generator, or the arguments back (a generator argument of a consumer
/// wrapped in `GenToList`) for the ordinary lowering to continue with.
pub(crate) fn lower_generator_consumer(
    callee: &str,
    mut args: Vec<MirExpr>,
) -> Result<MirExpr, Vec<MirExpr>> {
    if !args.iter().any(|arg| matches!(arg.ty(), Ty::Generator(_))) {
        return Err(args);
    }
    let first = args.first().map(MirExpr::ty);
    match (callee, first) {
        ("next", Some(Ty::Generator(element))) => {
            let mut args = args.into_iter();
            let generator = Box::new(args.next().expect("checked above"));
            Ok(MirExpr::GenNext {
                generator,
                default: args.next().map(Box::new),
                ty: *element,
            })
        }
        ("list", Some(Ty::Generator(element))) if args.len() == 1 => Ok(MirExpr::GenToList {
            generator: Box::new(args.remove(0)),
            ty: Ty::List(element),
        }),
        ("any" | "all", Some(Ty::Generator(_))) if args.len() == 1 => Ok(MirExpr::GenAnyAll {
            generator: Box::new(args.remove(0)),
            all: callee == "all",
        }),
        _ if is_consumer(callee) => Err(args
            .into_iter()
            .map(|arg| match arg.ty() {
                Ty::Generator(element) => MirExpr::GenToList {
                    generator: Box::new(arg),
                    ty: Ty::List(element),
                },
                _ => arg,
            })
            .collect()),
        _ => Err(args),
    }
}

/// `pycc_types::generator::is_generator_consumer`'s list (this crate does
/// not depend on `pycc_types`).
fn is_consumer(callee: &str) -> bool {
    matches!(
        callee,
        "list" | "sum" | "min" | "max" | "sorted" | "set" | "frozenset" | "any" | "all" | "tuple"
    )
}
