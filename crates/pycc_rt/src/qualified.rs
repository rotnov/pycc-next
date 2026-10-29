//! Display names of qualified module globals (v0.3 accept 1).
//!
//! A class or function a dependency module of a multi-module program
//! defines is compiled under the qualified spelling
//! `pycc_ast::namespace::module_global` gives it (`0m6helper_Point` for
//! `helper.py`'s `Point`). The runtime shows such a name as CPython does:
//! `helper.Point` where CPython qualifies a class by its module (an
//! uncaught exception's type, an instance's default `repr`) and `Point`
//! where it does not (an `AttributeError` message). This mirrors
//! `pycc_ast::namespace::split_module_global`; both crates pin the format
//! with the same examples.

/// The dotted module name and the plain name of a qualified spelling, or
/// `None` for any other name.
pub(crate) fn split(name: &str) -> Option<(String, &str)> {
    let mut rest = name.strip_prefix("0m")?;
    let mut module = String::new();
    loop {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            break;
        }
        let len: usize = rest[..digits].parse().ok()?;
        let segment = rest.get(digits..digits + len)?;
        if segment.is_empty() {
            return None;
        }
        if !module.is_empty() {
            module.push('.');
        }
        module.push_str(segment);
        rest = &rest[digits + len..];
    }
    if module.is_empty() {
        return None;
    }
    Some((module, rest.strip_prefix('_')?))
}

/// The plain name: `Point` for `0m6helper_Point`, `name` otherwise; and
/// (v0.10) `Owned` for `__pycc_excl_Owned__`, the hidden class declaring
/// the native layout of the exception class `Owned` (`pycc`'s
/// `fallback::exc_layout`), whose attributes CPython code sees on the
/// exception's objects.
pub(crate) fn plain(name: &str) -> &str {
    let name = split(name).map_or(name, |(_, plain)| plain);
    name.strip_prefix("__pycc_excl_")
        .and_then(|rest| rest.strip_suffix("__"))
        .unwrap_or(name)
}

/// The module-qualified name CPython shows for a class: `helper.Point` for
/// a dependency's class, `__main__.Point` for the entry module's own.
pub(crate) fn with_module(name: &str) -> String {
    match split(name) {
        Some((module, plain)) => format!("{module}.{plain}"),
        None => format!("__main__.{name}"),
    }
}

/// The name an uncaught exception's type prints as: `helper.MyError` for a
/// dependency's class, the name unchanged otherwise (CPython leaves out
/// `__main__.` and `builtins.`).
pub(crate) fn exception(name: &str) -> String {
    match split(name) {
        Some((module, plain)) => format!("{module}.{plain}"),
        None => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualified_spellings_display_as_cpython_names() {
        assert_eq!(split("0m3pkg3sub_X"), Some(("pkg.sub".to_string(), "X")));
        assert_eq!(split("0m6helper__p"), Some(("helper".to_string(), "_p")));
        assert_eq!(split("0gen_C"), None);
        assert_eq!(split("0m_x"), None);
        assert_eq!(split("0m9short_x"), None);
        assert_eq!(plain("0m6helper_Point"), "Point");
        assert_eq!(plain("Point"), "Point");
        assert_eq!(plain("__pycc_excl_Owned__"), "Owned");
        assert_eq!(plain("0m6helper___pycc_excl_E__"), "E");
        assert_eq!(plain("__pycc_excl_"), "__pycc_excl_");
        assert_eq!(with_module("0m6helper_Point"), "helper.Point");
        assert_eq!(with_module("Point"), "__main__.Point");
        assert_eq!(exception("0m3pkg3sub_E"), "pkg.sub.E");
        assert_eq!(exception("ValueError"), "ValueError");
    }
}
