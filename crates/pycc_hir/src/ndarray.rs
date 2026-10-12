//! Typed numpy arrays: `Ty::NdArray` (ROADMAP v0.2, accept item 2).
//!
//! A typed array is a numpy `ndarray` whose element type (`float64`, `int64`
//! or `bool`) and number of dimensions (1 or 2) are known at compile time.
//! At run time it is the `PyObject *` of the array itself -- the same
//! reference-counted pointer an opaque `object` is, so `pycc_mir` erases the
//! type to `Ty::Object` once the array operations are lowered, and every
//! ownership rule of an object applies unchanged. Element reads and writes,
//! `.shape`, `.size`, `.ndim`, `len()` and iteration are lowered natively
//! (`pycc_mir`'s `Nd*` nodes); everything else (`a.sum()`, `np.dot(a, b)`)
//! is an ordinary object operation through the CPython C API.
//!
//! Spellings, resolved by `func::annotation_to_ty`:
//!
//! * `NDArray[np.float64]` / `npt.NDArray[np.int64]` / `ndarray[np.bool_]`
//!   (any `X.float64`, `X.int64`, `X.bool_` dtype argument). The number of
//!   dimensions is not part of numpy's own spelling, so
//!   `pycc_ast::desugar`'s `ndarray_ndim` pass infers it from how the
//!   scope uses the name (`a[i, j]`, `a[i][j]`, `n, m = a.shape`) and writes
//!   it into the annotation as a second argument: `NDArray[np.float64, 2]`.
//! * `np.ndarray[tuple[int, int], np.dtype[np.float64]]`, numpy's own
//!   shape-typed spelling, whose `tuple` length is the number of dimensions.
//!
//! A bare `ndarray`/`NDArray` without a numpy dtype argument (or with a
//! builtin one, `NDArray[float]`) keeps meaning the `memoryview` buffer
//! carrier of a `pycc build --ext` parameter (`Ty::MemoryView`, #1129).

use pycc_ast::Expr;

mod lower;
mod walk;
pub use lower::lower_typed_arrays;
pub use walk::{expr_children, expr_children_mut};

/// A typed array operation, the payload of `HirExpr::Nd`.
#[derive(Debug, Clone, PartialEq)]
pub struct NdExpr {
    pub op: NdOp,
    /// The array: an object-typed expression (a name, an attribute).
    pub base: crate::HirExpr,
    /// The indices of `Get`; the indices and then the stored value of
    /// `Set`; empty otherwise.
    pub args: Vec<crate::HirExpr>,
    pub nd: NdTy,
    /// `base` is a function local every binding of which is followed by a
    /// `Check` (`lower`'s cache-safe locals). Code generation keeps such a
    /// local's header in registers between checks and re-checks it after
    /// any call that may run CPython code; every other base (a module
    /// global, an attribute, a call result, a local bound some unchecked
    /// way) is checked at each use, since CPython code can rebind it or
    /// change the array in place (`resize`, `__setstate__`) unseen.
    pub cached: bool,
}

impl NdExpr {
    /// The type the operation evaluates to.
    #[must_use]
    pub fn ty(&self) -> crate::Ty {
        match self.op {
            NdOp::Get { .. } => self.nd.elem_ty(),
            NdOp::Set { .. } | NdOp::Check => crate::Ty::None,
            NdOp::Shape => {
                crate::Ty::Tuple(Box::new(vec![crate::Ty::Int; usize::from(self.nd.ndim)]))
            }
            NdOp::Size | NdOp::Ndim | NdOp::Dim(_) => crate::Ty::Int,
        }
    }
}

/// What a `HirExpr::Nd` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NdOp {
    /// `a[i]`, `a[i, j]`; `chained` for `a[i][j]`, whose second index is
    /// reported against axis 0 when out of range (it indexes a row).
    Get { chained: bool },
    /// `a[i] = v`, `a[i, j] = v`, `a[i][j] = v` (`chained`).
    Set { chained: bool },
    /// `a.shape`: a tuple of `ndim` ints.
    Shape,
    /// `a.size`: the element count.
    Size,
    /// `a.ndim`.
    Ndim,
    /// `a.shape[k]` (and `len(a)`, which is `a.shape[0]`).
    Dim(u8),
    /// The run-time check that `base` is a `numpy.ndarray` of `nd`'s dtype
    /// and dimension count, raising `TypeError` otherwise. Evaluates to
    /// `None`; built as an expression statement after each binding of a
    /// typed array name, and before the body of a function with a typed
    /// array parameter.
    Check,
}

/// The element type of a typed array.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NdDtype {
    /// `numpy.float64`: a C `double`; elements read as `float`.
    F64,
    /// `numpy.int64`: a signed 64-bit integer; elements read as `int`.
    I64,
    /// `numpy.bool_`: one byte holding 0 or 1; elements read as `bool`.
    Bool,
}

