//! Unit tests for the `--ext` build seam.
//!
//! Every test here is non-`#[ignore]`d and needs no CPython installation:
//! `.github/workflows/ci.yml`'s coverage job runs `llvm-cov` without
//! `--include-ignored`, so an ignored test earns zero coverage while its
//! lines stay in `scripts/check_diff_coverage.py`'s denominator. The
//! CPython-driven behaviour of a *built* artifact is verified separately by
//! the ignored oracle tests under `tests/`.
//!
//! No test here asserts a rendered `Path`: `Display`/`to_string_lossy`
//! carries the building host's separator, so `dist/m` renders as `dist\m` on
//! Windows. Paths are compared as `PathBuf`s built with `Path::join`.
//!
//! For the same reason, a test that matches a *multi-line* fragment of
//! [`SHIM_C`] goes through [`shim_c`] rather than reading the constant
//! directly. A single-line match needs no normalization and uses `SHIM_C`.

use super::*;
use pycc_hir::Ty;

/// [`SHIM_C`] with its line endings normalized to `\n`.
///
/// `include_str!` embeds the shim exactly as the checkout holds it, and a
/// Windows checkout holds it with CRLF endings, so an assertion spanning a
/// line break matches on four Tier-1 targets and fails on the fifth. The
/// shipped artifact is deliberately left alone -- a C compiler does not care
/// which ending the source carries, and rewriting it here would make the
/// tests describe something other than what `--ext` writes out.
fn shim_c() -> String {
    SHIM_C.replace("\r\n", "\n")
}

fn func(name: &str, params: &[(&str, Ty)], return_ty: Ty) -> HirItem {
    func_with_body(name, params, return_ty, Vec::new())
}

/// [`func`], with a body -- the Part-1-of-#1142 writability walk is the
/// one collector input that reads a function's statements rather than only
/// its signature.
fn func_with_body(
    name: &str,
    params: &[(&str, Ty)],
    return_ty: Ty,
    body: Vec<pycc_hir::HirStmt>,
) -> HirItem {
    HirItem::Function {
        name: name.to_string(),
        params: params
            .iter()
            .map(|(n, ty)| ((*n).to_string(), ty.clone()))
            .collect(),
        return_ty,
        body,
    }
}

/// `<name>[0] = 1.0` as a statement, the shape `body_stores_into` answers.
fn element_store(name: &str) -> pycc_hir::HirStmt {
    pycc_hir::HirStmt::DictSet {
        dict: name.to_string(),
        key: pycc_hir::HirExpr::IntLiteral(0),
        value: pycc_hir::HirExpr::FloatLiteral(1.0),
    }
}

fn module(items: Vec<HirItem>) -> HirModule {
    HirModule {
        items,
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
        seeded_builtin_exception_classes: false,
    }
}

/// A minimal [`HirClassDef`] carrying only the two fields the `--ext` export
/// set reads: the class name and `exception_type_tag`.
///
/// #1143 excludes a user exception class by that tag, deliberately rather
/// than by `collect_user_exception_classes`' own selector: the tag is the one
/// field that is set for exactly the classes the runtime treats as
/// exceptions, so a selector drift elsewhere cannot silently widen the export
/// set. Every other field is empty here because nothing in `collect_exports`
/// consults it.
fn class_def(name: &str, exception_type_tag: Option<u8>) -> pycc_hir::HirClassDef {
    pycc_hir::HirClassDef {
        name: name.to_string(),
        bases: Vec::new(),
        mro: vec![name.to_string()],
        attrs: Vec::new(),
        methods: Vec::new(),
        properties: Vec::new(),
        static_methods: Vec::new(),
        class_methods: Vec::new(),
        type_param: None,
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: Vec::new(),
        class_attrs: Vec::new(),
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
        exception_type_tag,
    }
}

/// [`class_def`] made **constructible** under #1145: two `int` attribute
/// slots and an MRO-resolved `__init__` the caller supplies as an
/// [`HirItem::Function`] named `<name>.__init__`.
///
/// The `__init__` entry has to sit in `methods` rather than anywhere else:
/// `pycc_hir::class` mangles a constructor exactly like any other regular
/// method, and `resolved_init` walks `mro` against `class_defs` looking for
/// that one binding. `attrs` is what `flat_attr_layout` counts, so the two
/// entries here are the slot count the generated `tp_init` passes to
/// `pycc_rt_instance_new`.
fn constructible_class_def(name: &str) -> pycc_hir::HirClassDef {
    pycc_hir::HirClassDef {
        attrs: vec![("w".to_string(), Ty::Int), ("h".to_string(), Ty::Int)],
        methods: vec![("__init__".to_string(), format!("{name}.__init__"))],
        ..class_def(name, None)
    }
}

/// The [`HirItem::Function`] a [`constructible_class_def`] resolves to, with
/// the leading `self` every compiled method carries.
fn init_func(class: &str, params: &[(&str, Ty)], return_ty: Ty) -> HirItem {
    let mut all = vec![("self", Ty::Instance(Box::new(class.to_string())))];
    all.extend(params.iter().map(|(n, ty)| (*n, ty.clone())));
    func(
        &format!("{class}.__init__"),
        &all.iter()
            .map(|(n, ty)| (*n, ty.clone()))
            .collect::<Vec<_>>(),
        return_ty,
    )
}

/// [`module`] plus the class table `collect_exports` reads for the
/// exception-class exclusion.
fn module_with_classes(
    items: Vec<HirItem>,
    class_defs: Vec<(String, pycc_hir::HirClassDef)>,
) -> HirModule {
    HirModule {
        class_defs,
        ..module(items)
    }
}

fn probe(version: (u32, u32), include: &Path) -> ExtProbe {
    ExtProbe {
        version,
        include: include.to_path_buf(),
        libs: include.join("libs"),
    }
}

mod bridge_watermark;
mod buffer_slice_parity;
mod carrier_getsets;
mod carrier_types;
mod compiled_isinstance;
mod exports;
mod foreign_class_boundary;
mod generated_c;
mod getset;
mod init_only_publication;
mod instance_boundary;
mod instance_copy;
mod method_defaults;
mod object_text;
mod publication_order;
mod refusal_completeness;
mod toolchain;
