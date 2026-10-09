#!/usr/bin/env ruby
# frozen_string_literal: true

require "pathname"
require "psych"
require "digest"
require "json"

class RoadmapEvidenceError < StandardError; end

LIST_ITEM =
  /\A(?<indent>[ \t]*)(?<marker>[-*+]|\d+[.)])(?:(?<spacing>[ \t]+)(?<body>[^\r\n]*))?(?:\r?\n)?\z/
CHECKED_ITEM_BODY = /\A\[[xX]\][ \t]+(?<claim>.*)$/
ATX_HEADING = /\A {0,3}(?<marks>\#{1,6})[ \t]+(?<title>.*)$/
SETEXT_UNDERLINE = /\A {0,3}(?:=+|-+)[ \t]*(?:\r?\n)?\z/
RAW_HTML_BLOCK_START =
  /\A {0,3}<(?:\/?[A-Za-z][A-Za-z0-9-]*(?=[ \t\/>])|\?|![A-Z]|!\[CDATA\[)/i
EVIDENCE_MARKER = /<!--\s*roadmap-evidence:\s*(?<id>[a-z0-9][a-z0-9-]*)\s*-->/
EVIDENCE_CLAIMS = {
  "ci-tier1-cross-compile" =>
    "The five-target native CI matrix and one cross-host compilation path are live on `main`.",
  "ci-build-test-coverage-100" =>
    "The 100% line and region coverage gate is required and green for the current slice.",
  "conformance-fib-mandelbrot-tier1" =>
    "`fib` and `mandelbrot-ascii` compile and match CPython output on all five Tier-1 targets.",
  "check-throughput-1k-loc-75ms" =>
    "`pycc check` processes 1k LOC in under 75 ms.",
  "cli-spec-diagnostic-match" =>
    "The error demonstration matches the stable [CLI specification](./CLI_SPEC.md) output.",
  "readme-coverage-badge-bound" =>
    "The README coverage badge percentage is bound to ci.yml's enforced --fail-under-lines and --fail-under-regions thresholds."
}.freeze
EVIDENCE_SECTIONS = {
  "ci-tier1-cross-compile" => [
    "pycc Roadmap",
    "Current delivery status",
    "v0.1 acceptance checklist"
  ],
  "ci-build-test-coverage-100" => [
    "pycc Roadmap",
    "Current delivery status",
    "v0.1 acceptance checklist"
  ],
  "conformance-fib-mandelbrot-tier1" => [
    "pycc Roadmap",
    "Current delivery status",
    "v0.1 acceptance checklist"
  ],
  "check-throughput-1k-loc-75ms" => [
    "pycc Roadmap",
    "Current delivery status",
    "v0.1 acceptance checklist"
  ],
  "cli-spec-diagnostic-match" => [
    "pycc Roadmap",
    "Current delivery status",
    "v0.1 acceptance checklist"
  ],
  "readme-coverage-badge-bound" => [
    "pycc Roadmap",
    "Current delivery status",
    "v0.1 acceptance checklist"
  ]
}.freeze
# Historical audit-fixture digest. The public policy no longer accepts it.
D51_PAIRED_PERF_CI_WORKFLOW_SHA256 =
  "4b1d11afba108745a2bc375e3447d92ecde843376c3bea95ab32f76b3fc53249"
# Historical audit-fixture digest. The public policy no longer accepts it.
D56_SOURCE_AWARE_PERF_CI_WORKFLOW_SHA256 =
  "c696da18f4f8b876d4398c43f94fe574e870579badd84cf579fbe91fbd9d7b4b"
# Historical audit-fixture digest. The public policy no longer accepts it.
D62_REPLICATED_SOURCE_AWARE_PERF_CI_WORKFLOW_SHA256 =
  "a5135f7a8ebe2b0c0924ad026612ef9c90ade105c9a4fd484f803e10cf5b5c8d"
# Historical audit-fixture digest. The public policy no longer accepts it.
D80_CONFORMANCE_ORACLE_CI_WORKFLOW_SHA256 =
  "17611d861d10c34d6ccebbf21bc82d8dfaf006b969bb2fe1e12d57b9e9c81234"
# D-084: PR-6, Task 7's `pycc check` absolute-throughput-floor CI step
# added to build-test-coverage, after the cargo-test step so it is never
# the first exec of the freshly-linked binary in that job -- see D-084's
# text. Staged onto main first as D80's coexisting sibling by a separate
# stage PR, required by the pull_request_target audit's base-branch-only
# checker trust boundary (same mechanism D-080 already needed -- see
# D-080's "Staging note"); this activation commit retires D80 now that
# the digest below is already authorized.
# D-099 later retired this digest from the public allowlist; it remains here
# only as historical audit evidence and as D-099's exact pre-cache baseline.
D84_THROUGHPUT_FLOOR_CI_WORKFLOW_SHA256 =
  "d0e01df560e32fcd51b6092a8c75dfe4ac270137838907711b37cf043278b516"
# D-090: superseded before activation, never live. This digest covered
# only PR-8, Task 5's release-mode `pycc_rt` build step, added to
# `build-test-coverage` and every `native-build-test` leg. Caught before
# any activation PR shipped it: that content was missing the coverage
# sandbox's own release build (see D-091), so `build-test-coverage`'s
# isolated `nobody` run would have failed `tests/pycc_toml_release_
# default.rs` the moment this digest went live. Historical audit-fixture
# digest only -- the public policy no longer accepts it.
D90_RELEASE_PYCC_RT_CI_WORKFLOW_SHA256 =
  "67c04c8b2dcf8c93fff9f68535a712b942c7b559451b9f0745c12baa9d38ae48"
# D-091: composed correction/extension of D-090's own staged-but-never-
# activated content, discovered while opening PR-8's real pull request
# against `main`. Independent fixes, bundled into one digest since all
# must land before PR-8's `ci.yml` can activate correctly:
#
# 1. `frontend-perf-measure`'s "Verify exact benchmark revisions" step
#    previously hard-required root Cargo.toml/Cargo.lock (and, via a
#    separate digest, every crate's own Cargo.toml/build.rs) to be
#    byte-identical between the predecessor and candidate commits,
#    aborting the job on any diff. That conflated two different
#    guarantees: the benchmark corpus and toolchain (`benches`,
#    `rust-toolchain*`, `.cargo`) measure genuinely incomparable work if
#    they differ, so those stay a hard abort -- but a dependency-manifest
#    change is just another kind of "did the code that produces the
#    benchmarked binary change," already handled correctly by the
#    existing `executable_inputs_equal` classification and its real 2%
#    regression threshold for src/crates changes. A dependency `pycc
#    check` never invokes at runtime cannot confound the comparison, and
#    if a manifest change did somehow shift timing, the same strict
#    threshold already applied to any other executable-input change
#    would still catch it -- so this does not weaken the gate's power to
#    catch real regressions, it only stops aborting before attempting the
#    comparison at all. Moves Cargo.toml/Cargo.lock into that
#    classification instead.
# 2. A separate, standalone check (not part of `contract_paths` above,
#    and not removed by (1)) keeps root Cargo.toml's bench-defining tail
#    ([dev-dependencies] onward, i.e. [dev-dependencies] plus [[bench]]
#    today) a hard abort: nothing in that region reaches the benchmarked
#    binary, so relaxing it would let a bumped `criterion` or rewritten
#    `[[bench]]` target pass as a faster compiler rather than a faster
#    measuring apparatus. Two invariants are asserted so a future
#    manifest reorder cannot silently widen or narrow the pinned region:
#    every section after [dev-dependencies] must be [[bench]] (else
#    something unrelated could be swallowed into the softer
#    classification), and every [[bench]] in the file must be inside
#    that tail (else moving [[bench]] above [dev-dependencies] could
#    move it out of the pinned region). `Cargo.lock`'s own bench-tooling
#    entries are not isolated by this check and remain in the soft
#    classification -- an accepted, documented residual, not an
#    oversight.
# 3. Root-level `build.rs` (not present today, but previously covered by
#    the removed `**/build.rs` digest for any crate) is added to the
#    `executable_inputs_equal` classification loop, guarded the same way
#    `contract_paths` already guards nonexistent paths, so Cargo silently
#    picking up a future root build script cannot go unclassified.
#    Per-crate `build.rs` stays covered by the existing whole-directory
#    `crates` diff.
# 4. `build-test-coverage`'s "Hard coverage gate" step builds `pycc_rt`
#    for its cross-compile target and in debug profile inside its
#    isolated `nobody` sandbox, but never in release profile -- D-090's
#    own release-profile build step ran only later, outside that
#    sandbox, after coverage measurement. `tests/pycc_toml_release_
#    default.rs`'s non-ignored test needs a release-built `pycc_rt` to
#    exist, so it would fail every time coverage runs. Fix: build
#    `pycc_rt --release` inside the same isolated sandbox, before
#    `llvm-cov` runs -- landing at `$ISOLATED_ROOT/target/release`, which
#    `$GITHUB_WORKSPACE/target`'s own symlink already exposes at exactly
#    the `env!("CARGO_MANIFEST_DIR")`-relative path
#    `find_pycc_rt_lib_dir` looks up.
#
# Items 2 (bench-manifest tail) and 3 (build.rs) were flagged by GitHub's
# automated review (`chatgpt-codex-connector`) on PR #189 before merge and
# folded into this same digest rather than filed as follow-ups.
#
# Pre-D100 staged fixture retained as audit evidence. D-099 activated on
# `main` before this PR-8 branch's own merge, briefly retiring this digest;
# D-100 (below) composes these changes with D-099's cache boundary into the
# digest that actually activates.
D91_RELAX_FRONTEND_PERF_MANIFEST_CI_WORKFLOW_SHA256 =
  "f28a428d1e54e12e16bc180d8b4656c5acc3cd04333cd413036066d4abfd1747"
# D-099: active D84 successor plus a Windows-only vcpkg binary-cache
# restore/save boundary for D-027's libxml2 build. The exact cache key binds
# LLVM_VERSION, runner OS/architecture, the hosted runner image version,
# x64-windows-static-md, and the image's vcpkg commit. Pull requests restore
# only; an exact-key miss is saved only by a trusted push to main. D84 and
# the pre-D100 D91 digest were retired from the public allowlist when this
# activated; D-100 (below) composes this decision's own cache boundary with
# D-091's changes under a new digest.
D99_VCPKG_LIBXML2_CACHE_CI_WORKFLOW_SHA256 =
  "f8656c7a525fe8775f90c5afe0950b4706eb043ef6f78f6b66e1f50146fc5366"
# D-100: D-099 (Windows vcpkg libxml2 cache, unrelated to PR-8) activated on
# `main` while this PR-8 branch still carried D-091's own changes (release-
# mode `pycc_rt` build step, relaxed `frontend-perf-measure` manifest
# classification), retiring D-091's digest to audit-only status before PR-8
# could merge. This composes D-091's changes with D-099's cache boundary
# (disjoint regions of `ci.yml` -- D-091 touches `build-test-coverage`/every
# `native-build-test` leg's release-`pycc_rt` step plus `frontend-perf-
# measure`; D-099 touches only `native-build-test (windows-latest)`'s LLVM-
# install section) into the one digest PR-8 actually activates. D-100's own
# digest also needed its own separate stage PR (#231) before `main`'s
# base-owned audit would recognize it -- staging it here, alongside active
# D99, was the fix; this array now holds only the final, activated D-100
# entry since PR-8's own merge is that activation.
D100_COMPOSE_D91_D99_CI_WORKFLOW_SHA256 =
  "6b502ae3cabe0ab1d5a6d65ceffc0490c1f49f7d4d37090acdb460ce51dc9b47"
# Active (2026-08-01, D-112): ubuntu-latest frontend-perf-measure/gate.
# Coexists with D100 until the ci.yml activation PR retires D100 --
# mirrors D-090/D-091's own coexist-then-retire precedent for this array,
# needed because the audit's base-owned checker must already accept this
# digest before any PR can change ci.yml's live bytes to match it.
D112_UBUNTU_FRONTEND_PERF_CI_WORKFLOW_SHA256 =
  "bd92a9b715f67cd708bbc5b8fdafd57957a1ad5a201bc95902e519b0b2692bfc"
# Staged (D-114, round 3): the D112-shaped ci.yml plus an explicit "7.0"
# threshold argument on the "Compare exact predecessor and candidate"
# step -- byte-identical to the already-registered, retained historical
# fixture tests/fixtures/d114-frontend-perf-threshold-ci.yml (main has not
# touched this region of ci.yml since that fixture was first reviewed),
# so no new fixture file or manifest target is needed here, only this
# digest re-entering the accepted array. Coexists with D100 and D112
# until a later round retires them, mirroring this array's own
# established coexist-then-retire precedent. Not yet active: this array
# entry only authorizes the shape for a future ci.yml activation (a
# later, separate round) -- the live ci.yml is untouched by this round.
D114_FRONTEND_PERF_THRESHOLD_CI_WORKFLOW_SHA256 =
  "0176d030004f8be82c5148e86e93df27a1cb287a1b0f34aff1dd10aa36b986f2"
# Issue #229 (Phase 3): the D114-shaped ci.yml plus a new pages-performance
# job (hermetic Lighthouse 12.8.2 budget gate) and its ci-gate.needs /
# fail-step wiring. Coexists with D100/D112/D114 until a later round
# retires them, mirroring this array's own coexist-then-retire precedent.
# Not yet active: this array entry only authorizes the shape for a future
# ci.yml activation (Phase 3, PR 5) -- the live ci.yml is untouched by
# this staging round.
D229_PAGES_PERFORMANCE_CI_WORKFLOW_SHA256 =
  "42a87e33aebc82ab316b0969a793fd5d4746c4d0bf324201c142e543b69f916f"
# Issue #199: the D229-shaped ci.yml plus a new pages-accessibility job
# (hermetic Lighthouse 12.8.2 accessibility gate + ARIA conformance +
# reduced-motion evaluator) and its ci-gate.needs / fail-step wiring.
# Coexists with D100/D112/D114/D229 until a later round retires them,
# mirroring this array's own coexist-then-retire precedent.  Not yet
# active: this array entry only authorizes the shape for a future ci.yml
# activation (Merge 2 of #199) -- the live ci.yml is untouched by this
# staging round.
D199_PAGES_ACCESSIBILITY_CI_WORKFLOW_SHA256 =
  "b96af8b0aa4d3d7ce2509174f071a168dc53f2aac31372b75c0bdda20fc390b8"
# Issue #211: the D199-shaped ci.yml plus a new step in build-test-coverage
# that runs scripts/check_readme_coverage_badge.rb and its test suite, binding
# the README coverage badge to the enforced CI contract. Coexists with
# D100/D112/D114/D229/D199 until a later round retires them, mirroring this
# array's own coexist-then-retire precedent. Not yet active: this array entry
# only authorizes the shape for a future ci.yml activation (Merge 2 of #211)
# -- the live ci.yml is untouched by this staging round.
D211_COVERAGE_BADGE_BINDING_CI_WORKFLOW_SHA256 =
  "a22d8845e4f755bfc50e75e0bb6cc3ae285da7c9aea35ced95bf7a83c39c4b46"
# Python 3.14.7 oracle refresh: the active D211 workflow with both Tier-1
# setup-python pins and their exact-version assertions advanced from 3.14.6
# to 3.14.7. D-103 requires this digest to become base-owned before the
# reviewed workflow bytes can be activated; D-012's 3.14 language gate is
# unchanged.
PY3147_ORACLE_CI_WORKFLOW_SHA256 =
  "d1c4ab9a927723c37809c4621afffcc4b15ecde52a9706cc0951376ca1faf725"
# D-171 change-aware routing successor. This exact staged workflow adds the
# base-reviewed classifier, cancellation-compatible governance, conditional
# heavy jobs, and the fail-closed ci-gate truth table while retaining every
# existing hard-gate body.
D171_CHANGE_AWARE_CI_WORKFLOW_SHA256 =
  "785e6415aea979ac67563589fb8295f2d7a9ce991f11d269d2054422a527e33f"
REVIEWED_PERF_CI_WORKFLOW_SHA256S = [
  D100_COMPOSE_D91_D99_CI_WORKFLOW_SHA256,
  D112_UBUNTU_FRONTEND_PERF_CI_WORKFLOW_SHA256,
  D114_FRONTEND_PERF_THRESHOLD_CI_WORKFLOW_SHA256,
  D229_PAGES_PERFORMANCE_CI_WORKFLOW_SHA256,
  D199_PAGES_ACCESSIBILITY_CI_WORKFLOW_SHA256,
  D211_COVERAGE_BADGE_BINDING_CI_WORKFLOW_SHA256,
  PY3147_ORACLE_CI_WORKFLOW_SHA256,
  D171_CHANGE_AWARE_CI_WORKFLOW_SHA256
].freeze
PINNED_CHECKOUT_ACTION =
  "actions/checkout@d23441a48e516b6c34aea4fa41551a30e30af803"
PINNED_ARTIFACT_UPLOAD_ACTION =
  "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02"
PINNED_ARTIFACT_DOWNLOAD_ACTION =
  "actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093"
PAIRED_PERF_CHECKER_SHA256 =
  "257d37974b1d862de4df9561a9f37b05a125af0404aea7a6f0861271e5fbc56b"
PAIRED_PERF_CHECKER_TEST_SHA256 =
  "5c45928a89f099d6e175a1d30bccbef59c58501d89cac252963e84b9e00952c5"
D56_PERF_CHECKER_SHA256 =
  "55ce7259ff164a43a98187cb7b611794a8417dc3dfddf3f4fb689776ab83adcb"
D56_PERF_CHECKER_TEST_SHA256 =
  "b5ccd35af90dcff6f9fff30ec9075ab9d780694fc9941954913f5bd4407b2b34"
REPLICATED_PERF_CHECKER_SHA256 =
  "d296e48859b1813685f06244c6d401119de57ec1842fe7c47d8ecbbdb1318c06"
REPLICATED_PERF_CHECKER_TEST_SHA256 =
  "bdda7c21cf96d35ee00af6893493b9f84025b35998e5a4bcba88019604757b10"
PAIRED_PERF_CHECKER_VERIFY_SCRIPT = <<~SHELL.strip
  printf '%s  %s\\n' \\
    #{PAIRED_PERF_CHECKER_SHA256} \\
    scripts/check_paired_perf_regression.rb \\
    #{PAIRED_PERF_CHECKER_TEST_SHA256} \\
    scripts/test_check_paired_perf_regression.rb |
    shasum -a 256 --check
SHELL
D56_PERF_CHECKER_VERIFY_SCRIPT = <<~SHELL.strip
  printf '%s  %s\\n' \\
    #{D56_PERF_CHECKER_SHA256} \\
    scripts/check_source_aware_perf_regression.rb \\
    #{D56_PERF_CHECKER_TEST_SHA256} \\
    scripts/test_check_source_aware_perf_regression.rb |
    shasum -a 256 --check
SHELL
REPLICATED_PERF_CHECKER_VERIFY_SCRIPT = <<~SHELL.strip
  printf '%s  %s\\n' \\
    #{REPLICATED_PERF_CHECKER_SHA256} \\
    scripts/check_replicated_paired_perf_regression.rb \\
    #{REPLICATED_PERF_CHECKER_TEST_SHA256} \\
    scripts/test_check_replicated_paired_perf_regression.rb |
    shasum -a 256 --check
SHELL
PAIRED_PERF_PREDECESSOR_SCRIPT = <<~'SHELL'.strip
  set -euo pipefail
  case "$GITHUB_EVENT_NAME" in
    pull_request)
      predecessor_sha="$PR_BASE_SHA"
      ;;
    push)
      predecessor_sha="$PUSH_BEFORE_SHA"
      ;;
    *)
      echo "cannot resolve a performance predecessor for event $GITHUB_EVENT_NAME" >&2
      exit 1
      ;;
  esac
  if [ -z "$predecessor_sha" ] ||
     [ "$predecessor_sha" = "0000000000000000000000000000000000000000" ]; then
    echo "cannot resolve the exact performance predecessor SHA" >&2
    exit 1
  fi
  printf 'sha=%s\n' "$predecessor_sha" >> "$GITHUB_OUTPUT"
