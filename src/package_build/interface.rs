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
//! Each class whose instances another module may read natively is listed
//! too ([`InterfaceClass`]): its slots, in the order its layout holds them
//! and with their types, and its compiled methods with their signatures
//! (the symbol is the producer's prefix and `pyfn_<Class.method>`). A
//! consumer declares a class of the same shape and calls those methods
//! (`crate::package_build::siblings`).
//!
//! The text is the contract: [`content_hash`] of it is recorded by every
//! consumer and checked by the package build before the link.

use pycc_hir::Ty;
use std::collections::{BTreeMap, BTreeSet};

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

/// One compiled method of an [`InterfaceClass`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InterfaceMethod {
    /// Its name in the class.
    pub(crate) name: String,
    /// The compiled function (`Class.method`).
    pub(crate) function: String,
    /// The parameters after `self`.
    pub(crate) params: Vec<Ty>,
    pub(crate) ret: Ty,
}

/// One class whose instances other modules may read natively.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InterfaceClass {
    pub(crate) name: String,
    /// The slots, in layout order.
    pub(crate) slots: Vec<(String, Ty)>,
    pub(crate) methods: Vec<InterfaceMethod>,
}

/// A member's interface.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Interface {
    /// The module's dotted name.
    pub(crate) module: String,
    /// Its symbol prefix (`pycc_codegen::member_prefix`).
    pub(crate) prefix: String,
    pub(crate) functions: Vec<InterfaceFn>,
    pub(crate) classes: Vec<InterfaceClass>,
}

/// The spelling of a slot or method type of an [`InterfaceClass`], or
/// `None` for one it does not carry: [`interface_ty`]'s, `list[T]`,
/// `set[T]`, `dict[K,V]`, `T|None`, and `@C` for an instance of a listed
/// class `C` (`listed`). No spelling holds a space.
pub(crate) fn class_ty(ty: &Ty, listed: &dyn Fn(&str) -> bool) -> Option<String> {
    Some(match ty {
        Ty::List(inner) => format!("list[{}]", class_ty(inner, listed)?),
        Ty::Set(inner) => format!("set[{}]", class_ty(inner, listed)?),
        Ty::Dict(pair) => format!(
            "dict[{},{}]",
            class_ty(&pair.0, listed)?,
            class_ty(&pair.1, listed)?
        ),
        Ty::Optional(inner) if !matches!(inner.as_ref(), Ty::None | Ty::Optional(_)) => {
            format!("{}|None", class_ty(inner, listed)?)
        }
        Ty::Instance(class) if listed(class) => format!("@{class}"),
        other => interface_ty(other)?.to_string(),
    })
}

/// Reads a [`class_ty`] spelling.
fn parse_class_ty(text: &str) -> Option<Ty> {
    if let Some(inner) = text.strip_suffix("|None") {
        return Some(Ty::Optional(Box::new(parse_class_ty(inner)?)));
    }
    if let Some(class) = text.strip_prefix('@') {
        return (!class.is_empty()).then(|| Ty::Instance(Box::new(class.to_string())));
    }
    let generic = |head: &str| {
        text.strip_prefix(head)
            .and_then(|rest| rest.strip_prefix('['))
            .and_then(|rest| rest.strip_suffix(']'))
    };
    if let Some(inner) = generic("list") {
        return Some(Ty::List(Box::new(parse_class_ty(inner)?)));
    }
    if let Some(inner) = generic("set") {
        return Some(Ty::Set(Box::new(parse_class_ty(inner)?)));
    }
    if let Some(inner) = generic("dict") {
        // The comma at bracket depth 0 separates the key from the value.
        let mut depth = 0usize;
        let split = inner.char_indices().find_map(|(at, c)| {
            match c {
                '[' => depth += 1,
                ']' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => return Some(at),
                _ => {}
            }
            None
        })?;
        return Some(Ty::Dict(Box::new((
            parse_class_ty(&inner[..split])?,
            parse_class_ty(&inner[split + 1..])?,
        ))));
    }
    parse_ty(text)
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

/// A compiled function's parameters, return type and body.
type Signature<'a> = (&'a [(String, Ty)], &'a Ty, &'a [pycc_mir::MirStmt]);

