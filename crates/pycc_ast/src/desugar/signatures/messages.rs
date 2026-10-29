//! CPython 3.13's argument-binding `TypeError` messages (`ceval.c`'s
//! `initialize_locals`, `too_many_positional`, `missing_arguments`), for a
//! call the binder refuses at compile time, word for word (issue #10).

use super::Sig;

pub(super) fn multiple(qual: &str, name: &str) -> String {
    format!("{qual}() got multiple values for argument '{name}'")
}

/// An unknown keyword, with the "Did you mean" CPython 3.13 adds when one
/// of the parameters that may be passed by keyword (the positional-or-
/// keyword ones, a method's receiver among them, and the keyword-only ones)
/// is close enough to `name` ([`suggestion`]).
pub(super) fn unexpected(sig: &Sig, name: &str) -> String {
    let qual = &sig.qualname;
    let candidates: Vec<&str> = sig
        .regular
        .iter()
        .chain(&sig.kwonly)
        .map(|p| p.name.as_str())
        .collect();
    match suggestion(&candidates, name) {
        Some(close) => {
            format!("{qual}() got an unexpected keyword argument '{name}'. Did you mean '{close}'?")
        }
        None => format!("{qual}() got an unexpected keyword argument '{name}'"),
    }
}

pub(super) fn posonly_message(qual: &str, names: &str) -> String {
    format!("{qual}() got some positional-only arguments passed as keyword arguments: '{names}'")
}

/// CPython's `too_many_positional`.
pub(super) fn too_many(sig: &Sig, given: usize, kwonly_given: usize) -> String {
    let count = sig.positional_count();
    let defaults = sig.positional().filter(|p| p.default.is_some()).count();
    let (takes, plural) = if defaults > 0 {
        (format!("from {} to {count}", count - defaults), "s")
    } else {
        (count.to_string(), if count == 1 { "" } else { "s" })
    };
    let qual = &sig.qualname;
    if kwonly_given > 0 {
        format!(
            "{qual}() takes {takes} positional argument{plural} but {given} positional argument{} \
             (and {kwonly_given} keyword-only argument{}) were given",
            if given == 1 { "" } else { "s" },
            if kwonly_given == 1 { "" } else { "s" }
        )
    } else {
        format!(
            "{qual}() takes {takes} positional argument{plural} but {given} {} given",
            if given == 1 { "was" } else { "were" }
        )
    }
}

/// CPython's `missing_arguments`.
pub(super) fn missing_message(qual: &str, names: &[&str], kind: &str) -> String {
    let quoted: Vec<String> = names.iter().map(|n| format!("'{n}'")).collect();
    let list = match quoted.as_slice() {
        [one] => one.clone(),
        [a, b] => format!("{a} and {b}"),
        [init @ .., last] => format!("{}, and {last}", init.join(", ")),
        [] => String::new(),
    };
    format!(
        "{qual}() missing {} required {kind} argument{}: {list}",
        names.len(),
        if names.len() == 1 { "" } else { "s" }
    )
}

const MOVE_COST: usize = 2;
const CASE_COST: usize = 1;
const MAX_STRING_SIZE: usize = 40;
const MAX_CANDIDATE_ITEMS: usize = 750;

/// The candidate closest to `name`, as CPython 3.13's
/// `_Py_CalculateSuggestions` (`Python/suggestions.c`) picks it: a
/// Levenshtein distance over UTF-8 bytes, a case change costing half a
/// move, at most a third of the characters changed, the first of equally
/// close candidates.
pub(crate) fn suggestion<'a>(candidates: &[&'a str], name: &str) -> Option<&'a str> {
    if candidates.len() >= MAX_CANDIDATE_ITEMS {
        return None;
    }
    let mut best: Option<(&str, usize)> = None;
    for &item in candidates {
        if item == name {
            continue;
        }
        let max = (name.len() + item.len() + 3) * MOVE_COST / 6;
        let max = best.map_or(max, |(_, distance)| max.min(distance.saturating_sub(1)));
        let distance = levenshtein(name.as_bytes(), item.as_bytes(), max);
        if distance > max {
            continue;
        }
        if best.is_none_or(|(_, held)| distance < held) {
            best = Some((item, distance));
        }
    }
    best.map(|(item, _)| item)
}

fn substitution_cost(a: u8, b: u8) -> usize {
    if a & 31 != b & 31 {
        return MOVE_COST;
    }
    if a == b {
        0
    } else if a.eq_ignore_ascii_case(&b) {
        CASE_COST
    } else {
        MOVE_COST
    }
}

fn levenshtein<'a>(mut a: &'a [u8], mut b: &'a [u8], max_cost: usize) -> usize {
    while let ([x, ra @ ..], [y, rb @ ..]) = (a, b)
        && x == y
    {
        (a, b) = (ra, rb);
    }
    while let ([ra @ .., x], [rb @ .., y]) = (a, b)
        && x == y
    {
        (a, b) = (ra, rb);
    }
    if a.is_empty() || b.is_empty() {
        return (a.len() + b.len()) * MOVE_COST;
    }
    if a.len() > MAX_STRING_SIZE || b.len() > MAX_STRING_SIZE {
        return max_cost + 1;
    }
    if b.len() < a.len() {
        std::mem::swap(&mut a, &mut b);
    }
    if (b.len() - a.len()) * MOVE_COST > max_cost {
        return max_cost + 1;
    }
    let mut row: Vec<usize> = (1..=a.len()).map(|i| i * MOVE_COST).collect();
    let mut result = 0;
    for (b_index, &code) in b.iter().enumerate() {
        let mut distance = b_index * MOVE_COST;
        result = distance;
        let mut minimum = usize::MAX;
        for (index, &ch) in a.iter().enumerate() {
            let substitute = distance + substitution_cost(code, ch);
            distance = row[index];
            let insert_delete = result.min(distance) + MOVE_COST;
            result = insert_delete.min(substitute);
            row[index] = result;
            minimum = minimum.min(result);
        }
        if minimum > max_cost {
            return max_cost + 1;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::suggestion;

    /// Each answer is CPython 3.13's `_suggestions._generate_suggestions`.
    #[test]
    fn suggestions_match_cpython() {
        let cases: &[(&[&str], &str, Option<&str>)] = &[
            (&["self", "by"], "slf", Some("self")),
            (&["self", "by"], "b", Some("by")),
            (&["alpha", "beta", "gamma"], "gamm", Some("gamma")),
            (&["alpha", "beta"], "zz", None),
            (&["width", "height"], "heigh", Some("height")),
            (&["count", "key"], "countN", Some("count")),
            (&["count", "key"], "count2", Some("count")),
            (&["Value"], "value", Some("Value")),
            (&["x"], "x", None),
            (&[], "x", None),
            (&["ab", "ba"], "aa", Some("ab")),
        ];
        for (candidates, name, want) in cases {
            assert_eq!(suggestion(candidates, name), *want, "{candidates:?} {name}");
        }
    }
}
