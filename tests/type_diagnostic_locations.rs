//! #10: type-checker diagnostics point at the offending code. Each program
//! below has one error; the test checks it with `pycc check --error-format
//! json` and asserts that the reported line, column and length are those of
//! the marked expression or statement, and that the caret's label is a
//! short phrase rather than the headline again.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

/// One erroneous program: the code it must report, and where -- the
/// `occurrence`-th (0-based) appearance of `at` in `source`.
struct Case {
    code: &'static str,
    source: &'static str,
    at: &'static str,
    occurrence: usize,
    label: &'static str,
}

const fn case(code: &'static str, source: &'static str, at: &'static str) -> Case {
    Case {
        code,
        source,
        at,
        occurrence: 0,
        label: "",
    }
}

const CASES: &[Case] = &[
    Case {
        label: "expected `int`, found `str`",
        ..case(
            "T0021",
            "def fib(n: int) -> int:\n    return n\n\nprint(fib(\"5\"))\n",
            "\"5\"",
        )
    },
    Case {
        label: "expected 2 argument(s)",
        ..case(
            "T0021",
            "def add(a: int, b: int) -> int:\n    return a + b\n\ndef main() -> None:\n    print(add(1))\n",
            "add(1)",
        )
    },
    Case {
        label: "used before assignment",
        ..case(
            "T0021",
            "x = 1\n\ndef f() -> int:\n    print(x)\n    x = 2\n    return x\n",
            "x",
            ) .with_occurrence(1)
    },
    Case {
        label: "not defined",
        ..case("T0021", "def f() -> int:\n    return g(1)\n", "g(1)")
    },
    Case {
        label: "unsupported operand type",
        ..case(
            "T0021",
            "def f(n: int) -> int:\n    total = n * 2\n    return total + \"x\"\n",
            "total + \"x\"",
        )
    },
    Case {
        label: "these types cannot be compared",
        ..case("T0021", "def f(a: int, s: str) -> bool:\n    return a < s\n", "a < s")
    },
    Case {
        label: "expected `str`, found `int`",
        ..case(
            "T0022",
            "def f(x: int) -> str:\n    if x > 0:\n        return \"a\"\n    return x\n",
            "return x",
        )
    },
    Case {
        label: "expected `float` because of this return type",
        ..case(
            "T0022",
            "def f(x: float) -> float:\n    print(x)\n",
            "float",
        )
        .with_occurrence(1)
    },
    Case {
        label: "expected `int`, found `str`",
        ..case(
            "T0023",
            "def f(c: bool) -> int:\n    total = 0\n    if c:\n        total = \"x\"\n    return total\n",
            "\"x\"",
        )
    },
    Case {
        label: "expected `int`, found `str`",
        ..case("T0025", "def f() -> None:\n    x: int = \"s\"\n    print(x)\n", "\"s\"")
    },
    Case {
        label: "needs a type annotation",
        ..case("T0003", "def f() -> None:\n    xs = []\n    print(len(xs))\n", "[]")
    },
    Case {
        label: "not all cases are covered",
        ..case(
            "T0030",
            "def f(x: int) -> None:\n    match x:\n        case 0:\n            print(0)\n        case 1:\n            print(1)\n",
            "match x",
        )
    },
    Case {
        label: "expected `int`, found `str`",
        ..case("T0032", "xs = [1, \"two\", 3]\n", "\"two\"")
    },
    Case {
        label: "unsupported on this type",
        ..case("T0033", "def f(n: int) -> int:\n    return n[0]\n", "n[0]")
    },
    Case {
        label: "not a literal index in range",
        ..case("T0040", "def f(t: tuple[int, int]) -> int:\n    return t[5]\n", "t[5]")
    },
    Case {
        label: "may be unbound here",
        ..case(
            "T0041",
            "def f(c: bool) -> int:\n    if c:\n        y = 1\n    return y\n",
            "y",
        )
        .with_occurrence(1)
    },
    Case {
        label: "not a class instance",
        ..case("T0043", "def f(x: int) -> int:\n    return x.real_part\n", "x.real_part")
    },
    Case {
        label: "no such attribute",
        ..case(
            "T0044",
            "class P:\n    def __init__(self, x: int) -> None:\n        self.x = x\n\ndef f(p: P) -> int:\n    return p.y\n",
            "p.y",
        )
    },
    Case {
        label: "cannot assign twice to a `Final` name",
        ..case(
            "T0045",
            "from typing import Final\n\nx: Final[int] = 1\nx = 2\n",
            "x = 2",
        )
    },
    Case {
        label: "wrong number of values to unpack",
        ..case(
            "T0055",
            "def f() -> int:\n    t = (1, 2, 3)\n    a, b = t\n    return a\n",
            "a, b = t",
        )
    },
    Case {
        label: "`return` outside a function",
        ..case("T0024", "x = 1\nreturn\n", "return")
    },
    // A method's body, a lambda lifted to a `def`, an f-string
    // interpolation and an `elif` arm are located like any other code.
    Case {
        label: "expected `int`, found `str`",
        ..case(
            "T0021",
            "class Account:\n    def deposit(self, amount: int) -> int:\n        return amount\n\n\nAccount().deposit(\"ten\")\n",
            "\"ten\"",
        )
    },
    Case {
        label: "unsupported operand type",
        ..case("T0021", "sq = lambda x: x * x\nprint(sq(\"a\"))\n", "x * x")
    },
    Case {
        label: "unsupported operand type",
        ..case("T0021", "def f(x: int) -> str:\n    return f\"{x + 'a'}\"\n", "x + 'a'")
    },
    Case {
        label: "expected `str`, found `int`",
        ..case(
            "T0025",
            "def f(x: int) -> int:\n    if x == 0:\n        return 1\n    elif x == 1:\n        y: str = 3\n    return 0\n",
            "3",
        )
        .with_occurrence(0)
    },
];

