//! A member's interface (v0.8, the whole-package build's cross-module
//! native calls): the module-level functions of one module that the other
//! modules of its package may call natively.
//!
//! A member build writes its interface ([`INTERFACE_NAME`]) once its MIR is
//! built -- after the fallback loop converged, before LLVM runs -- so a
//! member importing it can start compiling while it is still in codegen.
//! Each function is listed with what a caller needs to call its compiled
//! body directly: its name (the symbol is the producer's prefix and
//! `pyfn_<name>`, which keeps its signature: it has external linkage) and
//! its parameter and return types, from which the caller's codegen derives
//! the same LLVM signature the producer's codegen did. Only signatures of
//! scalar and object types are listed ([`interface_ty`]), and only
//! functions defined once, published by the module (so a caller can tell
//! whether its name still binds the published object), and called with
//! nothing but their parameters (no buffer out-pointers, no multi-value
//! twins).
//!
//! The function's index is its position in the list: the producer's module
//! unit keeps, at that index of `pycc_ext_interface_table`, the object its
//! module dict binds to the name when its body has run
//! ([`Interface::c_header`]), and a caller calls natively only while its own
//! global still binds that object (`crate::package_build::siblings`).
//!
//! The text is the contract: [`content_hash`] of it is recorded by every
//! consumer and checked by the package build before the link.

use pycc_hir::Ty;

/// The interface file a member build writes into its output directory.
pub(crate) const INTERFACE_NAME: &str = "interface.txt";

/// The C header a member build writes beside its module unit.
pub(crate) const INTERFACE_HEADER_NAME: &str = "pycc_ext_interface.h";

/// The first line of every interface file.
const MAGIC: &str = "pycc-interface 1";

/// One function of an interface.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InterfaceFn {
    pub(crate) name: String,
    pub(crate) params: Vec<Ty>,
    pub(crate) ret: Ty,
}

/// A member's interface.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Interface {
    /// The module's dotted name.
    pub(crate) module: String,
    /// Its symbol prefix (`pycc_codegen::member_prefix`).
    pub(crate) prefix: String,
    pub(crate) functions: Vec<InterfaceFn>,
}

/// The spelling of a type an interface may carry, or `None` for one it
/// does not: `int`, `float`, `bool`, `str` and `object` (and `None` as a
/// return type), each a type the boundary converts to and from a CPython
/// object, so a caller can always fall back to calling the object.
pub(crate) fn interface_ty(ty: &Ty) -> Option<&'static str> {
    match ty {
        Ty::Int => Some("int"),
        Ty::Float => Some("float"),
        Ty::Bool => Some("bool"),
        Ty::Str => Some("str"),
        Ty::Object => Some("object"),
        Ty::None => Some("None"),
        _ => None,
    }
}

fn parse_ty(text: &str) -> Option<Ty> {
    Some(match text {
        "int" => Ty::Int,
        "float" => Ty::Float,
        "bool" => Ty::Bool,
        "str" => Ty::Str,
        "object" => Ty::Object,
        "None" => Ty::None,
        _ => return None,
    })
}

/// FNV-1a over `text`: what a consumer records of the interface it read.
pub(crate) fn content_hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

impl InterfaceFn {
    /// Whether a function of these types may be listed: every parameter
    /// and the return type has a spelling, and only the return is `None`.
    fn listable(params: &[Ty], ret: &Ty) -> bool {
        params
            .iter()
            .all(|ty| *ty != Ty::None && interface_ty(ty).is_some())
            && interface_ty(ret).is_some()
    }
}

impl Interface {
    /// The interface of the member `module` (symbol prefix `prefix`) whose
    /// MIR is `mir`: every function `published` names that
    /// [`InterfaceFn::listable`] admits, defined once and called with its
    /// parameters alone, in MIR order.
    pub(crate) fn collect(
        module: &str,
        prefix: &str,
        mir: &pycc_mir::MirModule,
        published: &std::collections::BTreeSet<String>,
    ) -> Self {
        let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for item in &mir.items {
            if let pycc_mir::MirItem::Function { name, .. } = item {
                *counts.entry(name.as_str()).or_insert(0) += 1;
            }
        }
        let mut functions = Vec::new();
        for item in &mir.items {
            let pycc_mir::MirItem::Function {
                name,
                params,
                return_ty,
                body,
            } = item
            else {
                continue;
            };
            let params: Vec<Ty> = params.iter().map(|(_, ty)| ty.clone()).collect();
            if published.contains(name)
                && counts[name.as_str()] == 1
                && pycc_mir::multi_return::out_tys(body).is_empty()
                && !pycc_codegen::body_returns_buffer_slice(body)
                && InterfaceFn::listable(&params, return_ty)
            {
                functions.push(InterfaceFn {
                    name: name.clone(),
                    params,
                    ret: return_ty.clone(),
                });
            }
        }
        Self {
            module: module.to_string(),
            prefix: prefix.to_string(),
            functions,
        }
    }

    /// The interface as its file holds it.
    pub(crate) fn to_text(&self) -> String {
        let mut out = format!("{MAGIC}\nmodule {}\nprefix {}\n", self.module, self.prefix);
        for function in &self.functions {
            let params: Vec<&str> = function
                .params
                .iter()
                .map(|ty| interface_ty(ty).expect("listed types have a spelling"))
                .collect();
            out.push_str(&format!(
                "fn {} {} -> {}\n",
                function.name,
                params.join(","),
                interface_ty(&function.ret).expect("listed types have a spelling")
            ));
        }
        out
    }