impl NdDtype {
    /// numpy's name for the dtype (`float64`, `int64`, `bool`).
    #[must_use]
    pub fn numpy_name(self) -> &'static str {
        match self {
            NdDtype::F64 => "float64",
            NdDtype::I64 => "int64",
            NdDtype::Bool => "bool",
        }
    }

    /// The code the run-time check (`pycc_ext_nd_check`) takes.
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            NdDtype::F64 => 0,
            NdDtype::I64 => 1,
            NdDtype::Bool => 2,
        }
    }

    /// The size of one element in bytes.
    #[must_use]
    pub fn itemsize(self) -> u64 {
        match self {
            NdDtype::F64 | NdDtype::I64 => 8,
            NdDtype::Bool => 1,
        }
    }

    /// The dtype a numpy scalar-type attribute names (`float64`, `int64`,
    /// `bool_`, and numpy's aliases of them), if any.
    #[must_use]
    pub fn from_attr(attr: &str) -> Option<NdDtype> {
        match attr {
            "float64" | "double" | "float_" => Some(NdDtype::F64),
            "int64" | "int_" | "intp" => Some(NdDtype::I64),
            "bool_" | "bool" => Some(NdDtype::Bool),
            _ => None,
        }
    }
}

/// A typed array's static shape: dtype and number of dimensions.
///
/// `ndim` is 1 or 2 once lowered; 0 never leaves `annotation_to_ty`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NdTy {
    pub dtype: NdDtype,
    pub ndim: u8,
}

impl NdTy {
    /// The element type a scalar read of this array yields.
    #[must_use]
    pub fn elem_ty(self) -> crate::Ty {
        match self.dtype {
            NdDtype::F64 => crate::Ty::Float,
            NdDtype::I64 => crate::Ty::Int,
            NdDtype::Bool => crate::Ty::Bool,
        }
    }

    /// The type of `a[i]`: an element for a 1-D array, a row for a 2-D one.
    #[must_use]
    pub fn item_ty(self) -> crate::Ty {
        if self.ndim == 1 {
            self.elem_ty()
        } else {
            crate::Ty::NdArray(NdTy {
                dtype: self.dtype,
                ndim: self.ndim - 1,
            })
        }
    }

    /// How diagnostics render the type: `ndarray[float64, 2-D]`.
    #[must_use]
    pub fn name(self) -> String {
        format!("ndarray[{}, {}-D]", self.dtype.numpy_name(), self.ndim)
    }
}

thread_local! {
    /// The typed array attributes of the program last lowered, by
    /// `(class, attribute)` (`lower::lower_typed_arrays` records them).
    static TYPED_SLOTS: std::cell::RefCell<std::collections::HashMap<(String, String), NdTy>> =
        std::cell::RefCell::default();
}

fn record_typed_slots(slots: &std::collections::HashMap<(String, String), NdTy>) {
    TYPED_SLOTS.with(|cell| *cell.borrow_mut() = slots.clone());
}

/// The typed array type of attribute `attr` of `class` in the program this
/// thread last lowered, if it is one. Erased to `object` in the HIR, an
/// array attribute is still checked where CPython code can store into it:
/// an `--ext` class's attribute setter (`src/ext_build/getset/setter.rs`).
#[must_use]
pub fn typed_array_slot(class: &str, attr: &str) -> Option<NdTy> {
    TYPED_SLOTS.with(|cell| cell.borrow().get(&(class.to_string(), attr.to_string())).copied())
}

/// The largest number of dimensions a typed array may have.
pub const MAX_NDIM: u8 = 2;

/// The dtype a dtype-argument expression names: `np.float64`,
/// `numpy.int64`, `np.dtype[np.bool_]` (any attribute base), or `None`.
#[must_use]
pub fn dtype_of_expr(expr: &Expr) -> Option<NdDtype> {
    match expr {
        Expr::Attribute(attr)
            if matches!(attr.value.as_ref(), Expr::Name(_) | Expr::Attribute(_)) =>
        {
            NdDtype::from_attr(attr.attr.as_str())
        }
        Expr::Subscript(sub) if is_dtype_wrapper(&sub.value) => dtype_of_expr(&sub.slice),
        _ => None,
    }
}

/// `np.dtype` / `numpy.dtype` / a bare `dtype`.
fn is_dtype_wrapper(expr: &Expr) -> bool {
    match expr {
        Expr::Attribute(attr) => attr.attr.as_str() == "dtype",
        Expr::Name(name) => name.id.as_str() == "dtype",
        _ => false,
    }
}

