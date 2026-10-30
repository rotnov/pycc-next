//! One object-file section per function on ELF targets, as C compilers'
//! `-ffunction-sections` does.
//!
//! The assembler lays out each section until every branch has its final
//! size: a jump whose target moves out of a short jump's range grows, which
//! moves what follows it, and the section is laid out again. In one `.text`
//! section holding every function of a large module, each pass walks the
//! whole section, and a module of a few thousand Python lines takes
//! hundreds of passes: most of the backend's time for diff-match-patch
//! went there. A section per function bounds each layout by its function.
//!
//! The linker merges `.text.<name>` into `.text` (its default script), so
//! the artifact is laid out as before. A function already given a section
//! keeps it; one LLVM would place in `.text.unlikely` (it is `cold`) goes
//! to `.text.unlikely.<name>`, where the linker merges it with the other
//! cold code. Other object formats are left alone: Mach-O lays out per
//! symbol already (`.subsections_via_symbols`), and COFF names sections
//! another way.

use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::module::Module;

/// Gives every function `module` defines its own section, when `triple`
/// names an ELF target (Linux).
pub(crate) fn assign(module: &Module<'_>, triple: &str) {
    if !is_elf(triple) {
        return;
    }
    let cold = Attribute::get_named_enum_kind_id("cold");
    for function in module.get_functions() {
        if function.count_basic_blocks() == 0 || function.get_section().is_some() {
            continue;
        }
        let prefix = if function
            .get_enum_attribute(AttributeLoc::Function, cold)
            .is_some()
        {
            ".text.unlikely."
        } else {
            ".text."
        };
        let name = function.get_name().to_string_lossy();
        function.set_section(Some(&format!("{prefix}{name}")));
    }
}

/// Whether `triple` names a target whose objects are ELF: a Linux one.
fn is_elf(triple: &str) -> bool {
    triple.contains("-linux")
}

#[cfg(test)]
mod tests {
    use super::*;
    use inkwell::context::Context;

    fn module_with_functions(context: &Context) -> Module<'_> {
        let module = context.create_module("sections");
        let fn_type = context.void_type().fn_type(&[], false);
        for name in ["plain", "chilly", "placed"] {
            let function = module.add_function(name, fn_type, None);
            let block = context.append_basic_block(function, "entry");
            let builder = context.create_builder();
            builder.position_at_end(block);
            builder.build_return(None).expect("a return");
        }
        let cold = Attribute::get_named_enum_kind_id("cold");
        module
            .get_function("chilly")
            .expect("defined")
            .add_attribute(
                AttributeLoc::Function,
                context.create_enum_attribute(cold, 0),
            );
        module
            .get_function("placed")
            .expect("defined")
            .set_section(Some(".text.mine"));
        module.add_function("declared", fn_type, None);
        module
    }

    fn section(module: &Module<'_>, name: &str) -> Option<String> {
        module
            .get_function(name)
            .expect("in the module")
            .get_section()
            .map(|s| s.to_string_lossy().into_owned())
    }

    #[test]
    fn an_elf_target_gets_a_section_per_defined_function() {
        let context = Context::create();
        let module = module_with_functions(&context);
        assign(&module, "x86_64-unknown-linux-gnu");
        assert_eq!(section(&module, "plain").as_deref(), Some(".text.plain"));
        assert_eq!(
            section(&module, "chilly").as_deref(),
            Some(".text.unlikely.chilly")
        );
        assert_eq!(section(&module, "placed").as_deref(), Some(".text.mine"));
        assert_eq!(section(&module, "declared"), None);
    }

    #[test]
    fn other_object_formats_are_left_alone() {
        for triple in ["arm64-apple-darwin", "x86_64-pc-windows-msvc"] {
            let context = Context::create();
            let module = module_with_functions(&context);
            assign(&module, triple);
            assert_eq!(section(&module, "plain"), None, "{triple}");
            assert_eq!(section(&module, "chilly"), None, "{triple}");
        }
    }
}