/// The classes of `mir` other modules may read natively: a class with a
/// carrier (`carriers`, so its instances cross into CPython as objects of
/// its own type) that has no base, no subclass, no type parameter and is
/// no enum, exception or protocol -- an instance of it typed as it is one
/// of it, of its layout -- whose slots all have a [`class_ty`] spelling.
/// Each lists its compiled methods (not the dunders) of listable
/// signatures, defined once (`counts`) and called with their parameters
/// alone.
fn collect_classes(
    mir: &pycc_mir::MirModule,
    carriers: &BTreeSet<String>,
    counts: &std::collections::HashMap<&str, usize>,
) -> Vec<InterfaceClass> {
    let subclassed: BTreeSet<&str> = mir
        .class_defs
        .iter()
        .flat_map(|(name, def)| def.mro.iter().filter(move |base| *base != name))
        .map(String::as_str)
        .collect();
    let mut listed: BTreeSet<String> = mir
        .class_defs
        .iter()
        .filter(|(name, def)| {
            carriers.contains(name)
                && def.bases.is_empty()
                && def.type_param.is_none()
                && !def.is_enum
                && !def.is_protocol
                && def.exception_type_tag.is_none()
                && !subclassed.contains(name.as_str())
        })
        .map(|(name, _)| name.clone())
        .collect();
    // A slot may hold an instance of another listed class only: drop the
    // classes with a slot no spelling covers until none is left.
    loop {
        let before = listed.len();
        let snapshot = listed.clone();
        let known = |class: &str| snapshot.contains(class);
        listed.retain(|name| {
            let def = &mir
                .class_defs
                .iter()
                .find(|(n, _)| n == name)
                .expect("a class")
                .1;
            def.attrs
                .iter()
                .all(|(_, ty)| class_ty(ty, &known).is_some())
        });
        if listed.len() == before {
            break;
        }
    }
    let functions: BTreeMap<&str, Signature<'_>> = mir
        .items
        .iter()
        .filter_map(|item| match item {
            pycc_mir::MirItem::Function {
                name,
                params,
                return_ty,
                body,
            } => Some((
                name.as_str(),
                (params.as_slice(), return_ty, body.as_slice()),
            )),
            _ => None,
        })
        .collect();
    let known = |class: &str| listed.contains(class);
    let mut classes = Vec::new();
    for (name, def) in &mir.class_defs {
        if !listed.contains(name) {
            continue;
        }
        let mut methods = Vec::new();
        for (method, function) in &def.methods {
            if method.starts_with("__") || counts.get(function.as_str()) != Some(&1) {
                continue;
            }
            let Some(&(params, ret, body)) = functions.get(function.as_str()) else {
                continue;
            };
            let Some(((_, receiver), rest)) = params.split_first() else {
                continue;
            };
            let spelled = |ty: &Ty| class_ty(ty, &known).is_some();
            if *receiver != Ty::Instance(Box::new(name.clone()))
                || !rest.iter().all(|(_, ty)| *ty != Ty::None && spelled(ty))
                || !spelled(ret)
                || !pycc_mir::multi_return::out_tys(body).is_empty()
                || pycc_codegen::body_returns_buffer_slice(body)
            {
                continue;
            }
            methods.push(InterfaceMethod {
                name: method.clone(),
                function: function.clone(),
                params: rest.iter().map(|(_, ty)| ty.clone()).collect(),
                ret: ret.clone(),
            });
        }
        classes.push(InterfaceClass {
            name: name.clone(),
            slots: def.attrs.clone(),
            methods,
        });
    }
    classes
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
        published: &BTreeSet<String>,
        carriers: &BTreeSet<String>,
    ) -> Self {
        let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for item in &mir.items {
            if let pycc_mir::MirItem::Function { name, .. } = item {
                *counts.entry(name.as_str()).or_insert(0) += 1;
            }
        }
        let classes = collect_classes(mir, carriers, &counts);
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
            classes,
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
        let listed = |_: &str| true;
        let spell = |ty: &Ty| class_ty(ty, &listed).expect("listed types have a spelling");
        for class in &self.classes {
            out.push_str(&format!("class {}\n", class.name));
            for (name, ty) in &class.slots {
                out.push_str(&format!("slot {name} {}\n", spell(ty)));
            }
            for method in &class.methods {
                out.push_str(&format!(
                    "method {} {} {}",
                    method.name,
                    method.function,
                    spell(&method.ret)
                ));
                for param in &method.params {
                    out.push(' ');
                    out.push_str(&spell(param));
                }
                out.push('\n');
            }
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
        let mut classes: Vec<InterfaceClass> = Vec::new();
        for line in lines {
            if let Some(name) = line.strip_prefix("class ") {
                classes.push(InterfaceClass {
                    name: name.to_string(),
                    slots: Vec::new(),
                    methods: Vec::new(),
                });
            } else if let Some(rest) = line.strip_prefix("slot ") {
                let class = classes.last_mut().ok_or_else(|| bad(line))?;
                let (name, ty) = rest.split_once(' ').ok_or_else(|| bad(line))?;
                let ty = parse_class_ty(ty).ok_or_else(|| bad(line))?;
                class.slots.push((name.to_string(), ty));
            } else if let Some(rest) = line.strip_prefix("method ") {
                let class = classes.last_mut().ok_or_else(|| bad(line))?;
                let mut words = rest.split(' ');
                let (Some(name), Some(function), Some(ret)) =
                    (words.next(), words.next(), words.next())
                else {
                    return Err(bad(line));
                };
                let params = words
                    .map(|param| parse_class_ty(param).ok_or_else(|| bad(line)))
                    .collect::<Result<Vec<_>, _>>()?;
                class.methods.push(InterfaceMethod {
                    name: name.to_string(),
                    function: function.to_string(),
                    params,
                    ret: parse_class_ty(ret).ok_or_else(|| bad(line))?,
                });
            } else if let Some(rest) = line.strip_prefix("module ") {
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
            classes,
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
            classes: vec![],
        }
    }

    fn with_class() -> Interface {
        Interface {
            classes: vec![InterfaceClass {
                name: "State".to_string(),
                slots: vec![
                    ("src".to_string(), Ty::Str),
                    (
                        "env".to_string(),
                        Ty::Dict(Box::new((Ty::Str, Ty::List(Box::new(Ty::Int))))),
                    ),
                    (
                        "parent".to_string(),
                        Ty::Optional(Box::new(Ty::Instance(Box::new("State".to_string())))),
                    ),
                    ("seen".to_string(), Ty::Set(Box::new(Ty::Object))),
                ],
                methods: vec![
                    InterfaceMethod {
                        name: "get".to_string(),
                        function: "State.get".to_string(),
                        params: vec![Ty::Int, Ty::Str],
                        ret: Ty::Optional(Box::new(Ty::Str)),
                    },
                    InterfaceMethod {
                        name: "clear".to_string(),
                        function: "State.clear".to_string(),
                        params: vec![],
                        ret: Ty::None,
                    },
                ],
            }],
            ..sample()
        }
    }

    #[test]
    fn a_class_round_trips_through_the_text() {
        let text = with_class().to_text();
        assert!(
            text.ends_with(
                "class State\nslot src str\nslot env dict[str,list[int]]\n\
                 slot parent @State|None\nslot seen set[object]\n\
                 method get State.get str|None int str\nmethod clear State.clear None\n"
            ),
            "{text}"
        );
        assert_eq!(Interface::parse(&text).unwrap(), with_class());
        for bad in [
            "slot x int\n",
            "class C\nslot x\n",
            "class C\nslot x list[\n",
            "class C\nmethod m\n",
            "class C\nmethod m C.m int frozenset[int]\n",
            "class C\nslot x dict[int]\n",
            "class C\nslot x @\n",
        ] {
            let text = format!("pycc-interface 1\nmodule m\nprefix p\n{bad}");
            assert!(Interface::parse(&text).is_err(), "{bad}");
        }
    }

    #[test]
    fn class_types_spell_only_what_a_class_carries() {
        let listed = |class: &str| class == "A";
        assert_eq!(
            class_ty(&Ty::Instance(Box::new("A".to_string())), &listed).as_deref(),
            Some("@A")
        );
        assert_eq!(
            class_ty(&Ty::Instance(Box::new("B".to_string())), &listed),
            None
        );
        assert_eq!(class_ty(&Ty::Optional(Box::new(Ty::None)), &listed), None);
        assert_eq!(class_ty(&Ty::Tuple(Box::new(vec![Ty::Int])), &listed), None);
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