SHELL
PAIRED_PERF_VERIFY_REVISIONS_SCRIPT = <<~'SHELL'.strip
  set -euo pipefail
  test "$(git -C previous rev-parse HEAD)" = "$EXPECTED_PREDECESSOR_SHA"
  test "$(git -C current rev-parse HEAD)" = "$EXPECTED_CURRENT_SHA"
  contract_paths=(
    benches
    Cargo.toml
    Cargo.lock
    rust-toolchain.toml
    rust-toolchain
    .cargo
  )
  for contract_path in "${contract_paths[@]}"; do
    previous_path="previous/$contract_path"
    current_path="current/$contract_path"
    if [ ! -e "$previous_path" ] && [ ! -L "$previous_path" ] &&
       [ ! -e "$current_path" ] && [ ! -L "$current_path" ]; then
      continue
    fi
    git diff --no-index --exit-code -- "$previous_path" "$current_path"
  done
  local_contract_digest() {
    checkout="$1"
    (
      cd "$checkout"
      git ls-files -z -- \
        ':(glob)crates/**/Cargo.toml' \
        ':(glob)**/build.rs' |
        while IFS= read -r -d '' relative_path; do
          file_digest="$(/usr/bin/shasum -a 256 "$relative_path" |
            /usr/bin/awk '{print $1}')"
          printf '%s\0%s\0' "$relative_path" "$file_digest"
        done
    ) | /usr/bin/shasum -a 256 | /usr/bin/awk '{print $1}'
  }
  test "$(local_contract_digest previous)" = \
    "$(local_contract_digest current)"
SHELL
D56_EXECUTABLE_INPUT_IDENTITY_SCRIPT = <<~'SHELL'.strip
  set -euo pipefail
  executable_inputs_equal=true
  for executable_path in src crates; do
    if git diff --no-index --no-ext-diff --no-textconv --quiet -- \
      "previous/$executable_path" "current/$executable_path"; then
      continue
    else
      diff_status="$?"
      if [ "$diff_status" -ne 1 ]; then
        echo "could not compare executable benchmark input $executable_path" >&2
        exit "$diff_status"
      fi
      executable_inputs_equal=false
    fi
  done
  printf 'executable_inputs_equal=%s\n' \
    "$executable_inputs_equal" >> "$GITHUB_OUTPUT"
SHELL
PAIRED_PERF_PREVIOUS_BENCHMARK_SCRIPT = <<~'SHELL'.strip
  set -euo pipefail
  previous_target="$RUNNER_TEMP/pycc-paired-perf-previous"
  (
    cd previous
    CARGO_TARGET_DIR="$previous_target" \
      cargo bench --locked --bench check_bench -- --save-baseline paired
  )
  previous_timing="$previous_target/criterion/pycc_check_frontend_fixture/paired/estimates.json"
  timing_dir="$(mktemp -d "$RUNNER_TEMP/pycc-previous-timing.XXXXXX")"
  cp "$previous_timing" "$timing_dir/estimates.json"
  printf 'timing_path=%s\n' "$timing_dir/estimates.json" >> "$GITHUB_OUTPUT"
SHELL
PAIRED_PERF_CURRENT_BENCHMARK_SCRIPT = <<~'SHELL'.strip
  set -euo pipefail
  current_target="$RUNNER_TEMP/pycc-paired-perf-current"
  (
    cd current
    CARGO_TARGET_DIR="$current_target" \
      cargo bench --locked --bench check_bench -- --save-baseline paired
  )
  current_timing="$current_target/criterion/pycc_check_frontend_fixture/paired/estimates.json"
  timing_dir="$(mktemp -d "$RUNNER_TEMP/pycc-current-timing.XXXXXX")"
  cp "$current_timing" "$timing_dir/estimates.json"
  printf 'timing_path=%s\n' "$timing_dir/estimates.json" >> "$GITHUB_OUTPUT"
SHELL
PAIRED_PERF_ARTIFACT_ID_REQUIRE_SCRIPT = <<~'SHELL'.strip
  set -euo pipefail
  for artifact_id in "$PREVIOUS_ARTIFACT_ID" "$CURRENT_ARTIFACT_ID"; do
    case "$artifact_id" in
      ""|*[!0-9]*)
        echo "paired frontend timing has an invalid artifact identity" >&2
        exit 1
        ;;
    esac
  done
  if [ "$PREVIOUS_ARTIFACT_ID" = "$CURRENT_ARTIFACT_ID" ]; then
    echo "paired frontend timings must have distinct artifact identities" >&2
    exit 1
  fi
SHELL
D56_EXECUTABLE_INPUT_IDENTITY_REQUIRE_SCRIPT = <<~'SHELL'.strip
  case "$EXECUTABLE_INPUTS_EQUAL" in
    true|false) ;;
    *)
      echo "paired frontend timing has invalid executable-input identity" >&2
      exit 1
      ;;
  esac
SHELL
PAIRED_PERF_REQUIRE_SCRIPT = <<~'SHELL'.strip
  set -euo pipefail
  timing_root="target/criterion/pycc_check_frontend_fixture"
  for revision in previous current; do
    timing="$timing_root/$revision/estimates.json"
    if [ ! -f "$timing" ] || [ -L "$timing" ]; then
      echo "paired frontend timing is missing $revision/estimates.json" >&2
      exit 1
    fi
  done
  file_count="$(find "$timing_root" -type f | wc -l | tr -d '[:space:]')"
  if [ "$file_count" -ne 2 ] || find "$timing_root" -type l | grep -q .; then
    echo "paired frontend timing must contain exactly two regular files" >&2
    exit 1
  fi
SHELL
PAIRED_PERF_COMPARE_SCRIPT = <<~'SHELL'.strip
  ruby scripts/check_paired_perf_regression.rb \
    target/criterion/pycc_check_frontend_fixture/current/estimates.json \
    target/criterion/pycc_check_frontend_fixture/previous/estimates.json
SHELL
D56_PERF_COMPARE_SCRIPT = <<~'SHELL'.strip
  ruby scripts/check_source_aware_perf_regression.rb \
    target/criterion/pycc_check_frontend_fixture/current/estimates.json \
    target/criterion/pycc_check_frontend_fixture/previous/estimates.json \
    "$EXECUTABLE_INPUTS_EQUAL"