    /// Reads an interface file's text.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let bad = |line: &str| format!("malformed interface line `{line}`");
        let mut lines = text.lines();
        if lines.next() != Some(MAGIC) {
            return Err("not a pycc interface (or one of another version)".to_string());
        }
        let mut module = None;
        let mut prefix = None;
        let mut functions = Vec::new();
        for line in lines {
            if let Some(rest) = line.strip_prefix("module ") {
                module = Some(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("prefix ") {
                prefix = Some(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("fn ") {
                let (head, ret) = rest.split_once(" -> ").ok_or_else(|| bad(line))?;
                let (name, params) = head.split_once(' ').unwrap_or((head, ""));
                let params = params
                    .split(',')
                    .filter(|param| !param.is_empty())
                    .map(|param| parse_ty(param).ok_or_else(|| bad(line)))
                    .collect::<Result<Vec<_>, _>>()?;
                functions.push(InterfaceFn {
                    name: name.to_string(),
                    params,
                    ret: parse_ty(ret).ok_or_else(|| bad(line))?,
                });
            } else {
                return Err(bad(line));
            }
        }
        Ok(Self {
            module: module.ok_or("an interface without its module")?,
            prefix: prefix.ok_or("an interface without its prefix")?,
            functions,
        })
    }

    /// The producer's C header: the listed names and the table of the
    /// objects its dict binds to them once its body has run (each a strong
    /// reference, so no other object can take its address while a caller
    /// compares against it).
    pub(crate) fn c_header(&self) -> String {
        let count = self.functions.len();
        let mut out = String::from(
            "/* Generated by pycc: the functions other modules of the package call \
             natively. */\n",
        );
        out.push_str(&format!(
            "PYCC_ENTRY PyObject *pycc_ext_interface_table[{}];\n",
            count.max(1)
        ));
        out.push_str("static const char *const pycc_ext_interface_names[] = {");
        for function in &self.functions {
            out.push_str(&format!("\"{}\", ", function.name));
        }
        out.push_str("NULL};\n\n");
        out.push_str(
            "static void pycc_ext_interface_fill(PyObject *dict)\n{\n    \
             for (int i = 0; pycc_ext_interface_names[i] != NULL; i++) {\n        \
             PyObject *value = PyDict_GetItemString(dict, pycc_ext_interface_names[i]);\n        \
             PyObject *old = pycc_ext_interface_table[i];\n\n        \
             Py_XINCREF(value);\n        \
             pycc_ext_interface_table[i] = value;\n        \
             Py_XDECREF(old);\n    }\n}\n",
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Interface {
        Interface {
            module: "pkg.util".to_string(),
            prefix: "pycc_P3pkg4util_".to_string(),
            functions: vec![
                InterfaceFn {
                    name: "escape".to_string(),
                    params: vec![Ty::Str, Ty::Int],
                    ret: Ty::Str,
                },
                InterfaceFn {
                    name: "noop".to_string(),
                    params: vec![],
                    ret: Ty::None,
                },
            ],
        }
    }

    #[test]
    fn an_interface_round_trips_through_its_text() {
        let text = sample().to_text();
        assert_eq!(
            text,
            "pycc-interface 1\nmodule pkg.util\nprefix pycc_P3pkg4util_\n\
             fn escape str,int -> str\nfn noop  -> None\n"
        );
        assert_eq!(Interface::parse(&text).unwrap(), sample());
        assert_ne!(content_hash(&text), content_hash("pycc-interface 1\n"));
    }

    #[test]
    fn malformed_interfaces_are_refused() {
        assert!(Interface::parse("other\n").is_err());
        assert!(Interface::parse("pycc-interface 1\nprefix p\n").is_err());
        assert!(Interface::parse("pycc-interface 1\nmodule m\n").is_err());
        assert!(
            Interface::parse("pycc-interface 1\nmodule m\nprefix p\nfn f list -> str\n").is_err()
        );
        assert!(Interface::parse("pycc-interface 1\nmodule m\nprefix p\nfn f\n").is_err());
        assert!(Interface::parse("pycc-interface 1\nmodule m\nprefix p\nwhat\n").is_err());
        assert!(Interface::parse("pycc-interface 1\nmodule m\nprefix p\nfn f -> list\n").is_err());
    }

    #[test]
    fn only_scalar_and_object_signatures_are_listable() {
        assert!(InterfaceFn::listable(&[Ty::Int, Ty::Object], &Ty::None));
        assert!(!InterfaceFn::listable(&[Ty::None], &Ty::Int));
        assert!(!InterfaceFn::listable(
            &[Ty::List(Box::new(Ty::Int))],
            &Ty::Int
        ));
        assert!(!InterfaceFn::listable(&[], &Ty::List(Box::new(Ty::Int))));
    }

    #[test]
    fn the_c_header_fills_one_slot_per_function() {
        let header = sample().c_header();
        assert!(
            header.contains("PyObject *pycc_ext_interface_table[2];"),
            "{header}"
        );
        assert!(header.contains("{\"escape\", \"noop\", NULL};"), "{header}");
        let empty = Interface {
            functions: vec![],
            ..sample()
        };
        assert!(empty.c_header().contains("pycc_ext_interface_table[1];"));
    }
}
