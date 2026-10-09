//! The D-029 mechanical guard's checking logic, split into its own
//! cohesion-driven submodule under AGENTS.md's decomposability rule
//! ("a Rust source file over ~1,000 lines is a maintainability and
//! agent-context risk ... decompose the part it touches into
//! cohesion-driven submodules as part of that same change"). #619
//! touched `tests.rs` (already far past that threshold) to extract this
//! guard's checks into a reusable function and add automated coverage
//! for it; this file is that extraction's home instead of growing the
//! parent file further.
//!
//! `tests.rs`'s own
//! `every_inkwell_llvm_string_call_routes_through_a_d029_wrapper` test
//! stays in the parent file (it belongs with this crate's other
//! `#[test]` functions and is the one thing every contributor actually
//! runs), and calls into [`d029_violations`] here.

/// The three D-029 checks that
/// `super::every_inkwell_llvm_string_call_routes_through_a_d029_wrapper`
/// runs against the crate's real sources, extracted so this module's own
/// tests can drive the identical logic against synthetic sources
/// instead.
///
/// `sources` is `(path, contents)` pairs, matching what a directory scan
/// yields; the path is accepted but not currently used by any check
/// (kept for parity with the real scan and for messages a future check
/// might want to name a file in). `expected_triple_call_sites` is the
/// tripwire target for check 3 below -- the real caller passes the
/// crate's current count (2); a synthetic fixture passes whatever count
/// its own snippet actually contains, since the tripwire is inherently
/// tied to how many call sites exist in whatever source it is pointed
/// at, not a universal constant.
///
/// Pure and panic-free: it returns one message per violated check
/// (empty when `sources` fully complies) rather than asserting, so a
/// caller -- the real test in the parent module, or a synthetic
/// negative test below -- decides what a violation means for it.
///
/// D-029 records three distinct protections, and this function checks
/// them to three different depths -- state that plainly rather than
/// letting the function's name imply uniform coverage:
///
///  1. `llvm_string_to_owned`, which forgets the wrapper instead of
///     dropping it. Fully checked: every `print_to_string` call must
///     name it on the same line.
///  2. `verify_module`, a no-op under `#[cfg(windows)]`. Fully checked:
///     exactly one direct `verify` call may exist, the wrapper's own.
///  3. `ManuallyDrop` at the point a `TargetTriple` is created, which
///     covers every exit path including the early `?`. Only tripwired:
///     the wrapping is structural and spans several lines, so a
///     line-oriented scan cannot confirm it. What it can do is pin the
///     number of triple-producing call sites, so adding one without
///     raising `expected_triple_call_sites` is flagged as a violation.
///
/// `Target::from_triple`'s and `write_to_file`'s `.map_err` sites fall
/// under (1) and are checked only insofar as they name the wrapper.
///
/// The needles are assembled at run time (`format!` over split
/// fragments, never a literal like `.print_to_string()`) so that this
/// function's own source, and any fixture string a caller builds the
/// same way, is never mistaken for a violation of itself -- important
/// specifically because the real caller in the parent module scans this
/// whole crate's `src/` directory, which now includes this file too.
pub(super) fn d029_violations(
    sources: &[(&str, &str)],
    expected_triple_call_sites: usize,
) -> Vec<String> {
    let printer = format!(".{}()", "print_to_string");
    let verifier = format!(".{}()", "verify");
    let wrapper = format!("llvm_string_to_{}(", "owned");
    let created = format!("TargetTriple::{}(", "create");
    let defaulted = format!("TargetMachine::get_default_{}()", "triple");
    let code_lines = || {
        sources
            .iter()
            .flat_map(|(_, source)| source.lines())
            .filter(|line| !line.trim_start().starts_with("//"))
    };

    let mut violations = Vec::new();

    // 1. Deliberately not keyed on the receiver's name: a correctly
    //    wrapped call on some other inkwell value must pass too.
    let printer_calls = code_lines().filter(|line| line.contains(&printer)).count();
    let wrapped_printer_calls = code_lines()
        .filter(|line| line.contains(&printer) && line.contains(&wrapper))
        .count();
    if printer_calls != wrapped_printer_calls {
        violations.push(
            "every inkwell print_to_string call must be an argument of \
             llvm_string_to_owned, or its LLVMString drops and faults on Windows (D-029)"
                .to_string(),
        );
    }

    // 2.
    let verify_calls = code_lines().filter(|line| line.contains(&verifier)).count();
    if verify_calls != 1 {
        violations.push(
            "the only direct inkwell verify call may be the one inside verify_module, \
             which is skipped on Windows; everything else must go through that wrapper (D-029)"
                .to_string(),
        );
    }

    // 3.
    let triple_calls = code_lines()
        .filter(|line| line.contains(&created) || line.contains(&defaulted))
        .count();
    if triple_calls != expected_triple_call_sites {
        violations.push(
            "a TargetTriple owns an LLVMString and must be created inside a ManuallyDrop \
             (D-029); this count is a tripwire, so if you added a call site, wrap it and \
             raise the number -- if you removed one, lower it"
                .to_string(),
        );
    }

    violations
}

