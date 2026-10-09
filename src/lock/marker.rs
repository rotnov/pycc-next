//! PEP 508 environment markers, parsed and evaluated fail-closed
//! (the pycc.lock decision entry, rule 2): anything this evaluator cannot
//! decide exactly refuses the lock instead of guessing.

use std::collections::BTreeMap;

/// The marker variables a lock probe reports, in probe order. `extra` is
/// not here: it is bound per evaluation.
pub(crate) const MARKER_VARIABLES: [&str; 11] = [
    "implementation_name",
    "implementation_version",
    "os_name",
    "platform_machine",
    "platform_python_implementation",
    "platform_release",
    "platform_system",
    "platform_version",
    "python_full_version",
    "python_version",
    "sys_platform",
];

/// The variables whose `==`/`!=` compare as versions, not strings.
const VERSION_VARIABLES: [&str; 3] = [
    "implementation_version",
    "python_full_version",
    "python_version",
];

/// The lock interpreter's marker environment: one value per
/// [`MARKER_VARIABLES`] entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MarkerEnv {
    values: BTreeMap<&'static str, String>,
}

impl MarkerEnv {
    /// Builds the environment from values in [`MARKER_VARIABLES`] order.
    pub(crate) fn new(values: [String; 11]) -> Self {
        Self {
            values: MARKER_VARIABLES.into_iter().zip(values).collect(),
        }
    }

    fn get(&self, name: &str) -> &str {
        self.values
            .get(name)
            .map(String::as_str)
            .expect("a parsed marker names only known variables")
    }
}

/// PEP 503 name normalization, which PEP 685 also applies to extras:
/// lowercase, with each run of `-`, `_` and `.` collapsed to one `-`.
pub(crate) fn normalize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut in_separator = false;
    for ch in name.chars() {
        if matches!(ch, '-' | '_' | '.') {
            if !in_separator {
                out.push('-');
            }
            in_separator = true;
        } else {
            out.extend(ch.to_lowercase());
            in_separator = false;
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Value {
    Variable(&'static str),
    Extra,
    Literal(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    Lt,
    Le,
    Eq,
    Ne,
    Ge,
    Gt,
    Compatible,
    In,
    NotIn,
}

/// A parsed marker expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Marker {
    Compare(Value, Op, Value),
    And(Box<Marker>, Box<Marker>),
    Or(Box<Marker>, Box<Marker>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Ident(String),
    Str(String),
    Op(String),
    Open,
    Close,
}

fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '(' {
            tokens.push(Token::Open);
            i += 1;
        } else if ch == ')' {
            tokens.push(Token::Close);
            i += 1;
        } else if ch == '\'' || ch == '"' {
            let end = chars[i + 1..]
                .iter()
                .position(|&c| c == ch)
                .ok_or_else(|| format!("unterminated string in marker `{text}`"))?;
            tokens.push(Token::Str(chars[i + 1..i + 1 + end].iter().collect()));
            i += end + 2;
        } else if matches!(ch, '<' | '>' | '=' | '!' | '~') {
            let start = i;
            while i < chars.len() && matches!(chars[i], '<' | '>' | '=' | '!' | '~') {
                i += 1;
            }
            tokens.push(Token::Op(chars[start..i].iter().collect()));
        } else if ch.is_ascii_alphanumeric() || ch == '_' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '_' | '.'))
            {
                i += 1;
            }
            tokens.push(Token::Ident(chars[start..i].iter().collect()));
        } else {
            return Err(format!("unexpected character `{ch}` in marker `{text}`"));
        }
    }
    Ok(tokens)
}

struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    text: &'a str,
}

