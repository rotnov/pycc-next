use super::*;
use crate::project_config;

fn table(text: &str) -> toml::Value {
    toml::from_str::<toml::Value>(text)
        .expect("valid TOML")
        .get("interop")
        .cloned()
        .expect("an interop value")
}

fn rejected(text: &str) -> String {
    validate_table(&table(text)).expect_err("the table must be rejected")
}

#[test]
fn a_valid_table_defaults_to_auto_with_no_roots() {
    assert_eq!(
        validate_table(&table("[interop]\n")),
        Ok(ConfiguredPolicy {
            policy: InteropPolicy::Auto,
            allow: Vec::new(),
        })
    );
    assert_eq!(
        validate_table(&table(
            "[interop]\npolicy = \"allowlist\"\nallow = [\"json\", \"numpy\"]\n"
        )),
        Ok(ConfiguredPolicy {
            policy: InteropPolicy::Allowlist,
            allow: vec!["json".to_string(), "numpy".to_string()],
        })
    );
    assert_eq!(
        validate_table(&table("[interop]\npolicy = \"deny\"\nallow = []\n")),
        Ok(ConfiguredPolicy {
            policy: InteropPolicy::Deny,
            allow: Vec::new(),
        })
    );
}

#[test]
fn every_malformed_table_is_rejected_with_its_own_reason() {
    for (text, expected) in [
        (
            "interop = \"deny\"\n",
            "`interop` must be a table (`[interop]`), not a string",
        ),
        (
            "[interop]\nmode = \"deny\"\n",
            "unknown key `mode` in `[interop]`",
        ),
        (
            "[interop]\npolicy = \"bogus\"\n",
            "must be `auto`, `allowlist` or `deny`, not `bogus`",
        ),
        (
            "[interop]\npolicy = 1\n",
            "`[interop] policy` must be a string, not a integer",
        ),
        (
            "[interop]\npolicy = \"allowlist\"\nallow = \"json\"\n",
            "must be an array of strings, not a string",
        ),
        (
            "[interop]\npolicy = \"allowlist\"\nallow = [1]\n",
            "entries must be strings, not a integer",
        ),
        (
            "[interop]\npolicy = \"allowlist\"\nallow = [\"\"]\n",
            "an `[interop] allow` entry is empty",
        ),
        (
            "[interop]\npolicy = \"allowlist\"\nallow = [\"os.path\"]\n",
            "entry `os.path` is dotted; `allow` lists direct import roots such as `os`",
        ),
        ("[interop]\nallow = [\"json\"]\n", "the policy is `auto`"),
        (
            "[interop]\npolicy = \"auto\"\nallow = [\"json\"]\n",
            "the policy is `auto`",
        ),
        (
            "[interop]\npolicy = \"deny\"\nallow = [\"json\"]\n",
            "the policy is `deny`",
        ),
    ] {
        let message = rejected(text);
        assert!(message.starts_with("pycc.toml: "), "{message}");
        assert!(message.contains(expected), "{text:?}: {message}");
    }
}

fn manifest(interop: Option<&str>) -> DiscoveredManifest {
    let mut text = "[project]\nname = \"t\"\nentry = \"main.py\"\npython = \"3.14\"\n".to_string();
    if let Some(interop) = interop {
        text.push_str(interop);
    }
    DiscoveredManifest {
        display: "proj\\pycc.toml".to_string(),
        config: project_config::parse(&text).expect("a valid manifest"),
    }
}

fn cli(policy: Option<InteropPolicy>) -> InteropCli {
    InteropCli {
        policy,
        pure: false,
    }
}

const PURE: InteropCli = InteropCli {
    policy: None,
    pure: true,
};

fn resolved(manifest: Option<&DiscoveredManifest>, cli: InteropCli) -> EffectivePolicy {
    match resolve(manifest, cli) {
        Ok(policy) => policy,
        Err(_) => panic!("the policy must resolve"),
    }
}

fn manifest_source() -> PolicySource {
    PolicySource::Manifest("proj/pycc.toml".to_string())
}

#[test]
fn with_no_flag_and_no_table_the_policy_is_auto() {
    assert_eq!(resolved(None, InteropCli::default()), EffectivePolicy::Auto);
    assert_eq!(
        resolved(Some(&manifest(None)), InteropCli::default()),
        EffectivePolicy::Auto
    );
}

#[test]
fn a_configured_policy_applies_when_no_flag_is_given() {
    let allowlist = manifest(Some(
        "[interop]\npolicy = \"allowlist\"\nallow = [\"json\"]\n",
    ));
    assert_eq!(
        resolved(Some(&allowlist), InteropCli::default()),
        EffectivePolicy::Allowlist {
            allow: vec!["json".to_string()],
            source: manifest_source(),
        }
    );
    let deny = manifest(Some("[interop]\npolicy = \"deny\"\n"));
    assert_eq!(
        resolved(Some(&deny), InteropCli::default()),
        EffectivePolicy::Deny {
            source: manifest_source()
        }
    );
    let auto = manifest(Some("[interop]\npolicy = \"auto\"\n"));
    assert_eq!(
        resolved(Some(&auto), InteropCli::default()),
        EffectivePolicy::Auto
    );
}

