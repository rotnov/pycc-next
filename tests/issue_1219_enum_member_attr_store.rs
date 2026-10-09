//! End-to-end proof for [#1219](https://github.com/rotnov/pycc/issues/1219):
//! assigning to an `Enum` member's `value` or `name` is refused with `T0044`
//! instead of compiling and mutating the member. CPython raises
//! `AttributeError` for every one of these stores, so a refusal is the only
//! answer pycc can give without a wrong result.

use pycc_scratch::ScratchDir;
use std::process::{Command, Output};

const ENUM: &str = "from enum import Enum\n\nclass Color(Enum):\n    RED = 1\n\n";

/// Each store and the attribute it names: through a parameter, through a
/// module-level alias, through the member expression itself, and as an
/// augmented assignment (#1209), which is lowered as the plain store.
const STORES: [(&str, &str); 4] = [
    (
        "def f(c: Color) -> None:\n    c.value = c.value + 1\n\nf(Color.RED)\n",
        "value",
    ),
    ("c = Color.RED\nc.name = \"x\"\n", "name"),
    ("Color.RED.value = 3\n", "value"),
    ("c = Color.RED\nc.value += 1\n", "value"),
];

fn write(dir: &ScratchDir, index: usize, store: &str) -> std::path::PathBuf {
    let path = dir.join(format!("store_{index}.py"));
    std::fs::write(&path, format!("{ENUM}{store}")).expect("write the subject");
    path
}

fn rendered(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
}

#[test]
fn an_enum_member_attribute_store_fails_pycc_check_with_t0044() {
    let dir = ScratchDir::new("e2e_issue_1219_check").expect("scratch");
    for (index, (store, attr)) in STORES.iter().enumerate() {
        let path = write(&dir, index, store);
        let output = Command::new(env!("CARGO_BIN_EXE_pycc"))
            .arg("check")
            .arg(&path)
            .output()
            .expect("pycc should spawn");
        assert!(!output.status.success(), "{store}");
        let text = rendered(&output);
        assert!(
            text.contains(&format!(
                "error[T0044]: cannot assign to `{attr}` of a member of enum `Color`: an enum \
                 member's `value` and `name` are read-only (CPython raises `AttributeError`)"
            )),
            "{store}: {text}"
        );
    }
}

/// The oracle half: the pinned CPython raises `AttributeError` for the same
/// stores, so the refusal replaces a divergence rather than a working program.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn cpython_raises_attribute_error_for_every_refused_store() {
    let dir = ScratchDir::new("e2e_issue_1219_oracle").expect("scratch");
    for (index, (store, attr)) in STORES.iter().enumerate() {
        let path = write(&dir, index, store);
        let output =
            Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
                .arg(&path)
                .output()
                .expect("python3 should spawn");
        assert!(!output.status.success(), "{store}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(&format!(
                "AttributeError: <enum 'Enum'> cannot set attribute '{attr}'"
            )),
            "{store}: {stderr}"
        );
    }
}