SHELL
REPLICATED_PERF_PREVIOUS_BENCHMARK_SCRIPT = <<~'SHELL'.strip
  set -euo pipefail
  previous_target="$RUNNER_TEMP/pycc-paired-perf-previous"
  timing_dir="$(mktemp -d "$RUNNER_TEMP/pycc-previous-timing.XXXXXX")"
  for round in 1 2 3 4 5; do
    baseline="replicate-$round"
    (
      cd previous
      CARGO_TARGET_DIR="$previous_target" \
        cargo bench --locked --bench check_bench -- --save-baseline "$baseline"
    )
    previous_timing="$previous_target/criterion/pycc_check_frontend_fixture/$baseline/estimates.json"
    cp "$previous_timing" "$timing_dir/round-$round.json"
  done
  printf 'timing_path=%s\n' "$timing_dir" >> "$GITHUB_OUTPUT"
SHELL
REPLICATED_PERF_CURRENT_BENCHMARK_SCRIPT = <<~'SHELL'.strip
  set -euo pipefail
  current_target="$RUNNER_TEMP/pycc-paired-perf-current"
  timing_dir="$(mktemp -d "$RUNNER_TEMP/pycc-current-timing.XXXXXX")"
  for round in 1 2 3 4 5; do
    baseline="replicate-$round"
    (
      cd current
      CARGO_TARGET_DIR="$current_target" \
        cargo bench --locked --bench check_bench -- --save-baseline "$baseline"
    )
    current_timing="$current_target/criterion/pycc_check_frontend_fixture/$baseline/estimates.json"
    cp "$current_timing" "$timing_dir/round-$round.json"
  done
  printf 'timing_path=%s\n' "$timing_dir" >> "$GITHUB_OUTPUT"
SHELL
REPLICATED_PERF_REQUIRE_SCRIPT = <<~'SHELL'.strip
  set -euo pipefail
  timing_root="target/criterion/pycc_check_frontend_fixture"
  for revision in previous current; do
    for round in 1 2 3 4 5; do
      timing="$timing_root/$revision/round-$round.json"
      if [ ! -f "$timing" ] || [ -L "$timing" ]; then
        echo "replicated frontend timing is missing $revision/round-$round.json" >&2
        exit 1
      fi
    done
  done
  file_count="$(find "$timing_root" -type f | wc -l | tr -d '[:space:]')"
  entry_count="$(find "$timing_root" -mindepth 1 | wc -l | tr -d '[:space:]')"
  if [ "$file_count" -ne 10 ] || [ "$entry_count" -ne 12 ] || \
      find "$timing_root" -type l | grep -q .; then
    echo "replicated frontend timing must contain exactly two directories and ten regular files" >&2
    exit 1
  fi
SHELL
REPLICATED_PERF_COMPARE_SCRIPT = <<~'SHELL'.strip
  ruby scripts/check_replicated_paired_perf_regression.rb \
    target/criterion/pycc_check_frontend_fixture/current \
    target/criterion/pycc_check_frontend_fixture/previous \
    "$EXECUTABLE_INPUTS_EQUAL"