impl Case {
    const fn with_occurrence(self, occurrence: usize) -> Self {
        Case { occurrence, ..self }
    }
}

/// 1-based line and column (in characters) of byte offset `offset`.
fn line_col(source: &str, offset: usize) -> (u64, u64) {
    let before = &source[..offset];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
    let column = source[line_start..offset].chars().count() + 1;
    (line as u64, column as u64)
}

fn check_json(dir: &Path, name: &str, source: &str) -> serde_json::Value {
    let path = dir.join(name);
    std::fs::write(&path, source).unwrap();
    let output = Command::new(pycc_bin())
        .args(["check", name, "--error-format", "json"])
        .current_dir(dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "{name}: {stdout}");
    let first = stdout
        .lines()
        .next()
        .unwrap_or_else(|| panic!("{name}: no diagnostic"));
    serde_json::from_str(first).unwrap_or_else(|e| panic!("{name}: {e}: {first}"))
}

#[test]
fn every_type_diagnostic_points_at_the_offending_code() {
    let dir = ScratchDir::new("type_diagnostic_locations").expect("scratch dir");
    let mut failures = Vec::new();
    for (index, case) in CASES.iter().enumerate() {
        let name = format!("case{index}.py");
        let diagnostic = check_json(&dir, &name, case.source);
        let offset = case
            .source
            .match_indices(case.at)
            .nth(case.occurrence)
            .unwrap_or_else(|| panic!("{name}: `{}` not in the source", case.at))
            .0;
        let (line, col) = line_col(case.source, offset);
        let span = &diagnostic["spans"][0];
        let actual = (
            diagnostic["code"].as_str().unwrap_or_default(),
            span["line"].as_u64(),
            span["col"].as_u64(),
            span["len"].as_u64(),
            span["label"].as_str().unwrap_or_default(),
        );
        let expected = (
            case.code,
            Some(line),
            Some(col),
            Some(case.at.chars().count() as u64),
            case.label,
        );
        if actual != expected {
            failures.push(format!(
                "{name} (`{}`): expected {expected:?}, got {actual:?}\n{diagnostic}",
                case.at
            ));
        }
        assert_ne!(
            span["label"], diagnostic["message"],
            "{name}: the label repeats the headline"
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A diagnostic in an imported module is rendered against that module's
/// own source, at the offending line.
#[test]
fn a_dependency_diagnostic_points_into_the_dependency() {
    let dir = ScratchDir::new("type_diagnostic_dependency").expect("scratch dir");
    std::fs::write(
        dir.join("helper.py"),
        "def double(n: int) -> int:\n    total = n * 2\n    return total + \"x\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("main.py"),
        "from helper import double\n\nprint(double(2))\n",
    )
    .unwrap();
    let output = Command::new(pycc_bin())
        .args(["check", "main.py"])
        .current_dir(&*dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert_eq!(
        stdout,
        "error[T0021]: operator Add is not defined for `int` and `str`\n \
         --> helper.py:3:12\n  |\n3 |     return total + \"x\"\n  |            \
         ^^^^^^^^^^^ unsupported operand type\n"
    );
}
