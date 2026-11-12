//! `pycc --help` and `pycc <command> --help` describe every subcommand and
//! flag for the user. clap takes the descriptions from doc comments, and
//! v0.12.0 printed internal ones there: a flattened argument struct's doc
//! comment as `run`'s description, issue numbers and decision ids in flag
//! help, and no description at all for several subcommands.

use std::process::Command;

const SUBCOMMANDS: [&str; 10] = [
    "build", "run", "check", "lock", "profile", "test", "explain", "init", "clean", "version",
];

fn help(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(args)
        .output()
        .expect("pycc runs");
    assert!(output.status.success(), "pycc {args:?} failed: {output:?}");
    String::from_utf8(output.stdout).expect("help is UTF-8")
}

/// The internal reference `text` holds, if any: a repository path, an issue
/// number, a decision id, a design-document section or an intra-doc link.
fn internal_marker(text: &str) -> Option<String> {
    for marker in ["docs/", "CLI_SPEC", ".md", "section", "[`", "Internal ("] {
        if text.contains(marker) {
            return Some(marker.to_string());
        }
    }
    let bytes = text.as_bytes();
    for (i, window) in bytes.windows(2).enumerate() {
        let next_is_digit = |at: usize| bytes.get(at).is_some_and(u8::is_ascii_digit);
        // `#3`, `#1224`: an issue number.
        if window[0] == b'#' && next_is_digit(i + 1) {
            return Some(text[i..].chars().take(6).collect());
        }
        // `D-249`: a decision id, not part of a longer word.
        let word_start = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if word_start && window == b"D-" && next_is_digit(i + 2) {
            return Some(text[i..].chars().take(6).collect());
        }
    }
    None
}

/// `pycc --help`'s `Commands:` section, as (name, description) pairs.
fn listed_subcommands(top: &str) -> Vec<(String, String)> {
    let section = top
        .split_once("Commands:\n")
        .expect("`pycc --help` lists its commands")
        .1;
    section
        .lines()
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let line = line.trim();
            let (name, description) = line.split_once(' ').unwrap_or((line, ""));
            (name != "help").then(|| (name.to_string(), description.trim().to_string()))
        })
        .collect()
}

#[test]
fn every_subcommand_has_a_user_facing_description() {
    let top = help(&["--help"]);
    let listed = listed_subcommands(&top);
    let names: Vec<&str> = listed.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, SUBCOMMANDS, "`pycc --help` lists:\n{top}");
    for (name, description) in &listed {
        assert!(
            !description.is_empty(),
            "`{name}` has no description:\n{top}"
        );
        assert_eq!(
            internal_marker(description),
            None,
            "`{name}`'s description leaks an internal reference: {description}"
        );
    }
    assert_eq!(internal_marker(&top), None, "`pycc --help`:\n{top}");
}

#[test]
fn every_subcommand_help_is_user_facing() {
    for name in SUBCOMMANDS {
        for flag in ["--help", "-h"] {
            let text = help(&[name, flag]);
            assert!(
                text.lines()
                    .next()
                    .is_some_and(|line| !line.starts_with("Usage:")),
                "`pycc {name} {flag}` has no description:\n{text}"
            );
            assert_eq!(
                internal_marker(&text),
                None,
                "`pycc {name} {flag}` leaks an internal reference:\n{text}"
            );
        }
    }
}

#[test]
fn internal_marker_finds_the_v0_12_0_leaks() {
    for leaked in [
        "Non-strict mode (#3), shared by `build`, `run`, `check` and `lock`",
        "(the pycc.lock decision entry, D-249; ...)",
        "Which functions a non-strict `--ext` build compiles (docs/design/compile-little.md, section 2)",
        "it in at most [`AUTO_MAX_WALL_NS`]; else wall time",
    ] {
        assert!(internal_marker(leaked).is_some(), "missed: {leaked}");
    }
    for clean in [
        "Compile a Python program and run it",
        "is a `C0003` error",
        "an `I0403` error, CPython-backed imports",
        "the shared library (ABI-3), 0.5% of time",
    ] {
        assert_eq!(internal_marker(clean), None, "flagged: {clean}");
    }
}