SHELL
# D-091: root Cargo.toml/Cargo.lock dropped from the hard-abort contract,
# except for the bench-defining tail ([dev-dependencies] onward) fingerprinted
# below -- see D91_EXECUTABLE_INPUT_IDENTITY_SCRIPT below, which classifies
# everything else instead.
D91_VERIFY_REVISIONS_SCRIPT = <<~'SHELL'.strip
  set -euo pipefail
  test "$(git -C previous rev-parse HEAD)" = "$EXPECTED_PREDECESSOR_SHA"
  test "$(git -C current rev-parse HEAD)" = "$EXPECTED_CURRENT_SHA"
  contract_paths=(
    benches
    rust-toolchain.toml
    rust-toolchain
    .cargo
  )
  for contract_path in "${contract_paths[@]}"; do
    previous_path="previous/$contract_path"
    current_path="current/$contract_path"
    if [ ! -e "$previous_path" ] && [ ! -L "$previous_path" ] &&
       [ ! -e "$current_path" ] && [ ! -L "$current_path" ]; then
      continue
    fi
    git diff --no-index --exit-code -- "$previous_path" "$current_path"
  done
  # D-091: root Cargo.toml's bench-defining tail ([dev-dependencies]
  # onward, which today is exactly [dev-dependencies] plus [[bench]])
  # stays part of this hard contract even though the rest of the
  # manifest is reclassified below -- see "Classify executable
  # benchmark inputs"' own comment. Nothing below [dev-dependencies]
  # reaches the built pycc binary the benchmark times, so relaxing
  # it would let a faster measuring apparatus (a bumped criterion, a
  # rewritten [[bench]] target) pass as a faster compiler. This is a
  # standalone check, not an addition to contract_paths above: it
  # extracts and diffs a sub-region of one file rather than
  # requiring a whole path byte-identical. Two invariants are
  # asserted, not assumed, so a future manifest edit cannot widen
  # or narrow the pinned region without a loud abort: (1)
  # [dev-dependencies] exists and only [[bench]] follows it, so an
  # unrelated section landing after [dev-dependencies] cannot be
  # silently swallowed into the pinned tail; (2) every [[bench]]
  # section in the file is inside that tail, so reordering
  # [[bench]] above [dev-dependencies] cannot silently move it out
  # of the pinned region and into the softer reclassification below.
  for repo_dir in previous current; do
    manifest="$repo_dir/Cargo.toml"
    if ! grep -q '^\[dev-dependencies\]$' "$manifest"; then
      echo "$manifest has no [dev-dependencies] section; bench-manifest fingerprint invariant violated" >&2
      exit 1
    fi
    extra_headers="$(awk '
      /^\[dev-dependencies\]$/ { found = 1 }
      found && /^\[/ && !/^\[dev-dependencies\]$/ && !/^\[\[bench\]\]$/ { print }
    ' "$manifest")"
    if [ -n "$extra_headers" ]; then
      echo "$manifest has unexpected section(s) after [dev-dependencies]: $extra_headers" >&2
      exit 1
    fi
    whole_bench_count="$(grep -c '^\[\[bench\]\]$' "$manifest" || true)"
    tail_bench_count="$(awk '/^\[dev-dependencies\]$/,0' "$manifest" | grep -c '^\[\[bench\]\]$' || true)"
    if [ "$whole_bench_count" != "$tail_bench_count" ]; then
      echo "$manifest has a [[bench]] section outside its [dev-dependencies]-onward tail; bench-manifest fingerprint invariant violated" >&2
      exit 1
    fi
  done
  diff <(awk '/^\[dev-dependencies\]$/,0' previous/Cargo.toml) \
       <(awk '/^\[dev-dependencies\]$/,0' current/Cargo.toml)
SHELL
D91_EXECUTABLE_INPUT_IDENTITY_SCRIPT = <<~'SHELL'.strip
  set -euo pipefail
  executable_inputs_equal=true
  for executable_path in src crates Cargo.toml Cargo.lock build.rs; do
    previous_path="previous/$executable_path"
    current_path="current/$executable_path"
    if [ ! -e "$previous_path" ] && [ ! -L "$previous_path" ] &&
       [ ! -e "$current_path" ] && [ ! -L "$current_path" ]; then
      continue
    fi
    if git diff --no-index --no-ext-diff --no-textconv --quiet -- \
      "$previous_path" "$current_path"; then
      continue
    else
      diff_status="$?"
      if [ "$diff_status" -ne 1 ]; then
        echo "could not compare executable benchmark input $executable_path" >&2
        exit "$diff_status"
      fi
      executable_inputs_equal=false
    fi
  done
  printf 'executable_inputs_equal=%s\n' \
    "$executable_inputs_equal" >> "$GITHUB_OUTPUT"
SHELL
PAIRED_PERF_MEASURE_STEPS = [
  {
    "name" => "Resolve exact predecessor",
    "id" => "predecessor",
    "env" => {
      "PR_BASE_SHA" => "${{ github.event.pull_request.base.sha }}",
      "PUSH_BEFORE_SHA" => "${{ github.event.before }}"
    },
    "run" => PAIRED_PERF_PREDECESSOR_SCRIPT
  },
  {
    "name" => "Check out exact predecessor",
    "uses" => PINNED_CHECKOUT_ACTION,
    "with" => {
      "persist-credentials" => "false",
      "ref" => "${{ steps.predecessor.outputs.sha }}",
      "path" => "previous"
    }
  },
  {
    "name" => "Check out candidate",
    "uses" => PINNED_CHECKOUT_ACTION,
    "with" => {
      "persist-credentials" => "false",
      "ref" => "${{ github.sha }}",
      "path" => "current"
    }
  },
  {
    "name" => "Verify exact benchmark revisions",
    "env" => {
      "EXPECTED_PREDECESSOR_SHA" => "${{ steps.predecessor.outputs.sha }}",
      "EXPECTED_CURRENT_SHA" => "${{ github.sha }}"
    },
    "run" => PAIRED_PERF_VERIFY_REVISIONS_SCRIPT
  },
  {
    "name" => "Show pinned toolchain",
    "run" => "rustup show"
  },
  {
    "name" => "Install LLVM 22 (D-015)",
    "run" => "brew install llvm@22"
  },
  {
    "name" => "Export LLVM_SYS_221_PREFIX",
    "run" => 'echo "LLVM_SYS_221_PREFIX=$(brew --prefix llvm@22)" >> "$GITHUB_ENV"'
  },
  {
    "name" => "Benchmark exact predecessor",
    "id" => "previous_benchmark",
    "run" => PAIRED_PERF_PREVIOUS_BENCHMARK_SCRIPT
  },
  {
    "name" => "Upload sealed predecessor frontend timing",
    "id" => "previous_upload",
    "uses" => PINNED_ARTIFACT_UPLOAD_ACTION,
    "with" => {
      "name" => "frontend-perf-previous",
      "path" => "${{ steps.previous_benchmark.outputs.timing_path }}",
      "if-no-files-found" => "error",
      "retention-days" => "90"
    }
  },
  {
    "name" => "Benchmark exact candidate",
    "id" => "current_benchmark",
    "run" => PAIRED_PERF_CURRENT_BENCHMARK_SCRIPT
  },
  {
    "name" => "Upload candidate frontend timing",
    "id" => "current_upload",
    "uses" => PINNED_ARTIFACT_UPLOAD_ACTION,
    "with" => {
      "name" => "frontend-perf-current",
      "path" => "${{ steps.current_benchmark.outputs.timing_path }}",
      "if-no-files-found" => "error",
      "retention-days" => "90"
    }
  }
].freeze
PAIRED_PERF_GATE_STEPS = [
  {
    "name" => "Check out only the reviewed performance checker",
    "uses" => PINNED_CHECKOUT_ACTION,
    "with" => {
      "persist-credentials" => "false",
      "ref" => "${{ needs.frontend-perf-measure.outputs.predecessor_sha }}",
      "sparse-checkout" =>
        "scripts/check_paired_perf_regression.rb\n" \
        "scripts/test_check_paired_perf_regression.rb",
      "sparse-checkout-cone-mode" => "false"
    }
  },
  {
    "name" => "Verify reviewed performance checker",
    "run" => PAIRED_PERF_CHECKER_VERIFY_SCRIPT
  },
  {
    "name" => "Test reviewed performance checker",
    "run" => "ruby scripts/test_check_paired_perf_regression.rb"
  },
  {
    "name" => "Require sealed artifact identities",
    "env" => {
      "PREVIOUS_ARTIFACT_ID" =>
        "${{ needs.frontend-perf-measure.outputs.predecessor_artifact_id }}",
      "CURRENT_ARTIFACT_ID" =>
        "${{ needs.frontend-perf-measure.outputs.current_artifact_id }}"
    },
    "run" => PAIRED_PERF_ARTIFACT_ID_REQUIRE_SCRIPT
  },
  {
    "name" => "Download sealed predecessor frontend timing",
    "uses" => PINNED_ARTIFACT_DOWNLOAD_ACTION,
    "with" => {
      "artifact-ids" =>
        "${{ needs.frontend-perf-measure.outputs.predecessor_artifact_id }}",
      "path" => "target/criterion/pycc_check_frontend_fixture/previous",
      "merge-multiple" => "true"
    }
  },
  {
    "name" => "Download candidate frontend timing",
    "uses" => PINNED_ARTIFACT_DOWNLOAD_ACTION,
    "with" => {
      "artifact-ids" =>
        "${{ needs.frontend-perf-measure.outputs.current_artifact_id }}",
      "path" => "target/criterion/pycc_check_frontend_fixture/current",
      "merge-multiple" => "true"
    }
  },
  {
    "name" => "Require exact predecessor and candidate timing",
    "run" => PAIRED_PERF_REQUIRE_SCRIPT
  },
  {
    "name" => "Compare exact predecessor and candidate",
    "run" => PAIRED_PERF_COMPARE_SCRIPT
  }
].freeze
PAIRED_PERF_MEASURE_JOB = {
  "runs-on" => "macos-14",
  "permissions" => { "contents" => "read" },
  "outputs" => {
    "predecessor_sha" => "${{ steps.predecessor.outputs.sha }}",
    "predecessor_artifact_id" =>
      "${{ steps.previous_upload.outputs.artifact-id }}",
    "current_artifact_id" =>
      "${{ steps.current_upload.outputs.artifact-id }}"
  },
  "steps" => PAIRED_PERF_MEASURE_STEPS
}.freeze
PAIRED_PERF_GATE_JOB = {
  "needs" => "frontend-perf-measure",
  "runs-on" => "macos-14",
  "permissions" => {
    "actions" => "read",
    "contents" => "read"
  },
  "steps" => PAIRED_PERF_GATE_STEPS
}.freeze
D56_SOURCE_AWARE_PERF_MEASURE_STEPS =
  Marshal.load(Marshal.dump(PAIRED_PERF_MEASURE_STEPS)).tap do |steps|
    verify_index = steps.index do |step|
      step["name"] == "Verify exact benchmark revisions"
    end
    steps.insert(
      verify_index + 1,
      {
        "name" => "Classify executable benchmark inputs",
        "id" => "benchmark_contract",
        "run" => D56_EXECUTABLE_INPUT_IDENTITY_SCRIPT
      }
    )
  end.freeze
D56_SOURCE_AWARE_PERF_GATE_STEPS =
  Marshal.load(Marshal.dump(PAIRED_PERF_GATE_STEPS)).tap do |steps|
    checkout = steps.find do |step|
      step["name"] == "Check out only the reviewed performance checker"
    end
    checkout.fetch("with")["sparse-checkout"] =
      "scripts/check_source_aware_perf_regression.rb\n" \
      "scripts/test_check_source_aware_perf_regression.rb"

    verify = steps.find do |step|
      step["name"] == "Verify reviewed performance checker"
    end
    verify["run"] = D56_PERF_CHECKER_VERIFY_SCRIPT

    checker_test = steps.find do |step|
      step["name"] == "Test reviewed performance checker"
    end
    checker_test["run"] =
      "ruby scripts/test_check_source_aware_perf_regression.rb"

    identity_index = steps.index do |step|
      step["name"] == "Require sealed artifact identities"
    end
    steps.insert(
      identity_index + 1,
      {
        "name" => "Require executable-input identity",
        "env" => {
          "EXECUTABLE_INPUTS_EQUAL" =>
            "${{ needs.frontend-perf-measure.outputs.executable_inputs_equal }}"
        },
        "run" => D56_EXECUTABLE_INPUT_IDENTITY_REQUIRE_SCRIPT
      }
    )

    compare = steps.find do |step|
      step["name"] == "Compare exact predecessor and candidate"
    end
    compare["env"] = {
      "EXECUTABLE_INPUTS_EQUAL" =>
        "${{ needs.frontend-perf-measure.outputs.executable_inputs_equal }}"
    }
    compare["run"] = D56_PERF_COMPARE_SCRIPT
  end.freeze
D56_SOURCE_AWARE_PERF_MEASURE_JOB = {
  "runs-on" => "macos-14",
  "permissions" => { "contents" => "read" },
  "outputs" => {
    "predecessor_sha" => "${{ steps.predecessor.outputs.sha }}",
    "predecessor_artifact_id" =>
      "${{ steps.previous_upload.outputs.artifact-id }}",
    "current_artifact_id" =>
      "${{ steps.current_upload.outputs.artifact-id }}",
    "executable_inputs_equal" =>
      "${{ steps.benchmark_contract.outputs.executable_inputs_equal }}"
  },
  "steps" => D56_SOURCE_AWARE_PERF_MEASURE_STEPS
}.freeze
D56_SOURCE_AWARE_PERF_GATE_JOB = {
  "needs" => "frontend-perf-measure",
  "runs-on" => "macos-14",
  "permissions" => {
    "actions" => "read",
    "contents" => "read"
  },
  "steps" => D56_SOURCE_AWARE_PERF_GATE_STEPS
}.freeze
REPLICATED_PERF_MEASURE_STEPS =
  D56_SOURCE_AWARE_PERF_MEASURE_STEPS.map do |step|
    replicated_step = Marshal.load(Marshal.dump(step))
    case replicated_step["name"]
    when "Benchmark exact predecessor"
      replicated_step["run"] = REPLICATED_PERF_PREVIOUS_BENCHMARK_SCRIPT
    when "Benchmark exact candidate"
      replicated_step["run"] = REPLICATED_PERF_CURRENT_BENCHMARK_SCRIPT
    end
    replicated_step
  end.freeze
REPLICATED_PERF_GATE_STEPS =
  D56_SOURCE_AWARE_PERF_GATE_STEPS.map do |step|
    replicated_step = Marshal.load(Marshal.dump(step))
    case replicated_step["name"]
    when "Check out only the reviewed performance checker"
      replicated_step.fetch("with")["sparse-checkout"] =
        "scripts/check_replicated_paired_perf_regression.rb\n" \
        "scripts/test_check_replicated_paired_perf_regression.rb"
    when "Verify reviewed performance checker"
      replicated_step["run"] = REPLICATED_PERF_CHECKER_VERIFY_SCRIPT
    when "Test reviewed performance checker"
      replicated_step["run"] =
        "ruby scripts/test_check_replicated_paired_perf_regression.rb"
    when "Require exact predecessor and candidate timing"
      replicated_step["run"] = REPLICATED_PERF_REQUIRE_SCRIPT
    when "Compare exact predecessor and candidate"
      replicated_step["run"] = REPLICATED_PERF_COMPARE_SCRIPT
    end
    replicated_step
  end.freeze
REPLICATED_PERF_MEASURE_JOB = D56_SOURCE_AWARE_PERF_MEASURE_JOB.merge(
  "steps" => REPLICATED_PERF_MEASURE_STEPS
).freeze
REPLICATED_PERF_GATE_JOB = D56_SOURCE_AWARE_PERF_GATE_JOB.merge(
  "steps" => REPLICATED_PERF_GATE_STEPS
).freeze
D91_RELAX_FRONTEND_PERF_MANIFEST_MEASURE_STEPS =
  Marshal.load(Marshal.dump(REPLICATED_PERF_MEASURE_STEPS)).tap do |steps|
    verify = steps.find { |step| step["name"] == "Verify exact benchmark revisions" }
    verify["run"] = D91_VERIFY_REVISIONS_SCRIPT
    classify =
      steps.find { |step| step["name"] == "Classify executable benchmark inputs" }
    classify["run"] = D91_EXECUTABLE_INPUT_IDENTITY_SCRIPT
  end.freeze
D91_RELAX_FRONTEND_PERF_MANIFEST_MEASURE_JOB = D56_SOURCE_AWARE_PERF_MEASURE_JOB.merge(
  "steps" => D91_RELAX_FRONTEND_PERF_MANIFEST_MEASURE_STEPS
).freeze
D112_UBUNTU_FRONTEND_PERF_MEASURE_STEPS =
  Marshal.load(Marshal.dump(D91_RELAX_FRONTEND_PERF_MANIFEST_MEASURE_STEPS)).tap do |steps|
    llvm_index = steps.index { |step| step["name"] == "Install LLVM 22 (D-015)" }
    raise "expected an existing macOS LLVM-install step to replace" unless llvm_index

    export_index = steps.index { |step| step["name"] == "Export LLVM_SYS_221_PREFIX" }
    raise "expected an existing LLVM_SYS_221_PREFIX export step to remove" unless export_index

    steps.delete_at(export_index)
    steps[llvm_index] = {
      "name" => "Install LLVM 22 (Linux, via apt.llvm.org)",
      "run" => <<~SHELL.strip
        wget https://apt.llvm.org/llvm.sh
        chmod +x llvm.sh
        sudo ./llvm.sh 22
        # llvm.sh's own packages don't pull in Polly's static lib; llvm-sys
        # links it explicitly, so it must be installed separately here.
        sudo apt-get install -y libpolly-22-dev
        echo "LLVM_SYS_221_PREFIX=/usr/lib/llvm-22" >> "$GITHUB_ENV"
      SHELL
    }
  end.freeze
D112_UBUNTU_FRONTEND_PERF_MEASURE_JOB = D91_RELAX_FRONTEND_PERF_MANIFEST_MEASURE_JOB.merge(
  "runs-on" => "ubuntu-latest",
  "steps" => D112_UBUNTU_FRONTEND_PERF_MEASURE_STEPS
).freeze
D112_UBUNTU_FRONTEND_PERF_GATE_JOB = REPLICATED_PERF_GATE_JOB.merge(
  "runs-on" => "ubuntu-latest"
).freeze
# D-114: a one-time, real (not runner-noise) regression from PR-10's `Ty`
# migration (D-109) needs a wider frontend-perf-gate threshold than D-112's
# implicit 2.0% default. This is the same D112_UBUNTU_FRONTEND_PERF_GATE_JOB
# shape with only the comparison step's threshold argument added -- the
# measure job is unchanged, so validate_source_aware_perf_gate_lifecycle's
# D112 branch below is widened to accept either gate-job shape rather than
# introducing a new measure-job branch.
D114_RAISED_THRESHOLD_FRONTEND_PERF_COMPARE_SCRIPT = <<~'SHELL'.strip
  ruby scripts/check_replicated_paired_perf_regression.rb \
    target/criterion/pycc_check_frontend_fixture/current \
    target/criterion/pycc_check_frontend_fixture/previous \
    "$EXECUTABLE_INPUTS_EQUAL" \
    "7.0"
SHELL
D114_RAISED_THRESHOLD_FRONTEND_PERF_GATE_STEPS =
  Marshal.load(Marshal.dump(D112_UBUNTU_FRONTEND_PERF_GATE_JOB.fetch("steps"))).tap do |steps|
    compare = steps.find { |step| step["name"] == "Compare exact predecessor and candidate" }
    raise "expected an existing compare step to raise the threshold on" unless compare

    compare["run"] = D114_RAISED_THRESHOLD_FRONTEND_PERF_COMPARE_SCRIPT
  end.freeze
D114_RAISED_THRESHOLD_FRONTEND_PERF_GATE_JOB = D112_UBUNTU_FRONTEND_PERF_GATE_JOB.merge(
  "steps" => D114_RAISED_THRESHOLD_FRONTEND_PERF_GATE_STEPS
).freeze
PAIRED_PERF_CI_GATE_NEEDS = [
  "build-test-coverage",
  "native-build-test",
  "cross-compile-build",
  "cross-compile-verify",
  "frontend-perf-measure",
  "frontend-perf-gate"
].freeze
PAIRED_PERF_CI_GATE_FAILURE_CONDITION = [
  "needs.build-test-coverage.result != 'success'",
  "needs.native-build-test.result != 'success'",
  "needs.cross-compile-build.result != 'success'",
  "needs.cross-compile-verify.result != 'success'",
  "needs.frontend-perf-measure.result != 'success'",
  "needs.frontend-perf-gate.result != 'success'"
].join(" || ").freeze
PAIRED_PERF_CI_GATE_RUN = <<~'SHELL'.strip
  echo "one or more required jobs did not succeed:"
  echo '${{ toJSON(needs) }}'
  exit 1
SHELL
PAIRED_PERF_CI_GATE_JOB = {
  "needs" => PAIRED_PERF_CI_GATE_NEEDS,
  "if" => "always()",
  "runs-on" => "ubuntu-latest",
  "permissions" => {},
  "steps" => [
    {
      "name" => "Fail unless every required job succeeded",
      "if" => PAIRED_PERF_CI_GATE_FAILURE_CONDITION,
      "run" => PAIRED_PERF_CI_GATE_RUN
    }
  ]
}.freeze
# Issue #229 (Phase 3): the D229 ci-gate shape adds pages-performance to the
# needs list and to the fail step's failure condition, so the aggregate gate
# stays fail-closed when the pages-performance job is required.  This is the
# seven-element successor to PAIRED_PERF_CI_GATE_JOB's six-element shape --
# validate_source_aware_perf_gate_lifecycle accepts either shape so the
# checker recognizes the D229 ci.yml the moment PR 5 activates it, without
# retiring the pre-D229 shape before that activation lands.
D229_PAIRED_PERF_CI_GATE_NEEDS = [
  "build-test-coverage",
  "native-build-test",
  "cross-compile-build",
  "cross-compile-verify",
  "frontend-perf-measure",
  "frontend-perf-gate",
  "pages-performance"
].freeze
D229_PAIRED_PERF_CI_GATE_FAILURE_CONDITION = [
  "needs.build-test-coverage.result != 'success'",
  "needs.native-build-test.result != 'success'",
  "needs.cross-compile-build.result != 'success'",
  "needs.cross-compile-verify.result != 'success'",
  "needs.frontend-perf-measure.result != 'success'",
  "needs.frontend-perf-gate.result != 'success'",
  "needs.pages-performance.result != 'success'"
].join(" || ").freeze
D229_PAIRED_PERF_CI_GATE_JOB = {
  "needs" => D229_PAIRED_PERF_CI_GATE_NEEDS,
  "if" => "always()",
  "runs-on" => "ubuntu-latest",
  "permissions" => {},
  "steps" => [
    {
      "name" => "Fail unless every required job succeeded",
      "if" => D229_PAIRED_PERF_CI_GATE_FAILURE_CONDITION,
      "run" => PAIRED_PERF_CI_GATE_RUN
    }
  ]
}.freeze
# Issue #199: the D229 seven-element ci-gate shape plus a new
# pages-accessibility job (hermetic Lighthouse 12.8.2 accessibility
# gate + ARIA conformance + reduced-motion evaluator).  This
# eight-element shape coexists with the pre-D229 and D229 shapes
# until a later round retires them.  Not yet active: this entry
# only authorizes the shape for a future ci.yml activation (Merge 2
# of #199) -- the live ci.yml is untouched by this staging round.
D199_PAGES_ACCESSIBILITY_CI_GATE_NEEDS = [
  "build-test-coverage",
  "native-build-test",
  "cross-compile-build",
  "cross-compile-verify",
  "frontend-perf-measure",
  "frontend-perf-gate",
  "pages-performance",
  "pages-accessibility"
].freeze
D199_PAGES_ACCESSIBILITY_CI_GATE_FAILURE_CONDITION = [
  "needs.build-test-coverage.result != 'success'",
  "needs.native-build-test.result != 'success'",
  "needs.cross-compile-build.result != 'success'",
  "needs.cross-compile-verify.result != 'success'",
  "needs.frontend-perf-measure.result != 'success'",
  "needs.frontend-perf-gate.result != 'success'",
  "needs.pages-performance.result != 'success'",
  "needs.pages-accessibility.result != 'success'"
].join(" || ").freeze
D199_PAGES_ACCESSIBILITY_CI_GATE_JOB = {
  "needs" => D199_PAGES_ACCESSIBILITY_CI_GATE_NEEDS,
  "if" => "always()",
  "runs-on" => "ubuntu-latest",
  "permissions" => {},
  "steps" => [
    {
      "name" => "Fail unless every required job succeeded",
      "if" => D199_PAGES_ACCESSIBILITY_CI_GATE_FAILURE_CONDITION,
      "run" => PAIRED_PERF_CI_GATE_RUN
    }
  ]
}.freeze
# The pre-D229 six-element ci-gate shape, the D229 seven-element
# shape (with pages-performance), and the D199 eight-element shape
# (with pages-accessibility) are all accepted by
# validate_source_aware_perf_gate_lifecycle.
ACCEPTED_PERF_CI_GATE_JOBS = [
  PAIRED_PERF_CI_GATE_JOB,
  D229_PAIRED_PERF_CI_GATE_JOB,
  D199_PAGES_ACCESSIBILITY_CI_GATE_JOB
].freeze
D171_CONCURRENCY = {
  "group" =>
    "ci-${{ github.event_name == 'pull_request' && github.event.pull_request.number || github.run_id }}",
  "cancel-in-progress" => "${{ github.event_name == 'pull_request' }}"
}.freeze
D171_TOP_LEVEL_KEYS = %w[name on permissions concurrency env jobs].freeze
D171_TRIGGERS = {
  "push" => { "branches" => ["main"] },
  "pull_request" => ""
}.freeze
D171_ENV = {
  "CARGO_LLVM_COV_VERSION" => "0.8.7",
  "LLVM_VERSION" => "22.1.1"
}.freeze
D171_CLASSIFIER_OUTPUTS = {
  "compiler" => "${{ steps.classify.outputs.compiler }}",
  "pages" => "${{ steps.classify.outputs.pages }}",
  "agent" => "${{ steps.classify.outputs.agent }}"
}.freeze
D171_CLASSIFIER_ENV = {
  "PR_BASE_SHA" => "${{ github.event.pull_request.base.sha }}",
  "PR_HEAD_SHA" => "${{ github.event.pull_request.head.sha }}",
  "PUSH_BASE_SHA" => "${{ github.event.before }}",
  "PUSH_HEAD_SHA" => "${{ github.sha }}"
}.freeze
D171_CLASSIFIER_RUN = <<~'SHELL'.strip
  set -euo pipefail
  case "$GITHUB_EVENT_NAME" in
    pull_request)
      base_sha="$PR_BASE_SHA"
      head_sha="$PR_HEAD_SHA"
      ;;
    push)
      base_sha="$PUSH_BASE_SHA"
      head_sha="$PUSH_HEAD_SHA"
      ;;
    *)
      echo "unsupported CI event: $GITHUB_EVENT_NAME" >&2
      exit 1
      ;;
  esac
  for revision in "$base_sha" "$head_sha"; do
    if ! printf '%s\n' "$revision" | grep -Eq '^[0-9a-f]{40}$' ||
       [ "$revision" = "0000000000000000000000000000000000000000" ]; then
      echo "could not resolve exact CI base/head revisions" >&2
      exit 1
    fi
    git cat-file -e "${revision}^{commit}"
  done
  git diff --no-renames --name-only --diff-filter=ACDMRTUXB -z "$base_sha" "$head_sha" |
    python3 -B scripts/classify_ci_changes.py \
      --event-name "$GITHUB_EVENT_NAME" \
      --github-output "$GITHUB_OUTPUT"
