//! A region's slow calls and conversions (`pycc_ast::specialize_regions`,
//! docs/design/selected-regions.md section 5.4): the code a native copy runs
//! only after a member was rebound while a region call was running. It has
//! no frame of its own (`frames::begin_function`): what it raises is raised
//! in its caller's, at the region call. And it is kept out of line
//! ([`mark`]), so the copies' hot paths branch around its calls.

use inkwell::context::Context;
use inkwell::values::FunctionValue;

/// The prefix of a region's conversions of a generic arm's result
/// (`__pycc_rconv_<shape>`, `pycc_ast::specialize_regions`).
const CONVERSION: &str = "__pycc_rconv_";

/// Whether `name`, or the function it is the native multi-value twin of
/// (`pycc_mir::multi_return::twin_name`), is a region's slow call or
/// conversion.
pub(crate) fn is_slow(name: &str) -> bool {
    let name = name.strip_prefix("0gen_multi_").unwrap_or(name);
    name.starts_with(pycc_hir::REGION_SLOW_PREFIX) || name.starts_with(CONVERSION)
}

/// Marks a slow call ([`is_slow`]) `cold` and `noinline`.
pub(crate) fn mark<'ctx>(context: &'ctx Context, function: FunctionValue<'ctx>) {
    crate::frames::mark_cold(context, function);
    let kind = inkwell::attributes::Attribute::get_named_enum_kind_id("noinline");
    function.add_attribute(
        inkwell::attributes::AttributeLoc::Function,
        context.create_enum_attribute(kind, 0),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_calls_conversions_and_their_twins_are_slow() {
        assert_eq!(pycc_mir::multi_return::twin_name("f"), "0gen_multi_f");
        for name in ["__pycc_rslow3", "0gen_multi___pycc_rslow3", "__pycc_rconv_io", "0gen_multi___pycc_rconv_io"] {
            assert!(is_slow(name), "{name}");
        }
        for name in ["__pycc_rnative_lift1_f__", "__pycc_lift1_f__", "0gen_multi2___pycc_lift1_f__", "rslow"] {
            assert!(!is_slow(name), "{name}");
        }
    }

    #[test]
    fn a_slow_call_is_cold_and_never_inlined() {
        let context = Context::create();
        let module = context.create_module("m");
        let f = module.add_function("pyfn___pycc_rslow0", context.void_type().fn_type(&[], false), None);
        mark(&context, f);
        let kind = |name: &str| inkwell::attributes::Attribute::get_named_enum_kind_id(name);
        for name in ["cold", "noinline"] {
            assert!(
                f.get_enum_attribute(inkwell::attributes::AttributeLoc::Function, kind(name)).is_some(),
                "{name}"
            );
        }
    }
}
