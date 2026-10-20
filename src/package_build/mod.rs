//! The whole-package build (v0.8, ROADMAP accepts 1 and 4): `pycc build
//! PKG --ext` compiles every module of the package directory `PKG` into one
//! extension, `PKG/__init__.abi3.so` (`.pyd` on Windows).
//!
//! Each module is compiled as it is compiled on its own -- same frontend,
//! same codegen, its own C unit -- by a `pycc build` process of its own (a
//! *member* build, [`Member`]), in parallel; the package build then compiles
//! the shim's runtime unit and the package unit (`src/ext/package/finder.c`,
//! whose finder imports each member as a module of its own) once, and links
//! everything into one shared object with one copy of the runtime. What
//! keeps the members apart in one object: every symbol a member's object or
//! C unit defines carries the member's prefix
//! (`pycc_codegen::member_prefix`), the C unit's own definitions are
//! `static`, the shim's per-module state is each member's own record
//! (`PyccExtModule`), and each member's user exception classes get tags no
//! other member uses ([`exception_tag_bases`]).
//!
//! `docs/CLI_SPEC.md` documents the command; `docs/DEVIATIONS.md` what
//! differs from separate extensions.

mod discover;
mod exceptions;
mod graph;
mod interface;
mod siblings;
mod unit;

pub(crate) use discover::{PackageModule, discover};
pub(crate) use exceptions::exception_tag_bases;
pub(crate) use graph::producers;
pub(crate) use interface::{INTERFACE_HEADER_NAME, INTERFACE_NAME, Interface, content_hash};
pub(crate) use siblings::{
    Producer, apply_siblings, codegen_targets, deny_failures as deny_sibling_failures,
    report as report_siblings,
};
pub(crate) use unit::{MEMBERS_HEADER_NAME, member_header, members_header};

use std::sync::OnceLock;

/// The C source of the package unit, compiled once per package build.
pub(crate) const FINDER_C: &str = include_str!("../ext/package/finder.c");

/// The member a `pycc build --package-member` process compiles: one module
/// of a package built whole, which ends in objects for the package build to
/// link rather than in an extension of its own.
#[derive(Debug)]
pub(crate) struct Member {
    /// The module's dotted name (`pkg.sub.mod`): its `__name__`, the name
    /// its classes' `__module__` reads, and what its symbol prefix is made
    /// from.
    pub(crate) dotted: String,
    /// `pycc_codegen::member_prefix(dotted)`.
    pub(crate) prefix: String,
    /// Whether the module is a package's `__init__`.
    pub(crate) is_package: bool,
    /// The other modules of the package this one may call natively: their
    /// interfaces, read when the build starts (`--sibling-interface`).
    pub(crate) producers: Vec<Producer>,
}

static MEMBER: OnceLock<Member> = OnceLock::new();

/// Makes this process a member build of `dotted`, whose source is `path`
/// (`pycc build --package-member`; its first exception tag is set on
/// `pycc_hir`), reading the interfaces `siblings` names
/// (`DOTTED=PATH`). Set once, before the build runs; `Err` names an
/// interface that cannot be read.
pub(crate) fn set_member(
    dotted: &str,
    path: &std::path::Path,
    siblings: &[String],
) -> Result<(), String> {
    let producers = siblings
        .iter()
        .map(|entry| Producer::read(entry))
        .collect::<Result<Vec<_>, _>>()?;
    let _ = MEMBER.set(Member {
        dotted: dotted.to_string(),
        prefix: pycc_codegen::member_prefix(dotted),
        is_package: path.file_name().is_some_and(|name| name == "__init__.py"),
        producers,
    });
    Ok(())
}

/// The member this process builds, if it is a member build.
pub(crate) fn member() -> Option<&'static Member> {
    MEMBER.get()
}

/// The file a member build writes into its output directory: the objects
/// to link, one per line, then `max_tag N`, the highest exception tag its
/// classes took (0 when none), which the package build checks against the
/// next member's base.
pub(crate) const MEMBER_MANIFEST_NAME: &str = "member.txt";
