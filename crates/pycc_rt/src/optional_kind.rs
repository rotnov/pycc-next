//! Container words of a `T | None` element (`list[int | None]`, a dict
//! value or tuple item that may be `None`): the word of a `T`, or `0` for
//! `None`. `0` is no `int` word (D-141) and no pointer, so it marks `None`
//! for an `int` and for every pointer kind; the element kind is the `T`'s
//! with [`OPTIONAL_KIND`] set. Every generic word operation
//! (`crate::value`) treats a `0` word of such a kind as `None` and any other
//! as its `T`.

/// Set in a `LIST_KIND_*` for a `T | None` element (`pycc_codegen`'s
/// `containers::word_kind` sets it).
pub const OPTIONAL_KIND: i64 = 32;

/// The `T`'s kind of a `T | None` element kind, or `None` for any other
/// kind.
pub(crate) fn inner(kind: i64) -> Option<i64> {
    (kind & OPTIONAL_KIND != 0).then_some(kind & !OPTIONAL_KIND)
}

/// CPython 3.13's `hash(None)`: the constant `Py_None` hashes to.
pub(crate) const NONE_HASH: i64 = 0xFCA8_6420;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_marks_an_optional_kind() {
        assert_eq!(inner(OPTIONAL_KIND), Some(crate::list::LIST_KIND_INT));
        assert_eq!(inner(OPTIONAL_KIND | crate::list::LIST_KIND_STR), Some(crate::list::LIST_KIND_STR));
        assert_eq!(inner(crate::list::LIST_KIND_STR), None);
    }
}