impl Parser<'_> {
    fn error(&self, what: &str) -> String {
        format!("{what} in marker `{}`", self.text)
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        token
    }

    fn keyword(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Token::Ident(ident)) if ident == word)
    }

    fn or(&mut self) -> Result<Marker, String> {
        let mut left = self.and()?;
        while self.keyword("or") {
            self.pos += 1;
            left = Marker::Or(Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Marker, String> {
        let mut left = self.atom()?;
        while self.keyword("and") {
            self.pos += 1;
            left = Marker::And(Box::new(left), Box::new(self.atom()?));
        }
        Ok(left)
    }

    fn atom(&mut self) -> Result<Marker, String> {
        if self.peek() == Some(&Token::Open) {
            self.pos += 1;
            let inner = self.or()?;
            if self.next() != Some(Token::Close) {
                return Err(self.error("an unclosed `(`"));
            }
            return Ok(inner);
        }
        let left = self.value()?;
        let op = self.op()?;
        let right = self.value()?;
        Ok(Marker::Compare(left, op, right))
    }

    fn value(&mut self) -> Result<Value, String> {
        match self.next() {
            Some(Token::Str(text)) => Ok(Value::Literal(text)),
            Some(Token::Ident(name)) if name == "extra" => Ok(Value::Extra),
            Some(Token::Ident(name)) => MARKER_VARIABLES
                .iter()
                .find(|known| **known == name)
                .map(|known| Value::Variable(known))
                .ok_or_else(|| self.error(&format!("an unknown marker variable `{name}`"))),
            _ => Err(self.error("a missing marker value")),
        }
    }

    fn op(&mut self) -> Result<Op, String> {
        match self.next() {
            Some(Token::Op(op)) => match op.as_str() {
                "<" => Ok(Op::Lt),
                "<=" => Ok(Op::Le),
                "==" => Ok(Op::Eq),
                "!=" => Ok(Op::Ne),
                ">=" => Ok(Op::Ge),
                ">" => Ok(Op::Gt),
                "~=" => Ok(Op::Compatible),
                _ => Err(self.error(&format!("an unsupported operator `{op}`"))),
            },
            Some(Token::Ident(word)) if word == "in" => Ok(Op::In),
            Some(Token::Ident(word)) if word == "not" => match self.next() {
                Some(Token::Ident(word)) if word == "in" => Ok(Op::NotIn),
                _ => Err(self.error("`not` without `in`")),
            },
            _ => Err(self.error("a missing marker operator")),
        }
    }
}

/// Parses a marker, refusing anything outside the supported grammar.
pub(crate) fn parse_marker(text: &str) -> Result<Marker, String> {
    let mut parser = Parser {
        tokens: tokenize(text)?,
        pos: 0,
        text,
    };
    let marker = parser.or()?;
    if parser.pos != parser.tokens.len() {
        return Err(parser.error("trailing tokens"));
    }
    Ok(marker)
}

/// The release segments of a plain PEP 440 release (`3`, `3.8`, `3.8.10`),
/// or `None` for anything else: a pre-, post-, dev or local version, an
/// epoch, or a `.*` wildcard.
fn release(text: &str) -> Option<Vec<u64>> {
    text.split('.')
        .map(|part| {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                None
            } else {
                part.parse().ok()
            }
        })
        .collect()
}

fn cmp_release(left: &[u64], right: &[u64]) -> std::cmp::Ordering {
    let len = left.len().max(right.len());
    (0..len)
        .map(|i| {
            left.get(i)
                .copied()
                .unwrap_or(0)
                .cmp(&right.get(i).copied().unwrap_or(0))
        })
        .find(|ordering| ordering.is_ne())
        .unwrap_or(std::cmp::Ordering::Equal)
}

fn compare_versions(left: &str, op: Op, right: &str, text: &str) -> Result<bool, String> {
    let refuse = || {
        format!(
            "a version comparison `{left}` against `{right}` that is not between plain \
             release versions in marker `{text}`"
        )
    };
    let lhs = release(left).ok_or_else(refuse)?;
    let rhs = release(right).ok_or_else(refuse)?;
    let ordering = cmp_release(&lhs, &rhs);
    Ok(match op {
        Op::Lt => ordering.is_lt(),
        Op::Le => ordering.is_le(),
        Op::Eq => ordering.is_eq(),
        Op::Ne => ordering.is_ne(),
        Op::Ge => ordering.is_ge(),
        // Containment never reaches here; `compare` answers it first.
        Op::Gt | Op::In | Op::NotIn => ordering.is_gt(),
        Op::Compatible => {
            if rhs.len() < 2 {
                return Err(format!(
                    "`~=` needs at least two release segments, got `{right}`, in marker `{text}`"
                ));
            }
            let prefix = &rhs[..rhs.len() - 1];
            let head: Vec<u64> = (0..prefix.len())
                .map(|i| lhs.get(i).copied().unwrap_or(0))
                .collect();
            ordering.is_ge() && head == prefix
        }
    })
}

impl Marker {
    /// Evaluates the marker with `extra` bound to `extra` (already
    /// normalized, `""` for none).
    pub(crate) fn evaluate(
        &self,
        env: &MarkerEnv,
        extra: &str,
        text: &str,
    ) -> Result<bool, String> {
        match self {
            Marker::And(left, right) => {
                Ok(left.evaluate(env, extra, text)? && right.evaluate(env, extra, text)?)
            }
            Marker::Or(left, right) => {
                Ok(left.evaluate(env, extra, text)? || right.evaluate(env, extra, text)?)
            }
            Marker::Compare(left, op, right) => compare(env, extra, left, *op, right, text),
        }
    }
}

fn compare(
    env: &MarkerEnv,
    extra: &str,
    left: &Value,
    op: Op,
    right: &Value,
    text: &str,
) -> Result<bool, String> {
    let is_extra = matches!(left, Value::Extra) || matches!(right, Value::Extra);
    let resolve = |value: &Value| match value {
        Value::Variable(name) => env.get(name).to_string(),
        Value::Extra => extra.to_string(),
        Value::Literal(text) if is_extra => normalize_name(text),
        Value::Literal(text) => text.clone(),
    };
    let (lhs, rhs) = (resolve(left), resolve(right));
    if is_extra {
        return match op {
            Op::Eq => Ok(lhs == rhs),
            Op::Ne => Ok(lhs != rhs),
            _ => Err(format!(
                "an `extra` comparison other than `==`/`!=` in marker `{text}`"
            )),
        };
    }
    let is_version_variable =
        |value: &Value| matches!(value, Value::Variable(name) if VERSION_VARIABLES.contains(name));
    match op {
        Op::In => Ok(rhs.contains(&lhs)),
        Op::NotIn => Ok(!rhs.contains(&lhs)),
        Op::Eq | Op::Ne if !is_version_variable(left) && !is_version_variable(right) => {
            Ok((lhs == rhs) == (op == Op::Eq))
        }
        _ => compare_versions(&lhs, op, &rhs, text),
    }
}

#[cfg(test)]
#[path = "marker_tests.rs"]
pub(crate) mod tests;
