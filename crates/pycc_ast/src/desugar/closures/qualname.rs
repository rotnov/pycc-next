//! The name a lifted nested function gets, which carries its Python
//! `__qualname__` (`outer.<locals>.inner`) so that its function values and
//! generators render as CPython's do (`<function outer.<locals>.inner at
//! 0x...>`) without a side table.
//!
//! The lifted name is `_pycc_q` followed by the qualname with `_` written
//! `_u`, `.` written `_d` and `<locals>` written `_l`, then `_` and a fresh
//! number. The escapes never put a digit after a `_`, so the first `_`
//! followed by a digit starts the number, and [`function_qualname`]
//! reverses the encoding exactly.

/// The prefix of an encoded lifted name.
const PREFIX: &str = "_pycc_q";

/// The lifted name of a nested function whose `__qualname__` is `qualname`,
/// made unique by `fresh` (decimal digits).
pub(crate) fn lifted_name(qualname: &str, fresh: &str) -> String {
    format!("{PREFIX}{}_{fresh}", encode(qualname))
}

/// `qualname` as identifier characters (see the module documentation).
pub(crate) fn encode(qualname: &str) -> String {
    let mut out = String::new();
    let mut rest = qualname;
    while let Some(c) = rest.chars().next() {
        if let Some(after) = rest.strip_prefix("<locals>") {
            out.push_str("_l");
            rest = after;
            continue;
        }
        match c {
            '_' => out.push_str("_u"),
            '.' => out.push_str("_d"),
            c => out.push(c),
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Reverses [`encode`], stopping at a `_` followed by a digit; `None` when
/// `encoded` is not an encoding.
pub(crate) fn decode(encoded: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = encoded.chars();
    while let Some(c) = chars.next() {
        if c != '_' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('u') => out.push('_'),
            Some('d') => out.push('.'),
            Some('l') => out.push_str("<locals>"),
            // A fresh number follows: the qualname is complete.
            Some(d) if d.is_ascii_digit() => return Some(out),
            _ => return None,
        }
    }
    Some(out)
}

/// The Python `__qualname__` of the compiled function `symbol`: the decoded
/// qualname of a lifted nested function, else `symbol` itself (a
/// module-level function `f`, or a method `C.m`).
#[must_use]
pub fn function_qualname(symbol: &str) -> String {
    symbol
        .strip_prefix(PREFIX)
        .and_then(decode)
        .unwrap_or_else(|| symbol.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lifted_name_round_trips_its_qualname() {
        for qualname in [
            "f.<locals>.g",
            "make_adder.<locals>._inner_2",
            "C.method.<locals>.helper",
            "a.<locals>.b.<locals>.c",
        ] {
            let lifted = lifted_name(qualname, "17");
            assert!(
                lifted
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
            );
            assert_eq!(function_qualname(&lifted), qualname, "{lifted}");
        }
        assert_eq!(lifted_name("f.<locals>.g", "3"), "_pycc_qf_d_l_dg_3");
    }

    #[test]
    fn any_other_symbol_is_its_own_qualname() {
        assert_eq!(function_qualname("add"), "add");
        assert_eq!(function_qualname("Box.area"), "Box.area");
        assert_eq!(function_qualname("_pycc_qx_z"), "_pycc_qx_z");
        assert_eq!(decode("Box_darea").as_deref(), Some("Box.area"));
    }
}
