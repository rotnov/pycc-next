//! The boxed payloads of `MirExpr` variants (`MirExpr::Nd`,
//! `MirExpr::Instantiate`).

use crate::{MirExpr, Ty};

/// `MirExpr::Nd`'s payload.
#[derive(Debug, Clone, PartialEq)]
pub struct MirNdExpr {
    pub op: pycc_hir::ndarray::NdOp,
    pub base: MirExpr,
    pub args: Vec<MirExpr>,
    pub nd: pycc_hir::ndarray::NdTy,
    /// `pycc_hir::ndarray::NdExpr::cached`.
    pub cached: bool,
}

impl MirNdExpr {
    /// The type the operation evaluates to (`pycc_hir::ndarray::NdExpr::ty`).
    #[must_use]
    pub fn ty(&self) -> Ty {
        use pycc_hir::ndarray::NdOp;
        match self.op {
            NdOp::Get { .. } => self.nd.elem_ty(),
            NdOp::Set { .. } | NdOp::Check => Ty::None,
            NdOp::Shape => Ty::Tuple(Box::new(vec![Ty::Int; usize::from(self.nd.ndim)])),
            NdOp::Size | NdOp::Ndim | NdOp::Dim(_) => Ty::Int,
        }
    }
}

/// `MirExpr::Instantiate`'s payload, boxed (not inlined into that variant
/// directly) to keep `MirExpr`'s own size close to its other variants --
/// `ctor: String` + the slot count + `args: Vec<MirExpr>` + `ty: Ty`
/// inlined directly measured large enough to trip clippy's
/// `large_enum_variant` lint (`-D warnings`) on `MirItem` (whose
/// `TopLevelStmt(MirStmt)` variant embeds `MirExpr` several layers deep,
/// e.g. via `MirStmt::AttrSet`'s two raw `MirExpr` fields and
/// `CompSource::Range`'s three), the same reasoning `Ty::Tuple(Box<Vec<Ty>>)`
/// and `Ty::Dict(Box<(Ty, Ty)>)` already apply one crate over.
#[derive(Debug, Clone, PartialEq)]
pub struct InstantiateExpr {
    pub ctor: String,
    /// The class's source name -- a monomorphised generic class's original
    /// name, not its `0gen_` mangling -- which `pycc_rt` uses to word the
    /// `AttributeError` of a slot read before its assignment (#1388).
    pub class_name: String,
    /// The name of every slot of the class's flat MRO layout, in slot
    /// order (`class::mro_attrs`). Its length is the slot count the
    /// instance is allocated with (#432); the names word the same
    /// `AttributeError` (#1388).
    pub slot_names: Vec<String>,
    pub args: Vec<MirExpr>,
    pub ty: Ty,
}