SHELL
D171_OPTIONAL_ROUTING = {
  "build-test-coverage" => ["compiler", "classify-changes"],
  "native-build-test" => ["compiler", "classify-changes"],
  "cross-compile-build" => ["compiler", "classify-changes"],
  "cross-compile-verify" =>
    ["compiler", ["classify-changes", "cross-compile-build"]],
  "frontend-perf-measure" => ["compiler", "classify-changes"],
  "frontend-perf-gate" =>
    ["compiler", ["classify-changes", "frontend-perf-measure"]],
  "pages-performance" => ["pages", "classify-changes"],
  "pages-accessibility" => ["pages", "classify-changes"]
}.freeze
D171_CHECKOUT_COUNTS = {
  "classify-changes" => 1,
  "governance" => 1,
  "build-test-coverage" => 1,
  "native-build-test" => 1,
  "cross-compile-build" => 1,
  "frontend-perf-measure" => 2,
  "frontend-perf-gate" => 1,
  "pages-performance" => 1,
  "pages-accessibility" => 1
}.freeze
D171_JOB_NAMES = [
  "classify-changes",
  "governance",
  *D171_OPTIONAL_ROUTING.keys,
  "ci-gate"
].freeze
D171_JOB_BODY_SHA256S = {
  "governance" => "02153e40dd8c808d90a010ecf958b46d51a00ae1f23f1165df93a4236a1514c3",
  "build-test-coverage" => "68c97f28128266224195f091d9992add05186d6bc84ef75c500d92103459af94",
  "native-build-test" => "8f13b95ff869a085e4239360c8d5e16d36ef357cca6e475bae21faa35788fa52",
  "cross-compile-build" => "b0ef76e4a4dee6a0c1255be9bc7b4b0846a54ff786c2ba6a6bce0c83a6d9bedd",
  "cross-compile-verify" => "18cde75ed226966ae96336f6b7898c6bacb26294d070074012fc58e7fe27ae35",
  "frontend-perf-measure" => "c4fc6a1575f8e18480f79a902e0d5e6a9fa0ea0cee88293c25121c8ddf41b962",
  "frontend-perf-gate" => "4db6345a6deb7a8c4ffff31f06aea4d1d7b279b16a7ad4f75a658fae60926bc3",
  "pages-performance" => "609568e141cf60f06d398953f39d43fcf640c7b3aef6866595348685691dfc41",
  "pages-accessibility" => "576c7a1fd01a8924689d7723a271ac4b83ac603a52eec414d9cd5c6eafc34016"
}.freeze
D171_TIER1_MATRIX = [
  { "os" => "ubuntu-latest", "target" => "x86_64-unknown-linux-gnu" },
  { "os" => "ubuntu-24.04-arm", "target" => "aarch64-unknown-linux-gnu" },
  { "os" => "macos-15-intel", "target" => "x86_64-apple-darwin" },
  { "os" => "windows-latest", "target" => "x86_64-pc-windows-msvc" }
].freeze
D171_TIER1_STRATEGY = {
  "fail-fast" => "false",
  "matrix" => { "include" => D171_TIER1_MATRIX }
}.freeze
D171_CI_GATE_NEEDS = [
  "classify-changes",
  "governance",
  *D171_OPTIONAL_ROUTING.keys
].freeze
D171_CI_GATE_FAILURE_CONDITION = [
  "needs.classify-changes.result != 'success'",
  "needs.governance.result != 'success'",
  *D171_CLASSIFIER_OUTPUTS.keys.map do |output|
    "(needs.classify-changes.outputs.#{output} != 'true' && " \
      "needs.classify-changes.outputs.#{output} != 'false')"
  end,
  *D171_OPTIONAL_ROUTING.flat_map do |job_name, (output, _needs)|
    [
      "(needs.classify-changes.outputs.#{output} == 'true' && " \
        "needs.#{job_name}.result != 'success')",
      "(needs.classify-changes.outputs.#{output} == 'false' && " \
        "needs.#{job_name}.result != 'skipped')"
    ]
  end
].join(" || ").freeze
D171_CI_GATE_RUN = <<~'SHELL'.strip
  echo "required CI jobs or exact change routing did not succeed:"
  echo '${{ toJSON(needs) }}'
  exit 1
SHELL
COVERAGE_JOB = "build-test-coverage"
COVERAGE_STEP = "Hard coverage gate — 100% lines + regions (D-014)"
COVERAGE_COMMAND =
  "run_isolated \"$TRUSTED_COV\" llvm-cov --workspace " \
  "--fail-under-lines 100 --fail-under-regions 100"
COVERAGE_SCRIPT = <<~SHELL.strip
  set -euo pipefail
  LLVM_SYS_221_PREFIX_VALUE="$(brew --prefix llvm@22)"
  TRUSTED_CARGO="$(rustup which cargo)"
  TRUSTED_RUSTC="$(rustup which rustc)"
  TRUSTED_RUSTDOC="$(rustup which rustdoc)"
  TRUSTED_COV="/Users/runner/.cargo/bin/cargo-llvm-cov"
  TRUSTED_TOOLCHAIN="$(dirname "$(dirname "$TRUSTED_CARGO")")"
  cd "$RUNNER_TEMP"
  RUSTC="$TRUSTED_RUSTC" RUSTDOC="$TRUSTED_RUSTDOC" "$TRUSTED_CARGO" install cargo-llvm-cov --locked --version "${CARGO_LLVM_COV_VERSION}"
  "$TRUSTED_COV" llvm-cov --version
  sudo chmod o+x /Users/runner /Users/runner/.cargo /Users/runner/.cargo/bin /Users/runner/.rustup /Users/runner/.rustup/toolchains
  sudo chmod -R o+rX "$TRUSTED_TOOLCHAIN"
  sudo chmod o+rx "$TRUSTED_COV"
  ISOLATED_ROOT="$RUNNER_TEMP/pycc-coverage"
  mkdir -p "$ISOLATED_ROOT/home" "$ISOLATED_ROOT/tmp" "$ISOLATED_ROOT/cargo-home" "$ISOLATED_ROOT/target"
  sudo chown -R nobody:nobody "$ISOLATED_ROOT"
  ISOLATED_ENV=(
    "HOME=$ISOLATED_ROOT/home"
    "TMPDIR=$ISOLATED_ROOT/tmp/"
    "CARGO_HOME=$ISOLATED_ROOT/cargo-home"
    "CARGO_TARGET_DIR=$ISOLATED_ROOT/target"
    "CARGO=$TRUSTED_CARGO"
    "RUSTC=$TRUSTED_RUSTC"
    "RUSTDOC=$TRUSTED_RUSTDOC"
    "LLVM_SYS_221_PREFIX=$LLVM_SYS_221_PREFIX_VALUE"
    "PATH=$(dirname "$TRUSTED_CARGO"):/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin"
  )
  run_isolated() {
    sudo -u nobody env -i "${ISOLATED_ENV[@]}" "$@"
  }
  ln -s "$ISOLATED_ROOT/target" "$GITHUB_WORKSPACE/target"
  cd "$GITHUB_WORKSPACE"
  run_isolated "$TRUSTED_CARGO" build --target x86_64-apple-darwin -p pycc_rt
  run_isolated "$TRUSTED_CARGO" build --workspace
  #{COVERAGE_COMMAND}
  rm "$GITHUB_WORKSPACE/target"
  printf 'LLVM_SYS_221_PREFIX=%s\\n' "$LLVM_SYS_221_PREFIX_VALUE" >> "$GITHUB_ENV"
