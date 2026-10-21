//! `bench/mypy_parity.py` (v0.9 accept 5) compares pycc's strict verdicts
//! with mypy's. Its parsing and comparison are checked here on fixed inputs,
//! without mypy: mypy's error lines (notes and summaries are not errors),
//! the DEVIATIONS table (an escaped `\|` inside a reason, a row with no
//! reason refused), and every outcome of the comparison -- a refusal mypy
//! also rejects, an unlisted refusal mypy accepts, a listed one, stale rows
//! and a reverse disagreement reported as information. The table in the
//! repository's own docs/DEVIATIONS.md parses too.

use std::process::Command;

const SCRIPT: &str = r###"
import collections, os, pathlib, sys
sys.path.insert(0, os.path.join(sys.argv[1], "bench"))
import mypy_parity as mp

out = """/tmp/x/src/p001.py:3: error: Need type annotation for "a"  [var-annotated]
/tmp/x/src/p001.py:5: error: Incompatible types in assignment (expression has type "str", variable has type "int")  [assignment]
/tmp/x/src/p001.py:5: note: Possible overload variants:
/tmp/x/src/p002.py:1:7: error: Function is missing a type annotation  [no-untyped-def]
/tmp/x/src/p002.py:4: error: Function is missing a type annotation  [no-untyped-def]
Found 4 errors in 2 files (checked 5 source files)
"""
errors = mp.parse_mypy(out)
print(sorted((k, sorted(v.items())) for k, v in errors.items()))

md = """# Title

| a | b |
|---|---|
| 999 | not this table |

## Strict refusals mypy accepts

| Program | Reason |
|---|---|
| 003 | rests on `x \\| Any` |
| 004 | listed |
| 007 | stale: pycc accepts |

## Next
| 005 | not this table |
"""
allowed = mp.parse_allowed(md)
print(sorted(allowed.items()))
try:
    mp.parse_allowed("## Strict refusals mypy accepts\n| 001 |  |\n")
    print("empty reason accepted")
except ValueError:
    print("empty reason refused")
print(mp.parse_allowed("# no table\n"))

problems = ["001", "002", "003", "004", "005", "006", "007"]
refused = {"001": "T0021: x", "003": "C0001: y", "004": "C0001: z", "005": "C0001: w"}
errors["p004"] = collections.Counter({"assignment": 1})
failures, info = mp.compare(refused, errors, problems, {**allowed, "008": "gone"})
for line in failures:
    print("FAIL", line)
for line in info:
    print("INFO", line)

root = pathlib.Path(sys.argv[1])
print("repo rows", sorted(mp.parse_allowed((root / "docs/DEVIATIONS.md").read_text(encoding="utf-8"))))
"###;

#[test]
#[ignore = "requires python3 (PYCC_PYTHON, default python3)"]
fn mypy_parity_parses_and_compares_verdicts() {
    let python = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    let out = Command::new(python)
        .args(["-I", "-B", "-c", SCRIPT, env!("CARGO_MANIFEST_DIR")])
        .output()
        .expect("python");
    let stdout = pycc_scratch::lf(&*String::from_utf8_lossy(&out.stdout)).replace("\r\n", "\n");
    assert!(out.status.success(), "{stdout}{}", pycc_scratch::lf(&*String::from_utf8_lossy(&out.stderr)));
    assert_eq!(
        stdout,
        "[('p001', [('assignment', 1), ('var-annotated', 1)]), ('p002', [('no-untyped-def', 2)])]\n\
         [('003', 'rests on `x \\\\| Any`'), ('004', 'listed'), ('007', 'stale: pycc accepts')]\n\
         empty reason refused\n\
         {}\n\
         FAIL 004: listed in docs/DEVIATIONS.md, but mypy rejects it: remove the row\n\
         FAIL 005: pycc strict refuses (C0001: w), mypy --strict accepts, and docs/DEVIATIONS.md does not list it\n\
         FAIL 007: listed in docs/DEVIATIONS.md, but pycc accepts it: remove the row\n\
         FAIL 008: listed in docs/DEVIATIONS.md, but no such corpus program\n\
         INFO 002: mypy rejects (no-untyped-def x2), pycc strict accepts\n\
         repo rows ['049']\n"
    );
}