/// The numpy shape-typed annotation `ndarray[tuple[int, int], dtype[...]]`
/// arguments: the dimension count of the `tuple[...]` (`None` unless every
/// element is `int`).
fn shape_tuple_ndim(expr: &Expr) -> Option<u8> {
    let Expr::Subscript(sub) = expr else {
        return None;
    };
    let Expr::Name(name) = sub.value.as_ref() else {
        return None;
    };
    if name.id.as_str() != "tuple" {
        return None;
    }
    let elts: Vec<&Expr> = match sub.slice.as_ref() {
        Expr::Tuple(tuple) => tuple.elts.iter().collect(),
        other => vec![other],
    };
    if elts
        .iter()
        .all(|e| matches!(e, Expr::Name(n) if n.id.as_str() == "int"))
    {
        u8::try_from(elts.len()).ok()
    } else {
        None
    }
}

/// What a subscripted annotation's arguments say about a typed array:
/// `(dtype, ndim)` with `ndim == 0` when the annotation leaves it to
/// inference, or `None` when the arguments are not a typed-array spelling.
///
/// Accepts `[dtype]`, `[dtype, N]` (the form `desugar` writes) and numpy's
/// `[tuple[int, ...], dtype[dtype]]`.
#[must_use]
pub fn typed_array_args(slice: &Expr) -> Option<(NdDtype, u8)> {
    match slice {
        Expr::Tuple(tuple) if tuple.elts.len() == 2 => {
            if let (Some(dtype), Expr::NumberLiteral(n)) =
                (dtype_of_expr(&tuple.elts[0]), &tuple.elts[1])
            {
                let pycc_ast::Number::Int(int) = &n.value else {
                    return None;
                };
                let ndim = u8::try_from(int.as_i64()?).ok()?;
                return Some((dtype, ndim));
            }
            let ndim = shape_tuple_ndim(&tuple.elts[0])?;
            let dtype = dtype_of_expr(&tuple.elts[1])?;
            Some((dtype, ndim))
        }
        other => dtype_of_expr(other).map(|dtype| (dtype, 0)),
    }
}

/// Whether `base` is an attribute-form typed-array base: `np.ndarray`,
/// `npt.NDArray`, `numpy.typing.NDArray`.
#[must_use]
pub fn is_attribute_array_base(base: &Expr) -> bool {
    matches!(base, Expr::Attribute(attr) if matches!(attr.attr.as_str(), "ndarray" | "NDArray"))
}

/// Whether `base` is a bare typed-array base name.
#[must_use]
pub fn is_bare_array_base(base: &Expr) -> bool {
    matches!(base, Expr::Name(name) if matches!(name.id.as_str(), "ndarray" | "NDArray"))
}

/// A typed numpy array annotation (`crate::ndarray`): `Some(Ty::NdArray)`
/// for `NDArray[np.float64]`, `npt.NDArray[np.int64, 2]`,
/// `np.ndarray[tuple[int, int], np.dtype[np.float64]]`; `None` for every
/// other subscript, including a bare `ndarray`/`NDArray` base a program's
/// own class or alias shadows and the buffer carrier's `NDArray[float]`.
pub(crate) fn typed_array_annotation(
    sub: &pycc_ast::ExprSubscript,
    aliases: &[(String, crate::Ty)],
    class_defs: &[crate::class::ClassAnnotationInfo],
) -> Result<Option<crate::Ty>, pycc_diag::Diagnostic> {
    let base = sub.value.as_ref();
    let eligible = is_attribute_array_base(base)
        || (is_bare_array_base(base) && {
            let Expr::Name(name) = base else {
                unreachable!()
            };
            let id = name.id.as_str();
            !class_defs.iter().any(|info| info.name == id)
                && !aliases.iter().any(|(alias, _)| alias == id)
        });
    if !eligible {
        return Ok(None);
    }
    let Some((dtype, ndim)) = typed_array_args(&sub.slice) else {
        if is_attribute_array_base(base) {
            return Err(crate::unsupported(
                "a typed numpy array annotation names its dtype: \
                 `NDArray[np.float64]`, `NDArray[np.int64]` or `NDArray[np.bool_]`",
                pycc_ast::expr_range(&sub.slice),
            ));
        }
        return Ok(None);
    };
    // `desugar`'s `ndarray_ndim` pass writes the inferred dimension count
    // into every annotation it can see; one it cannot (a class attribute
    // nothing indexes) is one-dimensional.
    let ndim = if ndim == 0 { 1 } else { ndim };
    if ndim > MAX_NDIM {
        return Err(crate::unsupported(
            format!("typed numpy arrays have 1 or 2 dimensions, not {ndim}"),
            pycc_ast::expr_range(&sub.slice),
        ));
    }
    Ok(Some(crate::Ty::NdArray(NdTy { dtype, ndim })))
}