// Automated proof of the five cases `.harden/incidents/platform-wrapper-bypassed-by-new-code/incident.md`
// previously recorded as `verify: manual` -- run by hand once and pasted
// into that file as a shell transcript, with no fixture and no gate
// against the checking logic itself regressing. #619 is exactly this:
// the guard test in the parent module catches a violator in *this
// crate's* real sources, but nothing proved the checking logic itself
// still recognizes each violation shape after some future edit to
// `d029_violations`. These tests are that proof, kept in sync with the
// incident file's own A-E labels.
mod d029_violations_tests {
    use super::d029_violations;

    // A fully compliant single-file fixture every negative case below
    // starts from and edits in one place, so each test's diff against
    // it is the one thing that test is actually about. Two triple call
    // sites, one verify call inside `verify_module`, one printer call
    // correctly wrapped -- exactly what the real crate's own sources
    // look like structurally, kept intentionally tiny.
    fn compliant_fixture() -> String {
        format!(
            "fn verify_module(module: &Module) {{\n\
             \x20   #[cfg(not(windows))]\n\
             \x20   module.{a}().expect(\"a pycc_codegen bug\");\n\
             }}\n\
             fn owned_ir(module: &Module) -> String {{\n\
             \x20   llvm_string_to_{b}(module.{c}())\n\
             }}\n\
             fn host_triple() -> TargetTriple {{\n\
             \x20   std::mem::ManuallyDrop::new(TargetTriple::{d}(\"host\"))\n\
             }}\n\
             fn default_triple() -> TargetTriple {{\n\
             \x20   std::mem::ManuallyDrop::new(TargetMachine::get_default_{e}())\n\
             }}\n",
            a = "verify",
            b = "owned",
            c = "print_to_string",
            d = "create",
            e = "triple",
        )
    }

    #[test]
    fn a_fully_compliant_source_has_no_violations() {
        let source = compliant_fixture();
        assert_eq!(
            d029_violations(&[("fixture.rs", &source)], 2),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_a_bare_verify_call_added_alongside_the_wrapper_is_a_violation() {
        // A second, unwrapped verify call -- e.g. a test helper reaching
        // past `verify_module` straight to inkwell's own API.
        let source = format!(
            "{}\nfn bare(module: &Module) {{ module.{}().unwrap(); }}\n",
            compliant_fixture(),
            "verify"
        );
        let violations = d029_violations(&[("fixture.rs", &source)], 2);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("the only direct inkwell verify call"));
    }

    #[test]
    fn b_a_printer_call_missing_the_wrapper_is_a_violation() {
        // The printer API called without `llvm_string_to_owned` around it.
        let source = format!(
            "{}\nfn unwrapped(module: &Module) -> inkwell::support::LLVMString {{ module.{}() }}\n",
            compliant_fixture(),
            "print_to_string"
        );
        let violations = d029_violations(&[("fixture.rs", &source)], 2);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("must be an argument of"));
    }

    #[test]
    fn c_the_same_violation_in_a_sibling_module_is_still_caught() {
        // Case A's violation (a bare, unwrapped verify call) again, but
        // planted in a second `(path, contents)` entry instead of the
        // first -- proving the scan is not limited to a single file.
        // Matches the incident file's own case C, which reuses A's
        // violation shape rather than introducing a new one.
        let sibling = format!(
            "fn bare(module: &Module) {{ module.{}().unwrap(); }}\n",
            "verify"
        );
        let compliant = compliant_fixture();
        let violations = d029_violations(&[("lib.rs", &compliant), ("sibling.rs", &sibling)], 2);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("the only direct inkwell verify call"));
    }

    #[test]
    fn d_a_new_unsuppressed_triple_call_site_is_a_violation() {
        // A third triple-producing call site with no matching raise of
        // the tripwire's expected count.
        let source = format!(
            "{}\nfn another_triple() -> TargetTriple {{ TargetTriple::{}(\"other\") }}\n",
            compliant_fixture(),
            "create"
        );
        let violations = d029_violations(&[("fixture.rs", &source)], 2);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("must be created inside a ManuallyDrop"));
    }

    #[test]
    fn e_a_correctly_wrapped_printer_call_on_a_different_receiver_is_accepted() {
        // The negative control: check 1 is deliberately not keyed on the
        // receiver's name, so wrapping a call on some other value must
        // still pass.
        let source = format!(
            "{}\nfn owned_other(other_module: &Module) -> String {{ llvm_string_to_{}(other_module.{}()) }}\n",
            compliant_fixture(),
            "owned",
            "print_to_string"
        );
        assert_eq!(
            d029_violations(&[("fixture.rs", &source)], 2),
            Vec::<String>::new()
        );
    }

    #[test]
    fn raising_the_expected_triple_count_accepts_an_added_wrapped_call_site() {
        // The tripwire's other side: a legitimately added, correctly
        // wrapped call site is accepted once the caller raises the
        // expected count to match -- proving the check compares against
        // the caller-supplied expectation, not a hardcoded constant.
        let source = format!(
            "{}\nfn another_triple() -> TargetTriple {{ std::mem::ManuallyDrop::new(TargetTriple::{}(\"other\")) }}\n",
            compliant_fixture(),
            "create"
        );
        assert_eq!(
            d029_violations(&[("fixture.rs", &source)], 3),
            Vec::<String>::new()
        );
    }
}