SHELL
# D-091: `build-test-coverage`'s own isolated sandbox additionally builds a
# release-profile `pycc_rt` before `llvm-cov` runs (see D-091's text) --
# `coverage_gate_present?`/`workflow-policy.yml`'s `pull_request_target`
# audit reaches this exact step body under the identical base-branch-only
# trust boundary `REVIEWED_PERF_CI_WORKFLOW_SHA256S` exists for, so this
# needed the same coexist-then-retire treatment as that digest array while
# `ci.yml` still had the pre-D91 shape. v0.2 PR-8's own merge is that later
# activation commit: `ci.yml` now matches the D91 shape exactly, so
# `COVERAGE_SCRIPT`/`TRUSTED_COVERAGE_STEPS` (the pre-D91 shape) are kept
# defined below only as historical audit fixtures, the same way D51/D56/D62/
# D80's own retired constants are kept -- `REVIEWED_COVERAGE_SCRIPTS`/
# `REVIEWED_TRUSTED_COVERAGE_STEPS` accept only the D91 shape now.
D91_COVERAGE_SCRIPT = <<~SHELL.strip
  set -euo pipefail
  LLVM_SYS_221_PREFIX_VALUE="$(brew --prefix llvm@22)"
  TRUSTED_CARGO="$(rustup which cargo)"
  TRUSTED_RUSTC="$(rustup which rustc)"
  TRUSTED_RUSTDOC="$(rustup which rustdoc)"
  TRUSTED_COV="/Users/runner/.cargo/bin/cargo-llvm-cov"
  TRUSTED_TOOLCHAIN="$(dirname "$(dirname "$TRUSTED_CARGO")")"
  cd "$RUNNER_TEMP"
  RUSTC="$TRUSTED_RUSTC" RUSTDOC="$TRUSTED_RUSTDOC" "$TRUSTED_CARGO" install cargo-llvm-cov --locked --version "${CARGO_LLVM_COV_VERSION}"
  "$TRUSTED_COV" llvm-cov --version
  sudo chmod o+x /Users/runner /Users/runner/.cargo /Users/runner/.cargo/bin /Users/runner/.rustup /Users/runner/.rustup/toolchains
  sudo chmod -R o+rX "$TRUSTED_TOOLCHAIN"
  sudo chmod o+rx "$TRUSTED_COV"
  ISOLATED_ROOT="$RUNNER_TEMP/pycc-coverage"
  mkdir -p "$ISOLATED_ROOT/home" "$ISOLATED_ROOT/tmp" "$ISOLATED_ROOT/cargo-home" "$ISOLATED_ROOT/target"
  sudo chown -R nobody:nobody "$ISOLATED_ROOT"
  ISOLATED_ENV=(
    "HOME=$ISOLATED_ROOT/home"
    "TMPDIR=$ISOLATED_ROOT/tmp/"
    "CARGO_HOME=$ISOLATED_ROOT/cargo-home"
    "CARGO_TARGET_DIR=$ISOLATED_ROOT/target"
    "CARGO=$TRUSTED_CARGO"
    "RUSTC=$TRUSTED_RUSTC"
    "RUSTDOC=$TRUSTED_RUSTDOC"
    "LLVM_SYS_221_PREFIX=$LLVM_SYS_221_PREFIX_VALUE"
    "PATH=$(dirname "$TRUSTED_CARGO"):/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin"
  )
  run_isolated() {
    sudo -u nobody env -i "${ISOLATED_ENV[@]}" "$@"
  }
  ln -s "$ISOLATED_ROOT/target" "$GITHUB_WORKSPACE/target"
  cd "$GITHUB_WORKSPACE"
  run_isolated "$TRUSTED_CARGO" build --target x86_64-apple-darwin -p pycc_rt
  run_isolated "$TRUSTED_CARGO" build --workspace
  run_isolated "$TRUSTED_CARGO" build --release -p pycc_rt
  #{COVERAGE_COMMAND}
  rm "$GITHUB_WORKSPACE/target"
  printf 'LLVM_SYS_221_PREFIX=%s\\n' "$LLVM_SYS_221_PREFIX_VALUE" >> "$GITHUB_ENV"
SHELL
REVIEWED_COVERAGE_SCRIPTS = [D91_COVERAGE_SCRIPT].freeze
TRUSTED_COVERAGE_ENV = {
  "CARGO_LLVM_COV_VERSION" => "0.8.7",
  "LLVM_VERSION" => "22.1.1"
}.freeze
TRUSTED_COVERAGE_STEPS = [
  {
    "uses" => "actions/checkout@d23441a48e516b6c34aea4fa41551a30e30af803",
    "with" => { "persist-credentials" => "false" }
  },
  {
    "name" => "Show pinned toolchain",
    "run" => "rustup show"
  },
  {
    "name" => "Install LLVM 22 (D-015)",
    "run" => "brew install llvm@22"
  },
  {
    "name" => "Install llvm-tools-preview",
    "run" => "rustup component add llvm-tools-preview"
  },
  {
    "name" => "Add x86_64-apple-darwin Rust target",
    "run" => "rustup target add x86_64-apple-darwin"
  },
  {
    "name" => COVERAGE_STEP,
    "run" => COVERAGE_SCRIPT
  }
].freeze
D91_TRUSTED_COVERAGE_STEPS =
  (TRUSTED_COVERAGE_STEPS[0..-2] + [
    {
      "name" => COVERAGE_STEP,
      "run" => D91_COVERAGE_SCRIPT
    }
  ]).freeze
REVIEWED_TRUSTED_COVERAGE_STEPS = [D91_TRUSTED_COVERAGE_STEPS].freeze

def yaml_mapping(node, context)
  raise RoadmapEvidenceError, "#{context} must be a mapping" unless node.is_a?(Psych::Nodes::Mapping)

  entries = {}
  node.children.each_slice(2) do |key, value|
    unless key.is_a?(Psych::Nodes::Scalar)
      raise RoadmapEvidenceError, "#{context} contains a non-scalar key"
    end

    if entries.key?(key.value)
      raise RoadmapEvidenceError, "#{context} contains duplicate key #{key.value.inspect}"
    end

    entries[key.value] = value
  end
  entries
end

def yaml_scalar(node, context)
  raise RoadmapEvidenceError, "#{context} must be a scalar" unless node.is_a?(Psych::Nodes::Scalar)

  node.value
end

def yaml_value(node, context)
  case node
  when Psych::Nodes::Scalar
    node.value.strip
  when Psych::Nodes::Mapping
    yaml_mapping(node, context).transform_values do |value|
      yaml_value(value, context)
    end
  when Psych::Nodes::Sequence
    node.children.map { |value| yaml_value(value, context) }
  else
    raise RoadmapEvidenceError, "#{context} contains an unsupported YAML value"
  end
end

def d171_mapping(value, context)
  return value if value.is_a?(Hash)

  raise RoadmapEvidenceError, "#{context} must be a mapping for D-171"
end

def d171_sequence(value, context)
  return value if value.is_a?(Array)

  raise RoadmapEvidenceError, "#{context} must be a sequence for D-171"
end

def d171_require_equal(actual, expected, context)
  return if actual == expected

  raise RoadmapEvidenceError, "#{context} does not match the reviewed D-171 routing"
end

def d171_normalize_expression(expression)
  expression.to_s.gsub(/\s+/, " ").strip
end

def d171_canonical_value(value)
  case value
  when Hash
    value.keys.sort.each_with_object({}) do |key, canonical|
      canonical[key] = d171_canonical_value(value[key])
    end
  when Array
    value.map { |entry| d171_canonical_value(entry) }
  else
    value
  end
end

def d171_canonical_sha256(value)
  Digest::SHA256.hexdigest(JSON.generate(d171_canonical_value(value)))
end

def validate_d171_ci_routing(workflow_text, source)
  stream = Psych.parse_stream(workflow_text, filename: source)
  if stream.children.length != 1 || stream.children.first.root.nil?
    raise RoadmapEvidenceError, "#{source} must contain exactly one YAML document for D-171"
  end

  workflow = yaml_value(stream.children.first.root, source)
  workflow = d171_mapping(workflow, source)
  jobs = d171_mapping(workflow["jobs"], "#{source} jobs")

  d171_require_equal(
    workflow.keys.sort,
    D171_TOP_LEVEL_KEYS.sort,
    "#{source} top-level keys"
  )
  d171_require_equal(workflow["name"], "CI", "#{source} workflow name")
  d171_require_equal(workflow["on"], D171_TRIGGERS, "#{source} triggers")
  d171_require_equal(workflow["env"], D171_ENV, "#{source} workflow environment")
  d171_require_equal(
    workflow["permissions"],
    { "contents" => "read" },
    "#{source} workflow permissions"
  )
  d171_require_equal(
    workflow["concurrency"],
    D171_CONCURRENCY,
    "#{source} concurrency"
  )
  D171_JOB_NAMES.each do |job_name|
    d171_mapping(jobs[job_name], "#{source} #{job_name}")
  end
  d171_require_equal(jobs.keys.sort, D171_JOB_NAMES.sort, "#{source} job set")

  checkout_counts = Hash.new(0)
  jobs.each do |job_name, job|
    next unless job.is_a?(Hash)

    Array(job["steps"]).each do |step|
      next unless step.is_a?(Hash)
      next unless /\Aactions\/checkout@/i.match?(step.fetch("uses", ""))

      d171_require_equal(
        step["uses"],
        PINNED_CHECKOUT_ACTION,
        "#{source} #{job_name} checkout pin"
      )
      with = d171_mapping(step["with"], "#{source} #{job_name} checkout inputs")
      d171_require_equal(
        with["persist-credentials"],
        "false",
        "#{source} #{job_name} checkout credentials"
      )
      checkout_counts[job_name] += 1
    end
  end
  d171_require_equal(
    checkout_counts,
    D171_CHECKOUT_COUNTS,
    "#{source} checkout locations"
  )

  classifier = d171_mapping(jobs["classify-changes"], "#{source} classify-changes")
  d171_require_equal(
    classifier.keys.sort,
    %w[outputs permissions runs-on steps].sort,
    "#{source} classify-changes keys"
  )
  d171_require_equal(
    classifier["runs-on"],
    "ubuntu-latest",
    "#{source} classify-changes runner"
  )
  d171_require_equal(
    classifier["permissions"],
    { "contents" => "read" },
    "#{source} classify-changes permissions"
  )
  d171_require_equal(
    classifier["outputs"],
    D171_CLASSIFIER_OUTPUTS,
    "#{source} classify-changes outputs"
  )
  classifier_steps = d171_sequence(
    classifier["steps"],
    "#{source} classify-changes steps"
  )
  d171_require_equal(
    classifier_steps,
    [
      {
        "name" => "Check out exact event head with full history",
        "uses" => PINNED_CHECKOUT_ACTION,
        "with" => {
          "persist-credentials" => "false",
          "fetch-depth" => "0",
          "ref" =>
            "${{ github.event_name == 'pull_request' && github.event.pull_request.head.sha || github.sha }}"
        }
      },
      {
        "name" => "Classify exact changed paths",
        "id" => "classify",
        "env" => D171_CLASSIFIER_ENV,
        "run" => D171_CLASSIFIER_RUN
      }
    ],
    "#{source} classify-changes job"
  )

  governance = d171_mapping(jobs["governance"], "#{source} governance")
  d171_require_equal(
    governance["needs"],
    "classify-changes",
    "#{source} governance dependency"
  )
  d171_require_equal(
    governance["if"],
    "${{ !cancelled() }}",
    "#{source} governance cancellation condition"
  )
  d171_require_equal(
    governance["runs-on"],
    "ubuntu-latest",
    "#{source} governance runner"
  )
  d171_require_equal(
    governance["permissions"],
    { "contents" => "read" },
    "#{source} governance permissions"
  )
  governance_steps = d171_sequence(governance["steps"], "#{source} governance steps")
  agent_steps = governance_steps.select do |step|
    [
      "Install LLVM 22 for offline alpha skill contract evals",
      "cargo build --workspace for offline alpha skill contract evals",
      "Run offline alpha skill contract evals"
    ].include?(step["name"])
  end
  d171_require_equal(
    agent_steps.length,
    3,
    "#{source} governance agent steps"
  )
  agent_steps.each do |step|
    d171_require_equal(
      step["if"],
      "needs.classify-changes.outputs.agent == 'true'",
      "#{source} governance agent condition"
    )
  end
  d171_require_equal(
    d171_canonical_sha256(governance),
    D171_JOB_BODY_SHA256S.fetch("governance"),
    "#{source} governance body"
  )

  D171_OPTIONAL_ROUTING.each do |job_name, (output, expected_needs)|
    job = d171_mapping(jobs[job_name], "#{source} #{job_name}")
    d171_require_equal(
      job["needs"],
      expected_needs,
      "#{source} #{job_name} dependency"
    )
    d171_require_equal(
      job["if"],
      "needs.classify-changes.outputs.#{output} == 'true'",
      "#{source} #{job_name} condition"
    )
  end

  d171_require_equal(
    jobs.dig("native-build-test", "strategy"),
    D171_TIER1_STRATEGY,
    "#{source} Tier-1 strategy"
  )

  ci_gate = d171_mapping(jobs["ci-gate"], "#{source} ci-gate")
  d171_require_equal(
    ci_gate.keys.sort,
    %w[if needs permissions runs-on steps].sort,
    "#{source} ci-gate keys"
  )
  d171_require_equal(ci_gate["needs"], D171_CI_GATE_NEEDS, "#{source} ci-gate needs")
  d171_require_equal(ci_gate["if"], "always()", "#{source} ci-gate condition")
  d171_require_equal(ci_gate["runs-on"], "ubuntu-latest", "#{source} ci-gate runner")
  d171_require_equal(ci_gate["permissions"], {}, "#{source} ci-gate permissions")
  gate_steps = d171_sequence(ci_gate["steps"], "#{source} ci-gate steps")
  d171_require_equal(gate_steps.length, 1, "#{source} ci-gate step count")
  gate_step = d171_mapping(gate_steps.first, "#{source} ci-gate fail step")
  d171_require_equal(
    gate_step.keys.sort,
    %w[if name run].sort,
    "#{source} ci-gate fail-step keys"
  )
  d171_require_equal(
    gate_step["name"],
    "Fail unless required jobs and exact routing succeeded",
    "#{source} ci-gate fail-step name"
  )
  d171_require_equal(
    d171_normalize_expression(gate_step["if"]),
    d171_normalize_expression(D171_CI_GATE_FAILURE_CONDITION),
    "#{source} ci-gate truth table"
  )
  d171_require_equal(gate_step["run"], D171_CI_GATE_RUN, "#{source} ci-gate failure")

  # Strip only the D-171 scheduling envelope, then delegate the unchanged
  # coverage, performance provenance, aggregate predecessor, and Pages
  # invariants to their established validators.
  unrouted = Marshal.load(Marshal.dump(workflow))
  unrouted.delete("concurrency")
  unrouted_jobs = unrouted.fetch("jobs")
  unrouted_jobs.delete("classify-changes")
  unrouted_jobs.delete("governance")
  D171_OPTIONAL_ROUTING.each_key do |job_name|
    job = unrouted_jobs.fetch(job_name)
    job.delete("if")
    needs = job["needs"]
    if needs.is_a?(Array)
      remaining = needs.reject { |need| need == "classify-changes" }
      if remaining.empty?
        job.delete("needs")
      else
        job["needs"] = remaining.length == 1 ? remaining.first : remaining
      end
    else
      job.delete("needs")
    end
  end
  unrouted_jobs["ci-gate"] = Marshal.load(Marshal.dump(D199_PAGES_ACCESSIBILITY_CI_GATE_JOB))
  unrouted_text = unrouted.to_yaml

  unless coverage_gate_present?(unrouted_text, source)
    raise RoadmapEvidenceError, "#{source}: D-171 does not retain the exact coverage gate"
  end
  validate_source_aware_perf_gate_lifecycle(unrouted_text, source)
  validate_pages_performance_lifecycle(unrouted_text, source)
  D171_OPTIONAL_ROUTING.each_key do |job_name|
    d171_require_equal(
      d171_canonical_sha256(unrouted_jobs.fetch(job_name)),
      D171_JOB_BODY_SHA256S.fetch(job_name),
      "#{source} #{job_name} body"
    )
  end
  true
