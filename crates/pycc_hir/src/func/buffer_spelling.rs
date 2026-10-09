//! The bare-name buffer-carrier annotation spellings `ndarray` and
//! `NDArray` (#1129/#1134). Extracted from `func.rs`'s `annotation_to_ty`
//! (#1380) with no logic change, per AGENTS.md's file-decomposition rule.

use crate::Ty;

/// `Ty::MemoryView` for the bare annotation names `ndarray` and `NDArray`,
/// `None` for any other name.
///
/// #1129/#1134: `ndarray` and `NDArray` are *further spellings* of the same
/// pycc type, not new ones. All three mean "a one-dimensional, C-contiguous,
/// format `'d'` buffer exporter", which is exactly what
/// `pycc_ext_unpack_memoryview` enforces at the boundary, so the three
/// spellings have no run-time observable difference and a distinct `Ty`
/// variant would carry information no consumer could read. Diagnostics
/// therefore render the canonical `memoryview` for any spelling, which is
/// already what the alias table does for `type Arr = memoryview`. `NDArray`
/// (#1134) is the capitalized `numpy.typing` spelling 18 of the 19
/// array-parameter occurrences in the #1039 census use; it joins the set on
/// exactly the terms `ndarray` did. Since #1380 the census's own binding,
/// `from numpy.typing import NDArray` (and `from numpy import ndarray`), is
/// admitted as a buffer-carrier import: it runs in the host and binds a
/// hidden name, so the spelling still reaches this fallback
/// (`import::carrier`, D-244). `import numpy as np` is #883, and an
/// attribute-form base `np.ndarray` is #889.
///
/// Recognized with **no import**, deliberately: `annotation_to_ty` receives
/// `type_param`, `class_name`, `aliases` and `class_defs` and no import
/// table at all, and `import numpy` builds only from a `pycc.lock` closure
/// (#1242), so requiring one would be new machinery gating a spelling on an
/// installed numpy. `Any`, `Annotated`, `TypeAlias` and `Self` are all
/// recognized on those terms.
///
/// Both are resolved by `annotation_to_ty` *after* `class_defs` and the
/// alias table rather than beside `memoryview` in its keyword list, and that
/// placement is the rule rather than a detail: every name in that list is a
/// Python builtin or a `typing` name, while `ndarray` and `NDArray` are
/// ordinary identifiers a program may bind itself. Reserving one ahead of
/// `class_defs` and `aliases` would make a module-level `class ndarray` or
/// `type NDArray = ...` mean something Python does not -- in Python a local
/// definition shadows an imported name, not the other way round -- and
/// measurably refused programs that compiled before the spelling existed.
/// The user's own definition therefore wins, and the buffer carrier is what
/// a name nothing else binds falls back to. (A module that also writes the
/// carrier import may not define the spelling at all: `import::carrier`
/// refuses that, #1380.) That is why neither spelling appears in
/// `name_resolves_before_class_defs` nor, through it, in the subscript
/// arm's own `name_resolves_before_aliases` ladder.
pub(super) fn buffer_carrier_spelling(name: &str) -> Option<Ty> {
    matches!(name, "ndarray" | "NDArray").then_some(Ty::MemoryView)
}
