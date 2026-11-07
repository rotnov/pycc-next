//! The identity of pass 3's winning set and the check that it still holds
//! (docs/design/compile-little.md, section 2.3, step 5).

use super::*;

/// The identity of pass 3's winning set: its members, each with the source
/// it was measured with, and the `--exclude` list and the interpreter
/// version it was measured under. A build compiles it only when all of
/// them still hold, and then exactly its members.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Winner {
    /// Module, qualified name, first line and source hash of each member.
    pub(crate) members: Vec<(String, String, u32, String)>,
    /// The modules kept in Python (`--exclude`), sorted.
    pub(crate) excludes: Vec<String>,
    /// The interpreter's version (`platform.python_version()`).
    pub(crate) python: String,
}

/// What a check of a profile's winning set compares it with: the whole
/// package's current sources (every module, whichever one a build
/// compiles), the interpreter's version, and the build's `--exclude` list
/// (`None` for a build of one module, which has none: then only its own
/// module must not be one the set was measured without).
pub(crate) struct Scope<'a> {
    pub(crate) sources: &'a [Function],
    pub(crate) python: &'a str,
    pub(crate) excludes: Option<&'a [String]>,
    /// The module a single-module build compiles.
    pub(crate) module: Option<&'a str>,
}

/// `excludes` as a [`Winner`] records them.
pub(crate) fn normalized(excludes: &[String]) -> Vec<String> {
    let set: BTreeSet<&String> = excludes.iter().collect();
    set.into_iter().cloned().collect()
}

/// Whether a function is in pass 3's winning set, as [`select`] reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Winning<'p> {
    /// Not a member (or no set was measured).
    No,
    /// A member of a valid winning set.
    Member,
    /// A member of a set that is no longer valid, and why.
    Invalid(&'p str),
}


impl Profile {
    /// Whether the winning set pass 3 measured is still valid in `scope`:
    /// every member has the source it was measured with somewhere in the
    /// package, and the interpreter and the `--exclude` list are the ones
    /// it was measured under; else why not. A profile pass 3 has not run on
    /// has no set, which is valid.
    pub(crate) fn winner_valid(&self, scope: &Scope<'_>) -> Result<(), String> {
        if !self.verified {
            return Ok(());
        }
        let Some(winner) = &self.winner else {
            return Err("the profile records no winning set (a profile of an older pycc)".to_string());
        };
        let minor = |v: &str| v.split('.').take(2).collect::<Vec<_>>().join(".");
        if minor(&winner.python) != minor(scope.python) {
            return Err(format!(
                "it was measured with Python {}, and this build uses Python {}",
                winner.python, scope.python
            ));
        }
        match scope.excludes {
            Some(excludes) if normalized(excludes) != winner.excludes => {
                return Err(format!(
                    "it was measured with --exclude {:?}, and this run has --exclude {:?}",
                    winner.excludes,
                    normalized(excludes)
                ));
            }
            _ => {}
        }
        if let Some(module) = scope.module
            && let Some(package) = module.split('.').next()
            && winner.excludes.iter().any(|e| {
                let relative = module.strip_prefix(package).and_then(|r| r.strip_prefix('.')).unwrap_or("");
                crate::package_build::excluded(package, relative, std::slice::from_ref(e))
            })
        {
            return Err(format!("it was measured with `{module}` kept in Python (--exclude)"));
        }
        for (module, qualname, line, sha256) in &winner.members {
            let found = scope
                .sources
                .iter()
                .find(|f| &f.module == module && &f.qualname == qualname && &f.sha256 == sha256);
            match found {
                Some(f) if f.excluded => {
                    return Err(format!("its member {module}:{qualname} is kept in Python (--exclude)"));
                }
                Some(_) => {}
                None => {
                    return Err(format!(
                        "its member {module}:{qualname} (line {line}) changed or is gone"
                    ));
                }
            }
        }
        Ok(())
    }

    /// Marks each of `candidates` that is a member of the winning set (by
    /// module, qualified name and source hash), a member of a valid one
    /// when `valid` is `Ok`.
    pub(crate) fn mark<'p>(&self, candidates: &mut [Candidate<'p>], valid: &'p Result<(), String>) {
        // An invalid set selects nothing: every candidate says why.
        if let Err(why) = valid {
            for c in candidates {
                c.winning = Winning::Invalid(why.as_str());
            }
            return;
        }
        let Some(winner) = &self.winner else {
            return;
        };
        let members: BTreeSet<(&str, &str, &str)> = winner
            .members
            .iter()
            .map(|(m, q, _, sha)| (m.as_str(), q.as_str(), sha.as_str()))
            .collect();
        for c in candidates {
            let e = c.entry;
            if members.contains(&(e.module.as_str(), e.qualname.as_str(), e.sha256.as_str())) {
                c.winning = Winning::Member;
            }
        }
    }
}