end

def coverage_gate_present?(workflow_text, source)
  stream = Psych.parse_stream(workflow_text, filename: source)
  if stream.children.length != 1 || stream.children.first.root.nil?
    raise RoadmapEvidenceError, "#{source} must contain exactly one YAML document"
  end

  root = yaml_mapping(stream.children.first.root, source)
  if root.key?("defaults")
    raise RoadmapEvidenceError,
          "#{source}: coverage evidence must not inherit run defaults"
  end
  workflow_env = root["env"] ? yaml_value(root["env"], "#{source} environment") : {}
  unless workflow_env == TRUSTED_COVERAGE_ENV
    raise RoadmapEvidenceError,
          "#{source}: coverage workflow environment does not match the trusted values"
  end

  triggers = yaml_mapping(root["on"], "#{source} triggers")
  pull_request = triggers["pull_request"]
  unfiltered_pull_request =
    (pull_request.is_a?(Psych::Nodes::Scalar) && pull_request.value.empty?) ||
    (pull_request.is_a?(Psych::Nodes::Mapping) && pull_request.children.empty?)
  unless unfiltered_pull_request
    raise RoadmapEvidenceError,
          "#{source}: coverage evidence must run on every pull request"
  end

  jobs = yaml_mapping(root["jobs"], "#{source} jobs")
  job = yaml_mapping(jobs[COVERAGE_JOB], "#{source} job #{COVERAGE_JOB.inspect}")
  if job.key?("env")
    raise RoadmapEvidenceError,
          "#{source}: coverage job environment does not match the trusted values"
  end
  if job.key?("defaults")
    raise RoadmapEvidenceError,
          "#{source}: coverage evidence must not inherit run defaults"
  end
  if job.key?("needs")
    raise RoadmapEvidenceError,
          "#{source}: coverage evidence must not depend on other jobs"
  end
  if job.key?("if")
    raise RoadmapEvidenceError, "#{source}: coverage evidence must run unconditionally"
  end
  allowed_job_keys = %w[runs-on steps continue-on-error]
  unless (job.keys - allowed_job_keys).empty?
    raise RoadmapEvidenceError,
          "#{source}: coverage job contains untrusted configuration"
  end
  unless yaml_scalar(job["runs-on"], "#{source} coverage runner") == "macos-14"
    raise RoadmapEvidenceError,
          "#{source}: coverage job must use the trusted runner"
  end
  job_continue_on_error = job["continue-on-error"]
  unsafe_job_continue_on_error =
    job_continue_on_error &&
    yaml_scalar(
      job_continue_on_error,
      "#{source} job #{COVERAGE_JOB.inspect} continue-on-error"
    ).strip != "false"
  if unsafe_job_continue_on_error
    raise RoadmapEvidenceError, "#{source}: coverage job must propagate failures"
  end

  steps = job["steps"]
  unless steps.is_a?(Psych::Nodes::Sequence)
    raise RoadmapEvidenceError, "#{source} job #{COVERAGE_JOB.inspect} steps must be a sequence"
  end

  coverage_index = nil
  steps.children.each_with_index do |step_node, index|
    step = yaml_mapping(step_node, "#{source} step")
    next unless step["name"] && step["run"]

    next unless yaml_scalar(step["name"], "#{source} step name") == COVERAGE_STEP
    next unless REVIEWED_COVERAGE_SCRIPTS.include?(
      yaml_scalar(step["run"], "#{source} step run").strip
    )

    if step.key?("shell")
      raise RoadmapEvidenceError, "#{source}: coverage step must use the default shell"
    end

    continue_on_error = step["continue-on-error"]
    unsafe_continue_on_error =
      continue_on_error &&
      yaml_scalar(continue_on_error, "#{source} step continue-on-error").strip != "false"
    if step.key?("if") || unsafe_continue_on_error
      raise RoadmapEvidenceError, "#{source}: coverage evidence must run unconditionally"
    end

    coverage_index = index
    break
  end
  return false unless coverage_index

  actual_prefix = steps.children.first(coverage_index + 1).map do |step_node|
    step = yaml_value(step_node, "#{source} coverage setup step")
    step.delete("continue-on-error") if step["continue-on-error"] == "false"
    step
  end
  unless REVIEWED_TRUSTED_COVERAGE_STEPS.include?(actual_prefix)
    raise RoadmapEvidenceError,
          "#{source}: coverage setup steps do not match the trusted sequence"
  end

  true
end

def validate_perf_gate_baseline_lifecycle(workflow_text, source)
  stream = Psych.parse_stream(workflow_text, filename: source)
  root = yaml_mapping(stream.children.first.root, source)
  jobs = yaml_mapping(root["jobs"], "#{source} jobs")
  measure_job_node = jobs["frontend-perf-measure"]
  perf_job_node = jobs["frontend-perf-gate"]

  unless measure_job_node
    raise RoadmapEvidenceError,
          "#{source}: active paired gate requires frontend-perf-measure"
  end
  measure_job =
    yaml_value(measure_job_node, "#{source} frontend-perf-measure job")
  unless measure_job == PAIRED_PERF_MEASURE_JOB
    raise RoadmapEvidenceError,
          "#{source}: frontend-perf-measure must match the reviewed paired measurement job"
  end

  unless perf_job_node
    raise RoadmapEvidenceError,
          "#{source}: active paired gate requires frontend-perf-gate"
  end
  perf_job = yaml_value(perf_job_node, "#{source} frontend-perf-gate job")
  unless perf_job == PAIRED_PERF_GATE_JOB
    raise RoadmapEvidenceError,
          "#{source}: frontend-perf-gate must match the reviewed paired comparison job"
  end

  ci_gate_node = jobs["ci-gate"]
  unless ci_gate_node
    raise RoadmapEvidenceError,
          "#{source}: paired performance jobs must be required by ci-gate"
  end
  ci_gate = yaml_value(ci_gate_node, "#{source} ci-gate job")
  unless ci_gate == PAIRED_PERF_CI_GATE_JOB
    raise RoadmapEvidenceError,
          "#{source}: ci-gate must match the reviewed fail-closed aggregate job"
  end

  true
end

def validate_source_aware_perf_gate_lifecycle(workflow_text, source)
  stream = Psych.parse_stream(workflow_text, filename: source)
  root = yaml_mapping(stream.children.first.root, source)
  jobs = yaml_mapping(root["jobs"], "#{source} jobs")
  measure_job_node = jobs["frontend-perf-measure"]
  perf_job_node = jobs["frontend-perf-gate"]

  unless measure_job_node
    raise RoadmapEvidenceError,
          "#{source}: source-aware gate requires frontend-perf-measure"
  end
  measure_job =
    yaml_value(measure_job_node, "#{source} frontend-perf-measure job")
  expected_perf_job =
    if measure_job == D56_SOURCE_AWARE_PERF_MEASURE_JOB
      D56_SOURCE_AWARE_PERF_GATE_JOB
    elsif measure_job == REPLICATED_PERF_MEASURE_JOB
      REPLICATED_PERF_GATE_JOB
    elsif measure_job == D91_RELAX_FRONTEND_PERF_MANIFEST_MEASURE_JOB
      REPLICATED_PERF_GATE_JOB
    elsif measure_job == D112_UBUNTU_FRONTEND_PERF_MEASURE_JOB
      # D-114: this measure job now accepts either gate-job shape (D-112's
      # own 2.0%-implicit-threshold shape, or D-114's 7.0%-explicit-threshold
      # shape) -- an array here, unlike every other branch's single job
      # constant, deliberately so a single measure-job shape can authorize
      # more than one gate-job shape without a parallel measure-job branch.
      [D112_UBUNTU_FRONTEND_PERF_GATE_JOB, D114_RAISED_THRESHOLD_FRONTEND_PERF_GATE_JOB]
    end
  unless expected_perf_job
    raise RoadmapEvidenceError,
          "#{source}: frontend-perf-measure must match the reviewed source-aware measurement job " \
          "or its fixed-replicate successor"
  end

  unless perf_job_node
    raise RoadmapEvidenceError,
          "#{source}: source-aware gate requires frontend-perf-gate"
  end
  perf_job = yaml_value(perf_job_node, "#{source} frontend-perf-gate job")
  accepted_perf_jobs =
    expected_perf_job.is_a?(Array) ? expected_perf_job : [expected_perf_job]
  unless accepted_perf_jobs.include?(perf_job)
    raise RoadmapEvidenceError,
          "#{source}: frontend-perf-gate must match the reviewed source-aware comparison job " \
          "for the selected measurement design"
  end

  ci_gate_node = jobs["ci-gate"]
  unless ci_gate_node
    raise RoadmapEvidenceError,
          "#{source}: source-aware performance jobs must be required by ci-gate"
  end
  ci_gate = yaml_value(ci_gate_node, "#{source} ci-gate job")
  unless ACCEPTED_PERF_CI_GATE_JOBS.include?(ci_gate)
    raise RoadmapEvidenceError,
          "#{source}: ci-gate must match the reviewed fail-closed aggregate job"
  end

  true
end