#[test]
fn an_explicit_flag_overrides_the_configured_policy_in_both_directions() {
    let allowlist = manifest(Some(
        "[interop]\npolicy = \"allowlist\"\nallow = [\"json\"]\n",
    ));
    let deny = manifest(Some("[interop]\npolicy = \"deny\"\n"));
    assert_eq!(
        resolved(Some(&allowlist), cli(Some(InteropPolicy::Auto))),
        EffectivePolicy::Auto
    );
    assert_eq!(
        resolved(Some(&deny), cli(Some(InteropPolicy::Auto))),
        EffectivePolicy::Auto
    );
    assert_eq!(
        resolved(Some(&allowlist), cli(Some(InteropPolicy::Deny))),
        EffectivePolicy::Deny {
            source: PolicySource::CliFlag
        }
    );
    assert_eq!(
        resolved(Some(&allowlist), PURE),
        EffectivePolicy::Deny {
            source: PolicySource::Pure
        }
    );
    // An explicit `allowlist` over a configured one keeps the configured
    // roots: they are the only place roots come from.
    assert_eq!(
        resolved(Some(&allowlist), cli(Some(InteropPolicy::Allowlist))),
        EffectivePolicy::Allowlist {
            allow: vec!["json".to_string()],
            source: PolicySource::CliFlag,
        }
    );
}

#[test]
fn a_switch_to_allowlist_without_configured_roots_has_an_empty_allow_set() {
    for manifest in [
        None,
        Some(manifest(None)),
        Some(manifest(Some("[interop]\npolicy = \"deny\"\n"))),
        Some(manifest(Some("[interop]\npolicy = \"auto\"\n"))),
    ] {
        assert_eq!(
            resolved(manifest.as_ref(), cli(Some(InteropPolicy::Allowlist))),
            EffectivePolicy::Allowlist {
                allow: Vec::new(),
                source: PolicySource::CliFlag,
            }
        );
    }
}

#[test]
fn a_malformed_table_is_reported_even_when_a_flag_overrides_it() {
    let bad = manifest(Some("[interop]\npolicy = \"bogus\"\n"));
    for flags in [
        InteropCli::default(),
        cli(Some(InteropPolicy::Auto)),
        cli(Some(InteropPolicy::Allowlist)),
        cli(Some(InteropPolicy::Deny)),
        PURE,
    ] {
        match resolve(Some(&bad), flags) {
            Err(FrontendFailure::Input { path, message }) => {
                assert_eq!(path, "proj\\pycc.toml");
                assert!(message.contains("not `bogus`"), "{message}");
            }
            _ => panic!("{flags:?}: a malformed table must be an input failure"),
        }
    }
}

fn hir(imports: Vec<ImportBinding>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: Vec::new(),
        type_aliases: Vec::new(),
        imports,
        class_defs: Vec::new(),
    }
}

/// The binding of an unaliased `import {path}`, which binds the root of a
/// dotted path (#1381).
fn foreign(path: &str, start: u32) -> ImportBinding {
    ImportBinding::Foreign {
        local_name: path.split('.').next().unwrap_or(path).to_string(),
        module_path: path.to_string(),
        from: None,
        site: pycc_hir::ForeignImportSite::Item(0),
        span: Span::new(start, start + 1),
    }
}

fn project() -> ImportBinding {
    ImportBinding::Project {
        local_name: "helper".to_string(),
        module_path: "pkg.helper".to_string(),
        kind: pycc_hir::ProjectBindingKind::Function,
    }
}

#[test]
fn a_program_without_a_foreign_import_never_reads_the_table() {
    let bad = manifest(Some("[interop]\npolicy = \"bogus\"\n"));
    match resolve_for_program(&hir(vec![project()]), Some(&bad), PURE) {
        Ok(policy) => assert_eq!(policy, EffectivePolicy::Auto),
        Err(_) => panic!("a native-only program must not validate `[interop]`"),
    }
    match resolve_for_program(&hir(vec![foreign("json", 0)]), Some(&bad), PURE) {
        Err(FrontendFailure::Input { .. }) => {}
        _ => panic!("a program with a CPython import validates `[interop]`"),
    }
    match resolve_for_program(&hir(vec![foreign("json", 0)]), None, PURE) {
        Ok(policy) => assert_eq!(
            policy,
            EffectivePolicy::Deny {
                source: PolicySource::Pure
            }
        ),
        Err(_) => panic!("no manifest is not an error"),
    }
}

