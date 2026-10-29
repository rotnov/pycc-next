//! v0.10 accept 5: the native layouts of exception classes CPython makes
//! (`crate::fallback`'s `exc_layout`), as the shim's templates read them
//! (`src/ext/module/exc_templates.c`).

use super::*;

/// The C declaration of the generated lookup of a layout by its hidden
/// class's name (`PyccExtModule.exc_layout`).
pub(crate) const EXC_LAYOUT_DECL: &str =
    "static const struct PyccExtExcLayout *pycc_ext_exc_layout(const char *hidden)";

/// The table of every hidden layout class among `carriers` -- its name, its
/// record, its slot count and its descriptor table -- and the lookup over
/// it. Always emitted (an empty table when the module has none), so the
/// module's record can name the lookup unconditionally.
pub(crate) fn exc_layouts_c(carriers: &[ExtCarrierClass], getsets: &[ExtClassGetsets]) -> String {
    let mut out = String::from("/* Exception layouts (src/ext/module/exc_templates.c). */\n");
    let mut rows = String::new();
    for carrier in carriers {
        let class = carrier.class.as_str();
        if !class.starts_with(crate::fallback::LAYOUT_PREFIX) {
            continue;
        }
        let CarrierCopy::Kinds(kinds) = &carrier.copy else {
            continue;
        };
        let record = pycc_codegen::record_symbol(class);
        let table = if getsets.iter().any(|table| table.class == class) {
            format!("pycc_ext_type_getset_{class}")
        } else {
            "NULL".to_string()
        };
        out.push_str(&format!("extern const unsigned char {record}[];\n"));
        rows.push_str(&format!(
            "    {{\"{class}\", {record}, {}, {table}}},\n",
            kinds.len()
        ));
    }
    out.push_str(&format!(
        "static const PyccExtExcLayout pycc_ext_exc_layouts[] = {{\n{rows}    \
         {{NULL, NULL, 0, NULL}},\n}};\n\n{EXC_LAYOUT_DECL}\n{{\n    \
         const PyccExtExcLayout *row;\n    \
         for (row = pycc_ext_exc_layouts; row->hidden != NULL; row++) {{\n        \
         if (strcmp(row->hidden, hidden) == 0) {{\n            return row;\n        }}\n    \
         }}\n    return NULL;\n}}\n\n"
    ));
    out
}
