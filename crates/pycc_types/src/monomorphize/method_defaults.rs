//! Carrying a generic class's method defaults onto each specialization
//! (Part 1 of #1191, issue #1438).
//!
//! `HirClassDef::method_defaults` is keyed by a method's mangled name, and
//! monomorphization renames every regular method from `Box.get` to
//! `Box[int].get`. Without re-keying, `check` (which reads the origin class)
//! would fill `Box(1).get()` while `build` (which re-infers against the
//! specialization) would report the arity `T0021`.
//!
//! The default nodes are copied unchanged. A parameter annotated with the
//! class's type parameter cannot declare a default (HIR lowering reports
//! `C0001` at the `def`), so every recorded default is a literal whose type
//! does not mention the type parameter and needs no substitution.

use pycc_hir::HirExpr;

/// The specialization's `method_defaults`: each origin entry whose mangled
/// name `renames` maps, re-keyed to the specialized mangled name.
///
/// `renames` holds `(origin mangled, specialized mangled)` pairs for the
/// regular methods the specialization actually carries; an origin entry
/// with no pair is dropped, exactly as its method is.
pub(super) fn rekey_method_defaults(
    origin: &[(String, Vec<Option<HirExpr>>)],
    renames: &[(String, String)],
) -> Vec<(String, Vec<Option<HirExpr>>)> {
    origin
        .iter()
        .filter_map(|(mangled, defaults)| {
            renames
                .iter()
                .find(|(from, _)| from == mangled)
                .map(|(_, to)| (to.clone(), defaults.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::rekey_method_defaults;
    use pycc_hir::HirExpr;

    #[test]
    fn a_renamed_method_keeps_its_defaults_and_an_unrenamed_one_is_dropped() {
        let origin = vec![
            (
                "Box.get".to_string(),
                vec![None, Some(HirExpr::IntLiteral(1))],
            ),
            ("Box.gone".to_string(), vec![None, None]),
        ];
        let renames = vec![("Box.get".to_string(), "Box[int].get".to_string())];
        let rekeyed = rekey_method_defaults(&origin, &renames);
        assert_eq!(rekeyed.len(), 1);
        assert_eq!(rekeyed[0].0, "Box[int].get");
        assert!(matches!(rekeyed[0].1[1], Some(HirExpr::IntLiteral(1))));
    }
}