# Issue #229: validate the pages-performance job's lifecycle in the live
# ci.yml.  This checks that the job exists, is required by ci-gate, is
# checked in ci-gate's fail step, has no continue-on-error, is not
# push-only, and has unprivileged (contents: read) permissions.  Unlike
# the paired-perf lifecycle validators above, this one does NOT compare
# the job against a frozen constant -- the pages-performance job's own
# steps (Lighthouse version, thresholds, artifact uploads) are reviewed
# separately by check_pages_performance_budget.rb's own test suite, so
# this validator only enforces the structural invariants that keep it
# fail-closed and unprivileged.
def validate_pages_performance_lifecycle(workflow_text, source)
  # Issue #229: the pages-performance job is added to ci.yml in Phase 3
  # (PR 5).  Between PR 3 (checker activation, which copies this staged
  # content to the live checker) and PR 5 (ci.yml activation, which adds
  # the pages-performance job), the live ci.yml still has its pre-D229
  # shape -- one of the other accepted digests in
  # REVIEWED_PERF_CI_WORKFLOW_SHA256S.  Enforcing the pages-performance
  # lifecycle validation against that pre-D229 shape would fail (the job
  # does not exist yet), so gate the full validation on the D229 digest:
  # enforce only when the live ci.yml matches
  # D229_PAGES_PERFORMANCE_CI_WORKFLOW_SHA256.  For any other accepted
  # digest (the pre-D229 transition shape), skip the pages-performance
  # lifecycle validation -- the workflow is in its pre-D229 shape, which
  # is still accepted.  For an unknown digest (not in the accepted array),
  # enforce fail-closed; validate_evidence's own digest check already
  # rejects unknown digests before this function runs in production, so
  # this default only matters for direct unit tests with synthetic
  # workflows that do not match any reviewed digest.
  digest = Digest::SHA256.hexdigest(workflow_text)
  if digest != D229_PAGES_PERFORMANCE_CI_WORKFLOW_SHA256 &&
     REVIEWED_PERF_CI_WORKFLOW_SHA256S.include?(digest)
    # Pre-D229 transition shape: the live ci.yml is still one of the
    # accepted pre-pages-performance digests.  Skip the pages-performance
    # lifecycle validation until PR 5 activates the D229 ci.yml shape.
    return true
  end

  stream = Psych.parse_stream(workflow_text, filename: source)
  root = yaml_mapping(stream.children.first.root, source)
  jobs = yaml_mapping(root["jobs"], "#{source} jobs")

  pages_job_node = jobs["pages-performance"]
  unless pages_job_node
    raise RoadmapEvidenceError,
          "#{source}: pages-performance job is required by issue #229"
  end
  pages_job = yaml_mapping(pages_job_node, "#{source} pages-performance job")

  # The job must not be push-only (no if: guard restricting to push).
  if pages_job.key?("if")
    if_value = yaml_scalar(pages_job["if"], "#{source} pages-performance if").strip
    if if_value.include?("github.event_name == 'push'") ||
       if_value.include?("github.ref == 'refs/heads/main'")
      raise RoadmapEvidenceError,
            "#{source}: pages-performance must not be push-only"
    end
  end

  # The job must not have continue-on-error (must propagate failures).
  continue_on_error = pages_job["continue-on-error"]
  if continue_on_error
    coe_value = yaml_scalar(continue_on_error, "#{source} pages-performance continue-on-error").strip
    if coe_value != "false"
      raise RoadmapEvidenceError,
            "#{source}: pages-performance must propagate failures"
    end
  end

  # The job must have unprivileged permissions (contents: read only).
  permissions_node = pages_job["permissions"]
  unless permissions_node
    raise RoadmapEvidenceError,
          "#{source}: pages-performance must declare explicit permissions"
  end
  permissions = yaml_mapping(permissions_node, "#{source} pages-performance permissions")
  contents_perm = permissions["contents"]
  unless contents_perm
    raise RoadmapEvidenceError,
          "#{source}: pages-performance must declare contents permission"
  end
  unless yaml_scalar(contents_perm, "#{source} pages-performance contents permission") == "read"
    raise RoadmapEvidenceError,
          "#{source}: pages-performance must have contents: read permission"
  end
  unless permissions.keys.length == 1
    raise RoadmapEvidenceError,
          "#{source}: pages-performance must have only contents: read permission"
  end

  # ci-gate must require pages-performance.
  ci_gate_node = jobs["ci-gate"]
  unless ci_gate_node
    raise RoadmapEvidenceError,
          "#{source}: pages-performance must be required by ci-gate"
  end
  ci_gate = yaml_mapping(ci_gate_node, "#{source} ci-gate job")
  needs_node = ci_gate["needs"]
  unless needs_node && needs_node.is_a?(Psych::Nodes::Sequence)
    raise RoadmapEvidenceError,
          "#{source}: ci-gate must have a needs sequence"
  end
  needs = needs_node.children.map do |need|
    yaml_scalar(need, "#{source} ci-gate needs entry")
  end
  unless needs.include?("pages-performance")
    raise RoadmapEvidenceError,
          "#{source}: ci-gate must require pages-performance"
  end

  # ci-gate's fail step must check needs.pages-performance.result != 'success'.
  steps = ci_gate["steps"]
  unless steps && steps.is_a?(Psych::Nodes::Sequence)
    raise RoadmapEvidenceError,
          "#{source}: ci-gate must have a steps sequence"
  end
  fail_step = steps.children.find do |step_node|
    step = yaml_mapping(step_node, "#{source} ci-gate step")
    step["name"] && yaml_scalar(step["name"], "#{source} ci-gate step name") =~ /Fail unless/
  end
  unless fail_step
    raise RoadmapEvidenceError,
          "#{source}: ci-gate must have a fail step"
  end
  fail_step_mapping = yaml_mapping(fail_step, "#{source} ci-gate fail step")
  if_cond = fail_step_mapping["if"]
  unless if_cond
    raise RoadmapEvidenceError,
          "#{source}: ci-gate fail step must have an if condition"
  end
  if_text = yaml_scalar(if_cond, "#{source} ci-gate fail step if").strip
  unless if_text.include?("needs.pages-performance.result != 'success'")
    raise RoadmapEvidenceError,
          "#{source}: ci-gate fail step must check needs.pages-performance.result"
  end

  # Issue #199: when the D199 eight-element shape is active (ci-gate
  # needs includes pages-accessibility), the fail step must also check
  # needs.pages-accessibility.result != 'success'.
  if needs.include?("pages-accessibility")
    unless if_text.include?("needs.pages-accessibility.result != 'success'")
      raise RoadmapEvidenceError,
            "#{source}: ci-gate fail step must check needs.pages-accessibility.result"
    end
  end

  true
end

def opening_fence(line)
  match = /\A {0,3}(?<marker>`{3,}|~{3,})(?<info>[^\r\n]*)(?:\r?\n)?\z/.match(line)
  return unless match

  marker = match[:marker]
  return if marker.start_with?("`") && match[:info].include?("`")

  [marker[0], marker.length]
end

def closing_fence?(line, fence)
  character, minimum_length = fence
  pattern =
    /\A {0,3}#{Regexp.escape(character)}{#{minimum_length},}[ \t]*(?:\r?\n)?\z/
  pattern.match?(line)
end

def without_html_comments(line, in_comment)
  visible = String.new
  cursor = 0
  loop do
    if in_comment
      closing = line.index("-->", cursor)
      return [visible, true] unless closing

      cursor = closing + 3
      in_comment = false
    else
      opening = line.index("<!--", cursor)
      unless opening
        visible << line[cursor..]
        return [visible, false]
      end

      visible << line[cursor...opening]
      cursor = opening + 4
      in_comment = true
    end
  end
end

def blockquote_content(line)
  content = line
  depth = 0
  loop do
    prefix = /\A {0,3}>[ \t]?/.match(content)
    break unless prefix

    content = content[prefix.end(0)..] || ""
    depth += 1
  end
  [content, depth]
end

def indentation_width(whitespace, initial_column = 0)
  whitespace.each_char.reduce(initial_column) do |column, character|
    character == "\t" ? column + (4 - (column % 4)) : column + 1
  end
end

def strip_indentation(line, columns)
  column = 0
  index = 0
  while column < columns
    character = line[index]
    return line unless character == " " || character == "\t"

    if character == "\t"
      next_column = column + (4 - (column % 4))
      if next_column > columns
        remainder = next_column - columns
        return (" " * remainder) + (line[(index + 1)..] || "")
      end
      column = next_column
    else
      column += 1
    end
    index += 1
  end
  line[index..] || ""
end

def list_container_content(line, containers)
  indent = indentation_width(line[/\A[ \t]*/])
  content_indent = containers.reverse.find { |candidate| indent >= candidate } || 0
  [strip_indentation(line, content_indent), content_indent]
end

def rendered_list_item(line, containers)
  item = LIST_ITEM.match(line)
  return unless item

  indent = indentation_width(item[:indent])
  parent_index = containers.rindex { |content_indent| indent >= content_indent }
  return if indent > 3 && parent_index.nil?

  containers.replace(parent_index ? containers.first(parent_index + 1) : [])
  marker_end = indent + item[:marker].length
  spacing = item[:spacing] || " "
  content_indent = indentation_width(spacing, marker_end)
  padding = content_indent - marker_end
  containers << (padding <= 4 ? content_indent : marker_end + 1)

  body = padding <= 4 ? (item[:body] || "") : nil
  {
    body: body,
    checked_item: body && CHECKED_ITEM_BODY.match(body),
    content_indent: containers.last
  }
end

def validate_roadmap(text)
  evidence_ids = []
  heading_path = []
  fence = nil
  in_html_comment = false
  list_containers = Hash.new { |containers, depth| containers[depth] = [] }
  text.each_line.with_index(1) do |line, line_number|
    if fence
      candidate, quote_depth = blockquote_content(line)
      candidate =
        if quote_depth == fence[2]
          strip_indentation(candidate, fence[3])
        else
          line
        end
      fence = nil if closing_fence?(candidate, fence)
      next
    end

    visible_line, in_html_comment = without_html_comments(line, in_html_comment)
    fence_candidate, quote_depth = blockquote_content(visible_line)
    containers = list_containers[quote_depth]
    normalized_container_content, content_indent =
      list_container_content(fence_candidate, containers)
    list_item = rendered_list_item(fence_candidate, containers)
    block_content = list_item ? list_item[:body] : normalized_container_content
    content_indent = list_item[:content_indent] if list_item

    if block_content && RAW_HTML_BLOCK_START.match?(block_content)
      raise RoadmapEvidenceError,
            "line #{line_number}: raw HTML blocks are not supported in the roadmap"
    end

    opening = block_content && opening_fence(block_content)
    if opening
      fence = [*opening, quote_depth, content_indent]
      next
    end

    if block_content && SETEXT_UNDERLINE.match?(block_content)
      raise RoadmapEvidenceError,
            "line #{line_number}: Setext headings are not supported; use ATX headings"
    end

    heading = block_content && ATX_HEADING.match(block_content)
    if heading
      list_containers.clear if quote_depth.zero? && content_indent.zero?
      level = heading[:marks].length
      title = heading[:title].sub(/[ \t]+#+[ \t]*$/, "").strip
      heading_path = heading_path.first(level - 1)
      heading_path[level - 1] = title
      next
    end

    item = list_item && list_item[:checked_item]
    item ||= CHECKED_ITEM_BODY.match(block_content) if !list_item && content_indent.positive?
    unless list_item || LIST_ITEM.match?(fence_candidate)
      if fence_candidate.strip.empty?
        next
      end

      indent = indentation_width(fence_candidate[/\A[ \t]*/])
      containers.pop while containers.any? && indent < containers.last
    end
    next unless item

    marker_ids = []
    line.scan(EVIDENCE_MARKER) { marker_ids << Regexp.last_match[:id] }
    if marker_ids.empty?
      raise RoadmapEvidenceError,
            "line #{line_number}: checked roadmap item is missing an evidence marker"
    end
    unless marker_ids.length == 1
      raise RoadmapEvidenceError,
            "line #{line_number}: checked roadmap item must contain exactly one evidence marker"
    end

    evidence_id = marker_ids.first
    expected_claim = EVIDENCE_CLAIMS[evidence_id]
    unless expected_claim
      raise RoadmapEvidenceError,
            "line #{line_number}: unknown roadmap evidence #{evidence_id.inspect}"
    end

    expected_section = EVIDENCE_SECTIONS.fetch(evidence_id)
    unless heading_path == expected_section
      raise RoadmapEvidenceError,
            "line #{line_number}: evidence #{evidence_id.inspect} must appear under " \
            "the expected roadmap section #{expected_section.join(' > ').inspect}"
    end

    actual_claim = item[:claim].strip
    if actual_claim == expected_claim
      evidence_ids << evidence_id
      next
    end

    raise RoadmapEvidenceError,
          "line #{line_number}: evidence #{evidence_id.inspect} does not prove this roadmap claim"
  end
  evidence_ids
end

def validate_evidence(root, _evidence_ids)
  workflow = root / ".github/workflows/ci.yml"
  workflow_text = workflow.read
  digest = Digest::SHA256.hexdigest(workflow_text)
  if digest == D171_CHANGE_AWARE_CI_WORKFLOW_SHA256
    unless REVIEWED_PERF_CI_WORKFLOW_SHA256S.include?(digest)
      raise RoadmapEvidenceError,
            "#{workflow}: D-171 workflow digest is not in the reviewed coexistence set"
    end
    validate_d171_ci_routing(workflow_text, workflow.to_s)
    return
  end
  unless coverage_gate_present?(workflow_text, workflow.to_s)
    raise RoadmapEvidenceError,
          "#{workflow}: evidence does not provide the exact 100% line and region gate"
  end
  unless REVIEWED_PERF_CI_WORKFLOW_SHA256S.include?(digest)
    raise RoadmapEvidenceError,
          "#{workflow}: does not match a reviewed active-or-staged performance CI workflow"
  end
  validate_source_aware_perf_gate_lifecycle(workflow_text, workflow.to_s)
  validate_pages_performance_lifecycle(workflow_text, workflow.to_s)
end

def main(arguments)
  raise RoadmapEvidenceError, "usage: check_roadmap_evidence.rb [repository-root]" if arguments.length > 1

  root = Pathname(arguments.first || ".")
  roadmap = root / "docs/ROADMAP.md"
  evidence_ids = validate_roadmap(roadmap.read)
  validate_evidence(root, evidence_ids)
  puts "Roadmap evidence policy passed."
  0
rescue Errno::ENOENT, Psych::SyntaxError, RoadmapEvidenceError => e
  warn e.message
  1
end

exit(main(ARGV)) if $PROGRAM_NAME == __FILE__
