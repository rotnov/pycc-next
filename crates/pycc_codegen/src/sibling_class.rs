//! Native instances of another module's classes in a package built whole
//! (v0.8).
//!
//! A module of the package reads the instances of a class another module
//! compiled through a *sibling class* the driver adds to it
//! (`_pycc_sibc_<Class>`, `pycc_hir::SIBLING_CLASS_PREFIX`): the same
//! slots, in the same order and of the same types, so the same layout, and
//! the same method signatures. Codegen never compiles the sibling class's
//! methods: a call of one calls the other module's compiled method
//! (`<prefix>pyfn_<Class.method>`), and every operation that needs the
//! class's CPython type -- reading a CPython object as an instance of it
//! (`isinstance`, a parameter's conversion) -- goes to the other module's
//! entry under the class's own name ([`route`]), since the other module
//! owns the carrier type. An instance packed into a CPython object finds
//! its type through the package's registry of carrier types, by the class
//! name its layout descriptor holds.

use std::cell::RefCell;

/// One class another module of the package compiled, read natively.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiblingClass {
    /// The sibling class (`_pycc_sibc_<Class>`).
    pub class: String,
    /// The class it stands for, as the other module names it.
    pub real: String,
    /// The other module's symbol prefix.
    pub prefix: String,
    /// Each method of the sibling class (`_pycc_sibc_<Class>.m`) with the
    /// other module's compiled function it calls (`<prefix>pyfn_<Class.m>`).
    pub methods: Vec<(String, String)>,
    /// v0.10: each method's entry there -- its dispatcher, which runs the
    /// overrides of subclasses wherever they were compiled -- which a call
    /// on a value typed with the sibling class runs (`<method>.0entry`,
    /// `pycc_mir::dispatch::ENTRY_SEGMENT`).
    pub entries_of: Vec<(String, String)>,
    /// v0.10: the class record of the class it stands for
    /// (`<prefix>pycc_class_<Class>`), the base of a subclass compiled
    /// over it.
    pub record: String,
    /// v0.10: the length of that class's method table, and each listed
    /// method's index in it with the sibling class's method and the other
    /// module's body (`<prefix>pyfn_<Class.m>`): the vtable a subclass
    /// compiled over it fills (`crate::class_record`).
    pub entries: usize,
    pub vtable: Vec<(usize, String, String)>,
}

thread_local! {
    static CLASSES: RefCell<Vec<SiblingClass>> = const { RefCell::new(Vec::new()) };
}

/// Sets the sibling classes of the module being compiled on this thread.
pub(crate) fn set(classes: &[SiblingClass]) {
    CLASSES.with(|cell| *cell.borrow_mut() = classes.to_vec());
}

/// The shim entry `symbol` (one of `member_symbols::MODULE_UNIT_ENTRIES`)
/// and the class name an operation on instances of `class` passes it: the
/// other module's entry and the class's own name for a sibling class,
/// `symbol` and `class` otherwise.
pub(crate) fn route(symbol: &str, class: &str) -> (String, String) {
    CLASSES.with(|cell| {
        cell.borrow()
            .iter()
            .find(|sibling| sibling.class == class)
            .map_or_else(
                || (symbol.to_string(), class.to_string()),
                |sibling| (format!("{}{symbol}", sibling.prefix), sibling.real.clone()),
            )
    })
}

/// The sibling class `class`, if it is one.
pub(crate) fn get(class: &str) -> Option<SiblingClass> {
    CLASSES.with(|cell| cell.borrow().iter().find(|sibling| sibling.class == class).cloned())
}

/// v0.10: each sibling class method's entry (`<method>.0entry`), with the
/// method and the other module's function a call of it runs on a value
/// typed with the sibling class.
pub(crate) fn producer_entries() -> Vec<(String, String, String)> {
    CLASSES.with(|cell| {
        cell.borrow()
            .iter()
            .flat_map(|sibling| {
                sibling.entries_of.iter().map(|(method, entry)| {
                    (
                        format!("{method}.{}", pycc_mir::dispatch::ENTRY_SEGMENT),
                        method.clone(),
                        entry.clone(),
                    )
                })
            })
            .collect()
    })
}

/// The other module's compiled function a sibling class's method `name`
/// (`_pycc_sibc_<Class>.m`) calls, if `name` is one.
pub(crate) fn producer_method(name: &str) -> Option<String> {
    if !name.starts_with(pycc_hir::SIBLING_CLASS_PREFIX) {
        return None;
    }
    CLASSES.with(|cell| {
        cell.borrow().iter().find_map(|sibling| {
            sibling
                .methods
                .iter()
                .find(|(method, _)| method == name)
                .map(|(_, producer)| producer.clone())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sibling_classes_route_to_the_other_module() {
        set(&[SiblingClass {
            class: "_pycc_sibc_S".to_string(),
            real: "S".to_string(),
            prefix: "pycc_P1p1c_".to_string(),
            methods: vec![(
                "_pycc_sibc_S.get".to_string(),
                "pycc_P1p1c_pyfn_S.get".to_string(),
            )],
            entries_of: vec![],
            record: "pycc_P1p1c_pycc_class_S".to_string(),
            entries: 1,
            vtable: vec![],
        }]);
        assert_eq!(
            route("pycc_ext_obj_unbox_instance", "_pycc_sibc_S"),
            (
                "pycc_P1p1c_pycc_ext_obj_unbox_instance".to_string(),
                "S".to_string()
            )
        );
        assert_eq!(
            route("pycc_ext_obj_unbox_instance", "Own"),
            ("pycc_ext_obj_unbox_instance".to_string(), "Own".to_string())
        );
        assert_eq!(
            producer_method("_pycc_sibc_S.get").as_deref(),
            Some("pycc_P1p1c_pyfn_S.get")
        );
        assert_eq!(producer_method("_pycc_sibc_S.__init__"), None);
        assert_eq!(producer_method("Own.get"), None);
        set(&[]);
    }
}
