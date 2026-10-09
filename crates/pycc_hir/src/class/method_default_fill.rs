//! Filling omitted trailing defaults at an in-module instance-method call
//! (Part 1 of #1191, issue #1438).
//!
//! The method part of #1140 records each method's admitted literal defaults
//! in [`HirClassDef::method_defaults`]. Until this part only the `--ext` host
//! boundary read them. An in-module call such as `self.copy()` that left a
//! defaulted parameter out was refused with the arity `T0021`.
//!
//! `HirExpr::MethodCall` stays purely positional. Only `pycc_types` knows the
//! receiver's class, and so the method a call resolves to, which means the
//! fill cannot happen during HIR lowering the way a module-level `def`'s fill
//! does (`expr::keyword_bind`). Instead:
//!
//! * `pycc_types` accepts a short call that the resolved method's defaults
//!   cover, and checks the defaults as if they had been written at the call
//!   site;
//! * `pycc_mir` appends the same default nodes to the call's argument
//!   vector.
//!
//! Both phases ask [`HirClassDef::omitted_method_defaults`], so they cannot
//! disagree about which calls are filled or with what. Both pass the class
//! whose method table the MRO walk matched, together with the mangled name
//! found there. That is the key `method_defaults` is stored under, and it is
//! never the name of a receiver-exact copy (D-254), because copies never
//! appear in a class's tables.
//!
//! A spliced node is the one `func::params::literal_default` produced from
//! the `def`. It is byte-identical to the same literal written at the call
//! site, so a filled call compiles exactly like the explicit one. Defaults
//! are literals, so appending them after the supplied arguments cannot
//! change the order in which any argument is evaluated.

use crate::{HirClassDef, HirExpr};

impl HirClassDef {
    /// The default nodes to append to a call of this class's own regular
    /// method `mangled`, given `supplied` positional arguments, excluding
    /// the receiver.
    ///
    /// Returns `Some` only when the call omits at least one parameter and
    /// every omitted parameter declares a default. Returns `None` in every
    /// other case, and the caller keeps its ordinary arity check:
    ///
    /// * the method records no defaults;
    /// * the call supplies every parameter, or more than the method takes;
    /// * an omitted parameter is required.
    ///
    /// `mangled` must name a method with a receiver: a regular method or
    /// `__init__`. Its recorded defaults lead with the receiver's `None`
    /// entry, which this skips.
    pub fn omitted_method_defaults(&self, mangled: &str, supplied: usize) -> Option<Vec<&HirExpr>> {
        let (_, defaults) = self
            .method_defaults
            .iter()
            .find(|(held, _)| held == mangled)?;
        let omitted = defaults
            .get(1 + supplied..)
            .filter(|rest| !rest.is_empty())?;
        omitted.iter().map(Option::as_ref).collect()
    }
}

#[cfg(test)]
#[path = "method_default_fill_tests.rs"]
mod tests;