fn gaps(policy: &EffectivePolicy, imports: Vec<ImportBinding>) -> Vec<(usize, String)> {
    policy_gaps(&hir(imports), policy)
        .into_iter()
        .map(|(position, gap)| {
            assert_eq!(gap.code, "I0402");
            (position, gap.message)
        })
        .collect()
}

#[test]
fn auto_rejects_nothing() {
    assert!(gaps(&EffectivePolicy::Auto, vec![foreign("numpy", 0), project()]).is_empty());
}

#[test]
fn deny_rejects_every_foreign_import_and_names_its_source() {
    for (source, named) in [
        (PolicySource::Pure, "set by `--pure`"),
        (PolicySource::CliFlag, "set by `--interop-policy deny`"),
        (manifest_source(), "set by `proj/pycc.toml`"),
    ] {
        let found = gaps(
            &EffectivePolicy::Deny { source },
            vec![foreign("json", 0), project(), foreign("numpy", 7)],
        );
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].0, 0);
        assert_eq!(found[1].0, 2);
        assert!(
            found[0].1.starts_with(
                "`import json` is a CPython-backed import, which the `deny` interop policy"
            ),
            "{}",
            found[0].1
        );
        assert!(found[1].1.contains(named), "{}", found[1].1);
    }
}

#[test]
fn allowlist_admits_listed_roots_and_covers_their_submodules() {
    let policy = EffectivePolicy::Allowlist {
        allow: vec!["json".to_string()],
        source: PolicySource::CliFlag,
    };
    // `import json.decoder` binds the root `json` (#1381) and is judged by
    // that root, pinned here on a hand-built binding.
    let found = gaps(
        &policy,
        vec![
            foreign("json", 0),
            foreign("json.decoder", 5),
            foreign("pprint", 9),
            foreign("jsonschema", 12),
        ],
    );
    assert_eq!(
        found
            .iter()
            .map(|(position, _)| *position)
            .collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert!(
        found[0].1.contains(
            "the `allowlist` interop policy (set by `--interop-policy allowlist`) rejects: \
             its root `pprint` is not in `[interop] allow`"
        ),
        "{}",
        found[0].1
    );
}

#[test]
fn a_rejection_is_reported_at_the_import_span() {
    let gap = rejection(
        &EffectivePolicy::Deny {
            source: PolicySource::Pure,
        },
        "numpy",
        "numpy",
        None,
        Span::new(3, 9),
    )
    .expect("deny rejects");
    assert_eq!(gap.span, Some(Span::new(3, 9)));
}

/// #1291: a nested import is judged like a top-level one, at its own span.
#[test]
fn a_block_foreign_import_is_judged_at_its_own_span() {
    let nested = ImportBinding::Foreign {
        local_name: "numpy".to_string(),
        module_path: "numpy".to_string(),
        from: None,
        site: pycc_hir::ForeignImportSite::Block { optional: false },
        span: Span::new(10, 22),
    };
    let found = policy_gaps(
        &hir(vec![nested]),
        &EffectivePolicy::Deny {
            source: PolicySource::CliFlag,
        },
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, 0);
    assert_eq!(found[0].1.code, "I0402");
    assert_eq!(found[0].1.span, Some(Span::new(10, 22)));
}

/// The binding `from {module} import {names}` makes for `names[index]`.
fn from_foreign(module: &str, names: &[&str], index: usize) -> ImportBinding {
    ImportBinding::Foreign {
        local_name: names[index].to_string(),
        module_path: module.to_string(),
        from: Some(pycc_hir::FromImport {
            name: names[index].to_string(),
            fromlist: names.iter().map(ToString::to_string).collect(),
            index,
            level: 0,
        }),
        site: pycc_hir::ForeignImportSite::Item(0),
        span: Span::new(0, 29),
    }
}

/// #1278: one statement, one `I0402`, quoting the statement as written --
/// under both rejecting policies.
#[test]
fn a_multi_name_from_import_is_rejected_once_quoting_the_statement() {
    let statement = || {
        vec![
            from_foreign("tkinter", &["Tk", "Label"], 0),
            from_foreign("tkinter", &["Tk", "Label"], 1),
        ]
    };
    for policy in [
        EffectivePolicy::Deny {
            source: PolicySource::CliFlag,
        },
        EffectivePolicy::Allowlist {
            allow: vec!["json".to_string()],
            source: PolicySource::CliFlag,
        },
    ] {
        let found = gaps(&policy, statement());
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, 0);
        assert!(
            found[0]
                .1
                .starts_with("`from tkinter import Tk, Label` is a CPython-backed import"),
            "{}",
            found[0].1
        );
    }
    let allowed = EffectivePolicy::Allowlist {
        allow: vec!["tkinter".to_string()],
        source: PolicySource::CliFlag,
    };
    assert!(gaps(&allowed, statement()).is_empty());
}
